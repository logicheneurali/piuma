//! The `taskbar` sensor: the open windows read from `hyprctl
//! clients -j` at every sensor tick (one fork per second, like every
//! polling sensor — no watcher) and the Hyprland 0.56 Lua dispatches
//! behind the icon clicks. Each entry carries its window's address, so
//! a click targets that exact window no matter what has the focus by
//! the time it lands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::icons;
use std::sync::atomic::{AtomicU16, Ordering};

/// Icon edge in px before the first tick computes the real one.
const ICON_SIZE: u16 = 16;

/// The slot's own padding around the icon (the hover fill covers the
/// whole slot); main subtracts it from the row height as well.
pub const SLOT_PAD: u16 = 2;
/// Gap the row puts between two slots — part of one slot's width.
pub const ROW_GAP: u16 = 4;

/// One slot's total width in the row: the icon, its padding both sides
/// and the gap after it — the number `main` needs to cap the row to the
/// sensor's room.
pub fn slot_width() -> u32 {
    u32::from(icon_target()) + 2 * u32::from(SLOT_PAD) + u32::from(ROW_GAP)
}

/// The icon edge in px the taskbar renders at right now; main computes
/// it from the sensor row's usable height (`icons::row_icon_target`,
/// like the tray's) and the resolution reads it too — the theme
/// lookup is memoized by size, so both sides must be this number.
static ICON_TARGET: AtomicU16 = AtomicU16::new(ICON_SIZE);

/// The icon edge in px the taskbar renders at right now.
pub fn icon_target() -> u16 {
    ICON_TARGET.load(Ordering::Relaxed)
}

/// Store the icon edge (main computes it from the sensor's usable
/// height); already-resolved slots re-resolve on the next tick.
pub fn set_icon_target(size: u16) {
    ICON_TARGET.store(size, Ordering::Relaxed);
}

/// One open window as the sensor lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinRow {
    /// Hyprland's address (`0x…`): the dispatches' target.
    pub address: String,
    /// WM_CLASS as the app sets it — the icon lookup key.
    pub class: String,
    pub title: String,
    /// Workspace name ("1", "special:…").
    pub workspace: String,
    /// `focusHistoryID == 0`: the window Hyprland reports as focused
    /// right now — highlighted in the row.
    pub focused: bool,
}

/// One taskbar slot: the address the clicks dispatch against, the
/// resolved theme icon (`None` = fall back to the generic executable),
/// the title shown on hover and the active flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskIcon {
    pub address: String,
    pub icon: Option<PathBuf>,
    pub title: String,
    pub focused: bool,
}

/// Visual parameters for the row: the slot corner radius plus the two
/// fills — the hovered slot wins over the active window's.
pub struct RowStyle {
    /// The sensor's corner radius: the fills follow it.
    pub radius: u16,
    /// Fill behind the slot under the cursor.
    pub hover_bg: Option<iced::Color>,
    /// Fill behind the active window's slot.
    pub active_bg: Option<iced::Color>,
}

/// The taskbar's slots: one per window of the shared snapshot, in
/// list order, each with its app icon resolved through the memoized
/// theme lookup (the per-tick cost is a map clone; the first lookup
/// may walk the theme once). A failed document yields no slots — the
/// sensor hides, like a tray with no items.
pub fn task_icons(snap: Option<&crate::hypr::Snapshot>) -> Vec<TaskIcon> {
    let Some(rows) = snap.and_then(|s| s.clients.as_ref()) else {
        return Vec::new();
    };
    let target = icon_target();
    rows.iter()
        .map(|row| {
            let icon = app_icon(&row.class)
                .and_then(|name| icons::resolve(icons::icon_theme(), &name, target))
                .or_else(|| {
                    icons::resolve(icons::icon_theme(), "application-x-executable", target)
                });
            TaskIcon {
                address: row.address.clone(),
                icon,
                title: row.title.clone(),
                focused: row.focused,
            }
        })
        .collect()
}

/// `hyprctl clients -j` → rows: mapped windows only (unmapped ones are
/// mid-transition), with the address sanitized down to bare hex — it
/// becomes a dispatch target later. (`hypr::snapshot` fetches the
/// document once for every sensor that needs it.)
pub(crate) fn parse_clients(json: &str) -> Vec<WinRow> {
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
                // `focusHistoryID` counts back from the focused window:
                // 0 is the active one (absent → not focused).
                focused: c.get("focusHistoryID").and_then(|v| v.as_i64()) == Some(0),
            })
        })
        .collect()
}

/// Workspace order (numeric names first, in numeric order — "2" before
/// "10" — named/special ones after), then title case-insensitively.
/// (`hypr::snapshot` sorts the shared list with it.) The lowercase is
/// computed once per row — not twice per comparison — and the
/// permutation applies by clone: rows are four small strings.
pub(crate) fn sort_rows(rows: &mut [WinRow]) {
    let keys: Vec<((u8, u64, &str), String)> = rows
        .iter()
        .map(|r| (ws_key(&r.workspace), r.title.to_lowercase()))
        .collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| {
        keys[a]
            .0
            .cmp(&keys[b].0)
            .then_with(|| keys[a].1.cmp(&keys[b].1))
    });
    let original: Vec<WinRow> = rows.to_vec();
    for (dst, &src) in order.iter().enumerate() {
        rows[dst] = original[src].clone();
    }
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

/// Move a window to a workspace silently: it leaves without dragging
/// the focus along (the bar stays where it is).
pub fn move_to_workspace(address: &str, workspace: u32) {
    run(move_call(address, &workspace.to_string()));
}

