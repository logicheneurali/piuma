//! Freedesktop autostart (roadmap N2).
//!
//! When `panel.autostart` is enabled the bar launches, once at startup,
//! the applications marked as autostart under the standard XDG
//! directories, skipping the ones already running. The parser honours
//! the Autostart entry spec: `Hidden`, `X-GNOME-Autostart-enabled`,
//! `OnlyShowIn`/`NotShowIn`, `TryExec`, `Path`, `Exec` field codes, and
//! a user file overriding the system file with the same name.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A parsed `.desktop` autostart entry (only the spec keys piuma honours).
#[derive(Debug, Default, Clone)]
struct Entry {
    /// File name (`foo.desktop`) the entry was read from: the config's
    /// selection (`panel.autostart_disabled`) keys on it.
    file: String,
    /// `Exec=` command line, field codes already stripped.
    exec: String,
    /// `Path=`: working directory of `Exec` (`None` = inherited).
    path: Option<String>,
    /// `Hidden=true` marks a removed entry (also used by the user to
    /// suppress a system entry with the same file name).
    hidden: bool,
    /// `X-GNOME-Autostart-enabled`; `None` = unset (treated as enabled).
    gnome_enabled: Option<bool>,
    /// `OnlyShowIn=`: visible only inside one of these desktops.
    only_show_in: Option<Vec<String>>,
    /// `NotShowIn=`: hidden inside one of these desktops.
    not_show_in: Option<Vec<String>>,
    /// `TryExec=`: the program must exist for the entry to run.
    try_exec: Option<String>,
    /// `Name=`: human-readable label (the setup editor's list, roadmap2
    /// #2); `None` when the file carries none.
    label: Option<String>,
}

/// Launch every eligible autostart entry on a background thread so the
/// bar's startup never waits for the spawned applications; `disabled`
/// is the config's own per-app selection (file names piuma must not
/// start — `panel.autostart_disabled`).
pub fn spawn(disabled: Vec<String>) {
    let _ = std::thread::Builder::new()
        .name("piuma-autostart".to_string())
        .spawn(move || run(disabled));
}

/// Autostart thread: merge both directories, apply the spec filters and
/// the config's selection, launch what survives — never a program
/// already running, never piuma. The caller pruned `disabled` against
/// the live files first (see [`prune_disabled`]).
fn run(disabled: Vec<String>) {
    let desktop = current_desktop();
    for entry in collect() {
        if !should_run(&entry, &desktop) {
            continue;
        }
        // The user's own selection wins last (piuma-only: other desktop
        // environments never see this list).
        if disabled.contains(&entry.file) {
            eprintln!(
                "[piuma] autostart: {} disabled in config, skipping",
                entry.file
            );
            continue;
        }
        let program = program_of(&entry.exec);
        if program.is_empty() || is_self(&program) {
            continue;
        }
        // "se non già in esecuzione": a running instance wins.
        if already_running(&program) {
            eprintln!("[piuma] autostart: {program} already running, skipping");
            continue;
        }
        launch(&entry, &program);
    }
}

/// The standard autostart directories, lowest priority first: the system
/// `$XDG_CONFIG_DIRS` entries (default `/etc/xdg`) and then the user's
/// `$XDG_CONFIG_HOME/autostart` (or `~/.config/autostart`), which
/// overrides a system file carrying the same name (Autostart spec).
fn autostart_dirs() -> Vec<PathBuf> {
    let config_dirs = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/etc/xdg".to_string());
    // The base-dirs spec orders `$XDG_CONFIG_DIRS` by descending
    // preference → reverse so `merge` (later wins) keeps that precedence.
    let mut dirs: Vec<PathBuf> = config_dirs
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(s).join("autostart"))
        .collect();
    dirs.reverse();
    if let Some(user) = user_config_dir() {
        dirs.push(user.join("autostart"));
    }
    dirs
}

/// The merged file names of the live directories: the config's disabled
/// list is validated against exactly what the launcher can see.
pub fn existing_files() -> BTreeSet<String> {
    winning_files(&autostart_dirs()).into_keys().collect()
}

