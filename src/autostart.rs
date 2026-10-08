//! Freedesktop autostart (roadmap N2).
//!
//! When `panel.autostart` is enabled the bar launches, once at startup,
//! the applications marked as autostart under the standard XDG
//! directories, skipping the ones already running. The parser honours
//! the Autostart entry spec: `Hidden`, `X-GNOME-Autostart-enabled`,
//! `OnlyShowIn`/`NotShowIn`, `TryExec`, `Path`, `Exec` field codes, and
//! a user file overriding the system file with the same name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A parsed `.desktop` autostart entry (only the spec keys piuma honours).
#[derive(Debug, Default, Clone)]
struct Entry {
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
}

/// Launch every eligible autostart entry on a background thread so the
/// bar's startup never waits for the spawned applications.
pub fn spawn() {
    let _ = std::thread::Builder::new()
        .name("piuma-autostart".to_string())
        .spawn(run);
}

/// Autostart thread: merge both directories, apply the spec filters and
/// launch what survives — never a program already running, never piuma.
fn run() {
    let desktop = current_desktop();
    for entry in collect() {
        if !should_run(&entry, &desktop) {
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

/// `$XDG_CONFIG_HOME`, falling back to `$HOME/.config` (Autostart spec).
fn user_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home);
    path.is_absolute().then_some(path)
}

/// Merge the given directories by file name (later entries win) and
/// parse every `.desktop` file found; unreadable files are skipped.
fn merge(dirs: &[PathBuf]) -> Vec<Entry> {
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
        .values()
        .filter_map(|path| parse(&std::fs::read_to_string(path).ok()?))
        .collect()
}

/// Entries of the live directories (see [`autostart_dirs`]).
fn collect() -> Vec<Entry> {
    merge(&autostart_dirs())
}

/// Parse the `[Desktop Entry]` group of a `.desktop` file; a file
/// without `Exec` is not an autostart candidate.
fn parse(text: &str) -> Option<Entry> {
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
            _ => {}
        }
    }
    (!entry.exec.is_empty()).then_some(entry)
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
fn program_of(exec: &str) -> String {
    let exec = exec.trim_start();
    if let Some(rest) = exec.strip_prefix('"') {
        let end = rest.find('"').unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    exec.split_whitespace().next().unwrap_or("").to_string()
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

/// "se non già in esecuzione": scan `/proc/*/cmdline` for a process
/// whose first argument has the same name as `program`.
fn already_running(program: &str) -> bool {
    let Some(want) = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir("/proc") else { return false };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
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
}