fn run(call: String) {
    let _ = std::process::Command::new("hyprctl")
        .args(["dispatch", &call])
        .spawn();
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

/// class → icon name, via the installed .desktop files: their
/// `StartupWMClass=` first, the file name as the second key (the class
/// usually equals it): indexed once per process (misses memoized too),
/// so the first tick pays the directory walk and every later one just
/// clones a map entry (~tens of KB of RAM).
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
                    // Two keys per file: the declared `StartupWMClass`
                    // first (it is what the app reports), then the file
                    // name — most classes equal it (kitty.desktop ↔
                    // class "kitty"), which covers the entries that
                    // declare no WMClass. `or_insert`: a first key (or
                    // the user's duplicate file) is never overridden.
                    if let Some(wm_class) = wm_class {
                        index
                            .entry(wm_class.to_lowercase())
                            .or_insert_with(|| icon.clone());
                    }
                    if let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase())
                    {
                        index.entry(stem).or_insert(icon);
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

// ── view ────────────────────────────────────────────────────────────

/// One icon slot per window: left focuses it, middle requests its
/// close, right opens the move-to-workspace menu under the cursor.
/// The address rides the message — never an index a tick could shift
/// under a queued press. The active window's slot keeps the sensor's
/// fill (hover wins over it) and a non-empty title rides a tooltip,
/// styled like the tray's. `per_line` is the wrap width in icons
/// (`icons::icons_per_line` over the box's room): the slots flow
/// horizontally and turn to the next line — every window visible and
/// clickable, none capped away.
pub fn row<'a>(
    items: &'a [TaskIcon],
    hover: Option<&str>,
    style: &RowStyle,
    per_line: u16,
) -> iced::Element<'a, crate::Message> {
    use iced::widget::{Column, MouseArea, Row, container};
    let radius = f32::from(style.radius);
    let mut col = Column::new().spacing(f32::from(ROW_GAP));
    // Windows without an icon never rendered a slot: they are filtered
    // before chunking so the lines count real icons only.
    let visible: Vec<_> = items
        .iter()
        .filter_map(|item| item.icon.as_deref().map(|path| (item, path)))
        .collect();
    for chunk in visible.chunks(usize::from(per_line)) {
        let mut row = Row::new().spacing(f32::from(ROW_GAP));
        for &(item, path) in chunk {
            let address = item.address.clone();
            // Hover wins over active: the slot under the cursor is the one
            // being acted on.
            let fill = if hover == Some(address.as_str()) {
                style.hover_bg
            } else if item.focused {
                style.active_bg
            } else {
                None
            };
            let slot = container(icons::widget(path, f32::from(icon_target())))
                .padding(SLOT_PAD)
                .style(move |_theme| iced::widget::container::Style {
                    background: fill.map(iced::Background::Color),
                    border: iced::Border {
                        radius: radius.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            let interactive = MouseArea::new(slot)
                .on_press(crate::Message::TaskbarLeft(address.clone()))
                .on_middle_press(crate::Message::TaskbarMiddle(address.clone()))
                .on_right_press(crate::Message::TaskbarRight(address.clone()))
                // Enter fills the slot and opens the title tooltip (empty
                // title → no window); the leave clears only its own
                // address — a stale exit must not drop the next icon's
                // hover or tooltip (see `TaskbarIconLeave`).
                .on_enter(crate::Message::TaskbarIconEnter {
                    address: address.clone(),
                    title: item.title.clone(),
                })
                .on_exit(crate::Message::TaskbarIconLeave { address })
                .interaction(iced::mouse::Interaction::Pointer);
            row = row.push(interactive);
        }
        col = col.push(row);
    }
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
        {"address": "0xAB12", "class": "kitty", "title": "shell", "mapped": true,
         "focusHistoryID": 0, "workspace": {"id": 1, "name": "1"}},
        {"address": "0xCD34", "class": "vivaldi-stable", "title": "Browser", "mapped": true,
         "focusHistoryID": 1, "workspace": {"id": 10, "name": "10"}},
        {"address": "0xEF56", "class": "ghost", "title": "mid-air", "mapped": false,
         "workspace": {"id": 1, "name": "1"}}
    ]"#;

    #[test]
    fn parse_clients_maps_windows_and_drops_unmapped() {
        let rows = parse_clients(CLIENTS);
        assert_eq!(rows.len(), 2, "unmapped windows are mid-transition");
        assert_eq!(rows[0].address, "0xAB12");
        assert_eq!(rows[0].class, "kitty");
        assert_eq!(rows[1].workspace, "10");
    }

    #[test]
    fn parse_clients_survives_garbage() {
        assert!(parse_clients("not json").is_empty());
        assert!(parse_clients("[]").is_empty());
    }

    /// Only `focusHistoryID == 0` is the active window; a background
    /// one (1) and a client with no history at all are not.
    #[test]
    fn parse_clients_marks_only_the_focused_window() {
        let rows = parse_clients(CLIENTS);
        assert!(rows[0].focused, "focusHistoryID 0 is the active window");
        assert!(!rows[1].focused, "history > 0 is background");
        let bare = parse_clients(
            r#"[{"address": "0x1", "class": "c", "title": "t", "mapped": true,
                 "workspace": {"id": 1, "name": "1"}}]"#,
        );
        assert!(!bare[0].focused, "an absent history means not focused");
    }

    #[test]
    fn sort_rows_orders_by_numeric_workspace_then_title() {
        let mut rows = parse_clients(CLIENTS);
        rows.push(WinRow {
            address: "0x1".into(),
            class: "c".into(),
            title: "apple".into(),
            workspace: "2".into(),
            focused: false,
        });
        sort_rows(&mut rows);
        let order: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        // "2" sorts before "10" (numeric, not lexicographic).
        assert_eq!(order, vec!["shell", "apple", "Browser"]);
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
}