/// The config's disabled names reduced to the ones whose `.desktop`
/// file still exists in `existing` (sorted, deduplicated): a removed
/// app's entry must not outlive its file. Pure — the bar prunes before
/// launching, piuma-setup prunes on every save.
pub fn prune_disabled(existing: &BTreeSet<String>, disabled: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = disabled
        .iter()
        .filter(|name| existing.contains(*name))
        .cloned()
        .collect();
    kept.sort();
    kept.dedup();
    kept
}

/// `$XDG_CONFIG_HOME`, falling back to `$HOME/.config` (Autostart spec).
fn user_config_dir() -> Option<PathBuf> {
    config_home_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// The config home out of the two spec variables: an absolute
/// `$XDG_CONFIG_HOME` wins, a relative one falls through to `$HOME`
/// (base-dirs spec), and the `$HOME` fallback is `$HOME/.config` —
/// regression: returning bare `$HOME` made the scan look at
/// `~/autostart` and miss `~/.config/autostart` entirely.
fn config_home_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = xdg {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    let home = PathBuf::from(home?);
    home.is_absolute().then(|| home.join(".config"))
}

/// Merge the given directories by file name (later entries win) and
/// parse every `.desktop` file found; unreadable files are skipped.
fn merge(dirs: &[PathBuf]) -> Vec<Entry> {
    winning_files(dirs)
        .into_iter()
        .filter_map(|(file, path)| {
            let mut entry = parse(&std::fs::read_to_string(path).ok()?)?;
            entry.file = file;
            Some(entry)
        })
        .collect()
}

/// The winning `.desktop` file per file name: the dirs are scanned in
/// order and a later dir's file replaces an earlier one (the user's dir
/// comes last — Autostart spec).
fn winning_files(dirs: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut by_name: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else { continue };
        for file in read.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if name.ends_with(".desktop") {
                by_name.insert(name, file.path());
            }
        }
    }
    by_name
}

/// Entries of the live directories (see [`autostart_dirs`]).
fn collect() -> Vec<Entry> {
    merge(&autostart_dirs())
}

/// Parse the `[Desktop Entry]` group of a `.desktop` file; a file
/// without `Exec` is not an autostart candidate.
fn parse(text: &str) -> Option<Entry> {
    let entry = parse_entry(text);
    (!entry.exec.is_empty()).then_some(entry)
}

/// The `[Desktop Entry]` group with no `Exec` gate: the setup editor
/// also lists entries that carry only their switch — a pure
/// `Hidden=true` override parses fine and shows as off (roadmap2 #2).
fn parse_entry(text: &str) -> Entry {
    let mut entry = Entry::default();
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Exec" => entry.exec = strip_field_codes(value.trim()),
            "Path" => entry.path = Some(value.trim().to_string()),
            "Hidden" => entry.hidden = value.trim() == "true",
            "X-GNOME-Autostart-enabled" => entry.gnome_enabled = Some(value.trim() == "true"),
            "OnlyShowIn" => entry.only_show_in = Some(split_list(value)),
            "NotShowIn" => entry.not_show_in = Some(split_list(value)),
            "TryExec" => entry.try_exec = Some(value.trim().to_string()),
            "Name" => entry.label = Some(value.trim().to_string()),
            _ => {}
        }
    }
    entry
}

/// The spec filters: hidden entries are removed, the GNOME switch and
/// the desktop-name lists decide visibility, `TryExec` must resolve.
fn should_run(entry: &Entry, current_desktop: &[String]) -> bool {
    if entry.hidden || entry.gnome_enabled == Some(false) {
        return false;
    }
    if let Some(only) = &entry.only_show_in {
        if !only.iter().any(|d| current_desktop.iter().any(|c| c == d)) {
            return false;
        }
    }
    if let Some(not) = &entry.not_show_in {
        if not.iter().any(|d| current_desktop.iter().any(|c| c == d)) {
            return false;
        }
    }
    match &entry.try_exec {
        Some(program) => exists(program),
        None => true,
    }
}

