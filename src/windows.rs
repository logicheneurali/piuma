//! The `window` sensor's popup: a snapshot of the open windows read
//! from `hyprctl clients -j` when the sensor is clicked (no watcher —
//! the list is the state of the moment) and the Hyprland 0.56 Lua
//! dispatches behind the row actions. Every row carries its window's
//! address, so an action targets that exact window no matter what has
//! the focus by the time the click lands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// One open window as the list renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinRow {
    /// Hyprland's address (`0x…`): the dispatches' target.
    pub address: String,
    /// WM_CLASS as the app sets it — the icon lookup key.
    pub class: String,
    pub title: String,
    /// Workspace name ("1", "special:…").
    pub workspace: String,
    pub floating: bool,
}

/// The open windows in list order (workspace, then title): one fork of
/// `hyprctl`, nothing kept between opens.
pub fn list() -> Vec<WinRow> {
    let Ok(out) = std::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
    else {
        return Vec::new();
    };
    let json = String::from_utf8_lossy(&out.stdout);
    let mut rows = parse_clients(&json);
    sort_rows(&mut rows);
    rows
}

/// `hyprctl clients -j` → rows: mapped windows only (unmapped ones are
/// mid-transition), with the address sanitized down to bare hex — it
/// becomes a dispatch target later.
fn parse_clients(json: &str) -> Vec<WinRow> {
    let Ok(clients) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    clients
        .iter()
        .filter(|c| c.get("mapped").and_then(|m| m.as_bool()).unwrap_or(true))
        .filter_map(|c| {
            let address = sanitize_address(c.get("address")?.as_str()?);
            Some(WinRow {
                address,
                class: c
                    .get("class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                title: c
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                workspace: c
                    .get("workspace")
                    .and_then(|w| w.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                floating: c.get("floating").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        })
        .collect()
}

/// Workspace order (numeric names first, in numeric order — "2" before
/// "10" — named/special ones after), then title case-insensitively.
fn sort_rows(rows: &mut [WinRow]) {
    rows.sort_by(|a, b| {
        ws_key(&a.workspace)
            .cmp(&ws_key(&b.workspace))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

fn ws_key(ws: &str) -> (u8, u64, &str) {
    match ws.parse::<u64>() {
        Ok(n) => (0, n, ""),
        Err(_) => (1, 0, ws),
    }
}

// ── row actions ─────────────────────────────────────────────────────
//
// Hyprland 0.56 dispatches in Lua form (the classic
// `hyprctl dispatch <name> <arg>` no longer parses); the window-taking
// dispatchers and their names were verified live against this build.

/// The `hyprctl` argument that focuses a window (brings its workspace).
pub fn focus_call(address: &str) -> String {
    format!(
        "hl.dsp.focus{{ window = \"address:{}\" }}",
        sanitize_address(address)
    )
}

/// The argument that requests a graceful close (the app may prompt).
pub fn close_call(address: &str) -> String {
    format!(
        "hl.dsp.window.close({{ window = \"address:{}\" }})",
        sanitize_address(address)
    )
}

/// The argument that toggles floating on a window.
pub fn float_call(address: &str) -> String {
    format!(
        "hl.dsp.window.float({{ window = \"address:{}\" }})",
        sanitize_address(address)
    )
}

/// The argument that silently moves a window to a workspace.
pub fn move_call(address: &str, workspace: &str) -> String {
    let ws: String = workspace
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\n'))
        .collect();
    format!(
        "hl.dsp.window.move({{ workspace = \"{ws}\", window = \"address:{}\", silent = true }})",
        sanitize_address(address)
    )
}

pub fn focus(address: &str) {
    run(focus_call(address));
}

pub fn close_window(address: &str) {
    run(close_call(address));
}

pub fn toggle_float(address: &str) {
    run(float_call(address));
}

/// "Bring here": move the window silently to the current workspace,
/// returning its name (so the caller can refresh the row's badge).
pub fn move_to_current(address: &str) -> Option<String> {
    let workspace = current_workspace()?;
    run(move_call(address, &workspace));
    Some(workspace)
}

fn run(call: String) {
    let _ = std::process::Command::new("hyprctl")
        .args(["dispatch", &call])
        .spawn();
}

fn current_workspace() -> Option<String> {
    let out = std::process::Command::new("hyprctl")
        .args(["activeworkspace", "-j"])
        .output()
        .ok()?;
    let json = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    value.get("name")?.as_str().map(str::to_string)
}

/// An address as the dispatches take it: `0x` plus hex digits only —
/// whatever reached us, no quote or brace can slip into the Lua call.
fn sanitize_address(address: &str) -> String {
    let hex: String = address
        .strip_prefix("0x")
        .unwrap_or(address)
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    format!("0x{hex}")
}

// ── app icons ───────────────────────────────────────────────────────

/// class → icon name, via the installed .desktop files'
/// `StartupWMClass=`: indexed once per process (misses memoized too),
/// so the popup's first open pays the directory walk and every later
/// one just clones a map entry (~tens of KB of RAM).
pub fn app_icon(class: &str) -> Option<String> {
    desktop_index()
        .get(&class.to_lowercase())
        .cloned()
        .flatten()
}

/// The class → icon map, built from the user's then the system's
/// application entries (the user's win on a duplicate class).
fn desktop_index() -> &'static HashMap<String, Option<String>> {
    static INDEX: OnceLock<HashMap<String, Option<String>>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
            roots.push(PathBuf::from(home).join("applications"));
        } else if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".local/share/applications"));
        }
        let dirs = std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
        for dir in dirs.split(':').filter(|d| !d.is_empty()) {
            roots.push(PathBuf::from(dir).join("applications"));
        }
        let mut index: HashMap<String, Option<String>> = HashMap::new();
        let mut pending = roots;
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("desktop"))
                {
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    let (wm_class, icon) = parse_desktop(&content);
                    if let Some(wm_class) = wm_class {
                        index.entry(wm_class.to_lowercase()).or_insert(icon);
                    }
                }
            }
        }
        index
    })
}