/// `OnlyShowIn`/`NotShowIn` are `;`-separated desktop names.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Desktop names of `$XDG_CURRENT_DESKTOP` (colon-separated).
fn current_desktop() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether a program resolves: an absolute/relative path must exist as
/// a file, a bare name somewhere on `$PATH`.
fn exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
        })
        .unwrap_or(false)
}

/// Freedesktop `Exec` field codes (`%f`, `%u`, `%%`, …) carry per-launch
/// arguments piuma never has; drop them (with `%%` kept as a literal `%`).
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(_) => {} // valid/unknown single-letter code: drop the pair
            None => out.push('%'),
        }
    }
    out.trim().to_string()
}

/// The program an `Exec` line runs: the first token of the cleaned line,
/// honouring double quotes — it is what the running-check compares.
/// A `sh -c "…"` wrapper is seen through (the spec forbids none, and
/// matching against the generic `sh` would match any shell alive in the
/// session — at boot there always is one, which is exactly how the
/// tray applet entry used to be skipped): the command inside `-c`
/// decides, and in a `&&`/`;`/`||` chain its LAST segment — the earlier
/// ones are setup (`sleep 3 && arch-update --tray`).
fn program_of(exec: &str) -> String {
    let exec = exec.trim_start();
    if let Some(inner) = shell_c_arg(exec) {
        let last = chain_last(&inner);
        let last = last.strip_prefix("exec ").unwrap_or(last);
        if !last.is_empty() && last != exec {
            return program_of(last);
        }
    }
    if let Some(rest) = exec.strip_prefix('"') {
        let end = rest.find('"').unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    exec.split_whitespace().next().unwrap_or("").to_string()
}

/// Shells whose `-c` argument carries the real command line.
const SHELLS: &[&str] = &["sh", "bash", "dash", "ksh", "zsh", "fish", "ash"];

/// The `-c` argument of a `<shell> -c "…"` wrapper (quotes stripped),
/// or `None` when `exec` is not that shape.
fn shell_c_arg(exec: &str) -> Option<String> {
    let exec = exec.trim_start();
    let shell = exec.split_whitespace().next()?;
    let base = Path::new(shell).file_name()?.to_string_lossy();
    if !SHELLS.contains(&base.as_ref()) {
        return None;
    }
    // The rest of the line after the shell token, then the `-c` flag.
    let arg = exec[shell.len()..].trim_start().strip_prefix("-c")?.trim_start();
    let arg = arg.strip_prefix('"').unwrap_or(arg);
    let arg = arg.strip_suffix('"').unwrap_or(arg);
    let arg = arg.strip_prefix('\'').unwrap_or(arg);
    let arg = arg.strip_suffix('\'').unwrap_or(arg);
    (!arg.is_empty()).then(|| arg.to_string())
}

/// The last `&&`/`;`/`||` segment of a `-c` command line: the part that
/// persists (the earlier ones are setup steps).
fn chain_last(cmd: &str) -> &str {
    cmd.rsplit(|c| c == '&' || c == ';' || c == '|')
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or(cmd)
}

/// True when `program` is piuma itself: a stray `piuma.desktop` in the
/// autostart directory must never spawn a second bar.
fn is_self(program: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe_name = exe.file_name().map(|n| n.to_string_lossy().into_owned());
    let prog_name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    matches!((&exe_name, &prog_name), (Some(a), Some(b)) if a == b)
}

/// "se non già in esecuzione": scan OUR processes for one whose first
/// argument has the same name as `program`. `/proc` sees the whole
/// system, so the uid filter matters: a root service (or anything else
/// of another user) is never "the app already running" for this
/// session's autostart.
fn already_running(program: &str) -> bool {
    let Some(want) = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return false;
    };
    let me = std::fs::metadata("/proc/self")
        .ok()
        .map(|m| m.uid());
    let Ok(entries) = std::fs::read_dir("/proc") else { return false };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(me) = me {
            let Ok(meta) = std::fs::metadata(entry.path()) else { continue };
            if meta.uid() != me {
                continue;
            }
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let Some(first) = cmdline.split(|b| *b == 0).next().filter(|s| !s.is_empty()) else {
            continue;
        };
        let first = String::from_utf8_lossy(first);
        if Path::new(first.as_ref())
            .file_name()
            .is_some_and(|n| n.to_string_lossy() == want.as_str())
        {
            return true;
        }
    }
    false
}