/// The two keys a row needs out of a .desktop file (first occurrence
/// wins; comments and other groups are skipped by the key match).
fn parse_desktop(content: &str) -> (Option<String>, Option<String>) {
    let mut wm_class = None;
    let mut icon = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("StartupWMClass=") {
            wm_class.get_or_insert_with(|| value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Icon=") {
            icon.get_or_insert_with(|| value.trim().to_string());
        }
    }
    (
        wm_class.filter(|s| !s.is_empty()),
        icon.filter(|s| !s.is_empty()),
    )
}

/// The window title cut for a row (the popup measures the cut string).
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
        {"address": "0xAB12", "class": "kitty", "title": "shell", "mapped": true,
         "floating": false, "workspace": {"id": 1, "name": "1"}},
        {"address": "0xCD34", "class": "vivaldi-stable", "title": "Browser", "mapped": true,
         "floating": true, "workspace": {"id": 10, "name": "10"}},
        {"address": "0xEF56", "class": "ghost", "title": "mid-air", "mapped": false,
         "floating": false, "workspace": {"id": 1, "name": "1"}}
    ]"#;

    #[test]
    fn parse_clients_maps_windows_and_drops_unmapped() {
        let rows = parse_clients(CLIENTS);
        assert_eq!(rows.len(), 2, "unmapped windows are mid-transition");
        assert_eq!(rows[0].address, "0xAB12");
        assert_eq!(rows[0].class, "kitty");
        assert!(!rows[0].floating);
        assert!(rows[1].floating, "the flags reach the row");
        assert_eq!(rows[1].workspace, "10");
    }

    #[test]
    fn parse_clients_survives_garbage() {
        assert!(parse_clients("not json").is_empty());
        assert!(parse_clients("[]").is_empty());
    }

    #[test]
    fn sort_rows_orders_by_numeric_workspace_then_title() {
        let mut rows = parse_clients(CLIENTS);
        rows.push(WinRow {
            address: "0x1".into(),
            class: "c".into(),
            title: "apple".into(),
            workspace: "2".into(),
            floating: false,
        });
        sort_rows(&mut rows);
        let order: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(order, vec!["shell", "apple", "Browser"]);
        // "2" sorts before "10" (numeric, not lexicographic).
        assert_eq!(rows[0].workspace, "1");
        assert_eq!(rows[1].workspace, "2");
        assert_eq!(rows[2].workspace, "10");
    }

    #[test]
    fn dispatch_calls_address_the_row_exactly() {
        assert_eq!(
            focus_call("0xAB12"),
            "hl.dsp.focus{ window = \"address:0xAB12\" }"
        );
        assert_eq!(
            close_call("0xAB12"),
            "hl.dsp.window.close({ window = \"address:0xAB12\" })"
        );
        assert_eq!(
            float_call("0xAB12"),
            "hl.dsp.window.float({ window = \"address:0xAB12\" })"
        );
        assert_eq!(
            move_call("0xAB12", "3"),
            "hl.dsp.window.move({ workspace = \"3\", window = \"address:0xAB12\", silent = true })"
        );
    }

    #[test]
    fn dispatch_calls_cannot_inject_lua() {
        // Whatever reaches the formatter, the Lua call sees bare hex:
        // no quote, brace or space can slip into the address slot.
        let call = focus_call("0xAB\"} os.execute('x')");
        assert!(
            call.starts_with("hl.dsp.focus{ window = \"address:0x"),
            "{call}"
        );
        assert!(call.ends_with("\" }"), "{call}");
        let residue: String = call
            .trim_start_matches("hl.dsp.focus{ window = \"address:0x")
            .trim_end_matches("\" }")
            .chars()
            .filter(|c| !c.is_ascii_hexdigit())
            .collect();
        assert!(residue.is_empty(), "non-hex residue: {residue}");
        assert!(move_call("0x1", "sp\"ecial").contains("workspace = \"special\""));
    }

    #[test]
    fn parse_desktop_reads_wmclass_and_icon() {
        let (wm_class, icon) = parse_desktop(
            "[Desktop Entry]\nName=Konsole\nIcon=org.kde.konsole\nStartupWMClass=org.kde.konsole\n",
        );
        assert_eq!(wm_class.as_deref(), Some("org.kde.konsole"));
        assert_eq!(icon.as_deref(), Some("org.kde.konsole"));
        // No StartupWMClass: nothing to key the lookup by.
        assert!(parse_desktop("[Desktop Entry]\nIcon=foo\n").0.is_none());
    }

    #[test]
    fn cut_truncates_with_an_ellipsis() {
        assert_eq!(cut("short", 10), "short");
        assert_eq!(cut("abcdef", 4), "abc…");
    }
}