/// Spawn the entry's `Exec` through `sh -c` (background, fire-and-forget),
/// honouring `Path=` as the working directory.
fn launch(entry: &Entry, program: &str) {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", &entry.exec]);
    if let Some(dir) = &entry.path {
        cmd.current_dir(dir);
    }
    match cmd.spawn() {
        Ok(_) => eprintln!("[piuma] autostart: started {program}"),
        Err(err) => eprintln!("[piuma] autostart: failed to start {program}: {err}"),
    }
}

// ── setup editor (roadmap2 #2) ──────────────────────────────────────
/// The setup editor's view of the autostart dirs (roadmap2 #2): the bar
/// itself only ever runs [`spawn`]; this section is consumed by
/// piuma-setup, which includes this file through `#[path]` — hence the
/// allowance that keeps it out of the bar's warning baseline.
#[allow(dead_code)]
pub mod editor {
    use super::*;

    /// One autostart `.desktop` file as the editor lists it: which file
    /// wins the merge, how it calls itself and its effective switch.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct App {
        /// File name (`foo.desktop`), unique across the autostart dirs.
        pub file: String,
        /// `Name=` of the winning file, falling back to the file name.
        pub label: String,
        /// The effective switch: the spec state AND the config's own
        /// selection (absent from `panel.autostart_disabled`).
        pub enabled: bool,
        /// The spec state alone: off = the file's own switches (`Hidden`,
        /// GNOME) keep it out and piuma's selection has no say — the
        /// editor's switch is inert.
        pub spec: bool,
    }

    /// One [`App`] per file name over the live directories (see
    /// [`autostart_dirs`]), sorted by name — the winning file of each
    /// name decides label and spec state, `disabled` is the config's
    /// selection (`panel.autostart_disabled`).
    pub fn apps(disabled: &[String]) -> Vec<App> {
        apps_in(&autostart_dirs(), disabled)
    }

    /// [`apps`] over explicit directories (the tests pass their own); a
    /// file that cannot be read is left out, exactly like the launcher's
    /// own scan skips it.
    pub fn apps_in(dirs: &[PathBuf], disabled: &[String]) -> Vec<App> {
        winning_files(dirs)
            .into_iter()
            .filter_map(|(file, path)| {
                let entry = parse_entry(&std::fs::read_to_string(path).ok()?);
                let spec = !entry.hidden && entry.gnome_enabled != Some(false);
                Some(App {
                    label: entry.label.unwrap_or_else(|| file.clone()),
                    enabled: spec && !disabled.contains(&file),
                    spec,
                    file,
                })
            })
            .collect()
    }

    /// The editor's switch write rule — and the selection lives in the
    /// config (`panel.autostart_disabled`), never in the autostart
    /// directories: off adds the file name to the list, on removes it,
    /// an already-correct list is left untouched. Only plain `*.desktop`
    /// file names are accepted.
    pub fn toggle_disabled(
        disabled: &mut Vec<String>,
        file: &str,
        enabled: bool,
    ) -> std::io::Result<()> {
        if file.is_empty()
            || !file.ends_with(".desktop")
            || file.contains('/')
            || file.contains('\\')
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a plain .desktop file name",
            ));
        }
        if enabled {
            disabled.retain(|name| name != file);
        } else if !disabled.iter().any(|name| name == file) {
            disabled.push(file.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field codes vanish from `Exec` and quoting keeps `program_of` right.
    #[test]
    fn field_codes_and_quotes() {
        assert_eq!(program_of(&strip_field_codes("foo --open %f %U")), "foo");
        assert_eq!(program_of(&strip_field_codes(r#"/usr/bin/app "%u""#)), "/usr/bin/app");
        assert_eq!(program_of(r#""/usr/bin/my app" %f"#), "/usr/bin/my app");
        assert_eq!(strip_field_codes("app %%literal %i"), "app %literal");
        assert_eq!(strip_field_codes("app %"), "app %");
    }

    /// A `sh -c "…"` wrapper resolves to the real program — the chain's
    /// last command — never the generic shell: matching `sh` would match
    /// any shell alive in the session (at boot: always one), which is
    /// how the tray applet entry used to be skipped on a cold start.
    #[test]
    fn shell_wrappers_resolve_to_the_real_program() {
        assert_eq!(
            program_of(r#"/bin/sh -c "sleep 3 && arch-update --tray""#),
            "arch-update"
        );
        assert_eq!(program_of("sh -c foo"), "foo");
        assert_eq!(program_of(r#"bash -c "exec bar""#), "bar");
        assert_eq!(program_of(r#"sh -c "sh -c nested""#), "nested");
        // Not a wrapper: unchanged.
        assert_eq!(program_of("sh myscript.sh"), "sh");
        assert_eq!(program_of("notashell -c foo"), "notashell");
    }

    /// The spec filters decide visibility: hidden, the GNOME switch and
    /// the desktop-name lists — the `OnlyShowIn` case is what keeps KDE
    /// autostart entries out of a Hyprland session.
    #[test]
    fn spec_filters_apply() {
        let hyprland = vec!["Hyprland".to_string()];
        let base = Entry {
            exec: "true".to_string(),
            ..Entry::default()
        };
        assert!(should_run(&base, &hyprland));
        let hidden = Entry { hidden: true, ..base.clone() };
        assert!(!should_run(&hidden, &hyprland));
        let gnome_off = Entry { gnome_enabled: Some(false), ..base.clone() };
        assert!(!should_run(&gnome_off, &hyprland));
        let kde = Entry { only_show_in: Some(vec!["KDE".to_string()]), ..base.clone() };
        assert!(!should_run(&kde, &hyprland));
        let not_here = Entry { not_show_in: Some(vec!["Hyprland".to_string()]), ..base.clone() };
        assert!(!should_run(&not_here, &hyprland));
        let missing = Entry { try_exec: Some("/definitely/missing".to_string()), ..base };
        assert!(!should_run(&missing, &hyprland));
    }

    /// The user's file overrides the system one with the same name (so a
    /// user `Hidden=true` suppresses it) and only the `[Desktop Entry]`
    /// group is read — `[Desktop Action …]` Exec lines never leak in.
    #[test]
    fn user_file_overrides_system_and_groups_are_scoped() {
        let root = std::env::temp_dir().join(format!("piuma-autostart-{}", std::process::id()));
        let system = root.join("system");
        let user = root.join("user");
        std::fs::create_dir_all(&system).expect("temp dirs");
        std::fs::create_dir_all(&user).expect("temp dirs");
        std::fs::write(system.join("dup.desktop"), "[Desktop Entry]\nExec=sysapp\n")
            .expect("write fixture");
        std::fs::write(user.join("dup.desktop"), "[Desktop Entry]\nExec=userapp\nHidden=true\n")
            .expect("write fixture");
        std::fs::write(
            system.join("other.desktop"),
            "[Desktop Entry]\nExec=other\n\n[Desktop Action run]\nExec=nope\n",
        )
        .expect("write fixture");

        let entries = merge(&[system.clone(), user.clone()]);
        assert_eq!(entries.len(), 2);
        let dup = entries.iter().find(|e| e.exec.contains("userapp")).expect("user wins");
        assert!(dup.hidden, "user Hidden=true must suppress the system entry");
        assert!(entries.iter().any(|e| e.exec == "other"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// The running test binary counts as "already running".
    #[test]
    fn running_process_is_detected() {
        let exe = std::env::current_exe().expect("test binary path");
        let name = exe.file_name().expect("test binary name").to_string_lossy();
        assert!(already_running(&name));
    }

    /// The editor lists each file name once with the winning file's
    /// label and the effective switch — the spec state merged with the
    /// config's selection — and the switch only rewrites that list: no
    /// autostart file is ever written, so other desktop environments
    /// keep their own, unaffected view. Plain file names only.
    #[test]
    fn editor_lists_apps_and_toggles_the_config_list() {
        let root =
            std::env::temp_dir().join(format!("piuma-autostart-editor-{}", std::process::id()));
        let system = root.join("system");
        std::fs::create_dir_all(&system).expect("temp dir");
        let user = root.join("user"); // deliberately not created: nothing is ever written there
        std::fs::write(
            system.join("a.desktop"),
            "[Desktop Entry]\nName=Alpha\nExec=alpha\n",
        )
        .expect("write fixture");
        std::fs::write(
            system.join("b.desktop"),
            "[Desktop Entry]\nName=Beta\nExec=beta\nHidden=true\n",
        )
        .expect("write fixture");
        let dirs = vec![system, user.clone()];

        let apps = editor::apps_in(&dirs, &[]);
        assert_eq!(apps.len(), 2, "one row per file name, sorted");
        assert_eq!(apps[0].file, "a.desktop");
        assert_eq!(apps[0].label, "Alpha");
        assert!(apps[0].spec, "the file itself says run");
        assert!(apps[0].enabled);
        assert_eq!(apps[1].label, "Beta");
        assert!(!apps[1].spec, "the system's Hidden=true is the spec state");
        assert!(!apps[1].enabled, "and it shows as off");

        // The config's selection flips the effective switch only …
        let mut disabled = Vec::new();
        editor::toggle_disabled(&mut disabled, "a.desktop", false).expect("disable");
        assert_eq!(disabled, vec!["a.desktop".to_string()]);
        let apps = editor::apps_in(&dirs, &disabled);
        assert!(!apps[0].enabled, "the config's selection shows as off");
        assert!(apps[0].spec, "the spec state is unaffected");
        // … and back.
        editor::toggle_disabled(&mut disabled, "a.desktop", true).expect("enable");
        assert!(disabled.is_empty());
        assert!(editor::apps_in(&dirs, &disabled)[0].enabled);

        // Toggling twice does not duplicate the name.
        editor::toggle_disabled(&mut disabled, "a.desktop", false).expect("disable");
        editor::toggle_disabled(&mut disabled, "a.desktop", false).expect("disable again");
        assert_eq!(disabled.len(), 1);

        // A path is refused before it ever reaches anything.
        assert!(editor::toggle_disabled(&mut disabled, "../evil.desktop", false).is_err());
        assert!(editor::toggle_disabled(&mut disabled, "evil", false).is_err());
        assert!(editor::toggle_disabled(&mut disabled, "", false).is_err());

        // No autostart directory was created or written.
        assert!(!user.exists(), "the editor never writes the autostart dirs");

        let _ = std::fs::remove_dir_all(root);
    }

    /// The config's selection is pruned against the merged files: a name
    /// whose `.desktop` is gone disappears, the living ones stay, and
    /// the result comes out sorted and deduplicated.
    #[test]
    fn prune_drops_removed_files() {
        let existing: BTreeSet<String> = ["a.desktop", "b.desktop"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let stale = vec![
            "b.desktop".to_string(),
            "gone.desktop".to_string(),
            "a.desktop".to_string(),
            "b.desktop".to_string(),
        ];
        assert_eq!(
            prune_disabled(&existing, &stale),
            vec!["a.desktop".to_string(), "b.desktop".to_string()]
        );
        assert!(prune_disabled(&existing, &[]).is_empty());
    }

    /// The XDG fallback rule (regression): an absolute `$XDG_CONFIG_HOME`
    /// wins, a relative one falls through to `$HOME`, and the `$HOME`
    /// fallback is `$HOME/.config` — the dir the Autostart spec calls
    /// the user's config home. Returning bare `$HOME` made the scan look
    /// at `~/autostart` and miss `~/.config/autostart` entirely.
    #[test]
    fn config_home_follows_the_xdg_spec() {
        assert_eq!(
            config_home_from(Some("/x".into()), Some("/h".into())),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(
            config_home_from(Some("relative".into()), Some("/h".into())),
            Some(PathBuf::from("/h/.config"))
        );
        assert_eq!(
            config_home_from(None, Some("/h".into())),
            Some(PathBuf::from("/h/.config"))
        );
        assert_eq!(config_home_from(None, None), None);
        assert_eq!(config_home_from(None, Some("relative".into())), None);
    }
}
