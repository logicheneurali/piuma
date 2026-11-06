//! One Hyprland read per tick, shared by every sensor that asks the
//! compositor: `clients -j` feeds the taskbar slots and the window
//! title (its focused row), the workspace document feeds the buttons
//! and the text sensor. Before this module each sensor forked and
//! parsed its own copy — five forks for three facts, with the same
//! JSON parsed twice in the same second.

use crate::windows;

/// One available workspace as the compositor lists it: `special:*`
/// overlays are already dropped and ids sort ascending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
}

/// The compositor state the sensors share for one tick. A `None` part
/// is a failed document: every consumer degrades the way its own
/// sensor already did (no slots, "No Focus", no buttons).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Mapped windows (`hyprctl clients -j`), compositor order.
    pub clients: Option<Vec<windows::WinRow>>,
    /// Available workspaces (`hyprctl workspaces -j`).
    pub workspaces: Option<Vec<Workspace>>,
    /// The active workspace's name (`hyprctl activeworkspace -j`).
    pub active_workspace: Option<String>,
}

/// Fetch every document once — three forks instead of one per sensor —
/// and parse each of them here, once.
pub fn snapshot() -> Snapshot {
    Snapshot {
        clients: fetch("clients").map(|json| {
            let mut rows = windows::parse_clients(&json);
            windows::sort_rows(&mut rows);
            rows
        }),
        workspaces: fetch("workspaces").and_then(|json| parse_workspaces(&json)),
        active_workspace: fetch("activeworkspace").and_then(|json| parse_active(&json)),
    }
}

/// A fresh `hyprctl clients -j`: the measured layout resizes re-read
/// right after every dispatch (the panel's own snapshot lags one tick
/// behind).
pub(crate) fn fresh_clients() -> Option<Vec<windows::WinRow>> {
    fetch("clients").map(|json| windows::parse_clients(&json))
}

/// Whether the focused window is fullscreen, from the shared snapshot
/// (no fork): the auto-hide tweak reads it every tick.
pub fn focused_is_fullscreen(snapshot: Option<&Snapshot>) -> bool {
    snapshot
        .and_then(|s| s.clients.as_deref())
        .unwrap_or(&[])
        .iter()
        .any(|row| row.focused && row.fullscreen)
}

/// Whether the focused window is fullscreen (1 = within the gaps, 2 =
/// bare): the idle screensaver stays off then, so a game or a video
/// that does not inhibit idle is never covered. `None` when the
/// compositor cannot answer — the caller treats it as "not fullscreen",
/// an unreachable compositor must not disable the feature silently.
pub fn active_is_fullscreen() -> Option<bool> {
    let json = fetch("activewindow")?;
    parse_fullscreen(&json)
}

/// `fullscreen` (0/1/2) of the `activewindow` document: missing keys
/// mean no focused window, which is not fullscreen.
fn parse_fullscreen(json: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(v.get("fullscreen").and_then(|f| f.as_u64()).unwrap_or(0) != 0)
}

/// `hyprctl <cmd> -j` stdout, or `None` when the command fails (the
/// sensor's own "no data" path).
fn fetch(cmd: &str) -> Option<String> {
    let out = std::process::Command::new("hyprctl")
        .args([cmd, "-j"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `workspaces -j` → the list every button shows: `special:*`
/// overlays are toggles, not focus targets, so they drop out; junk in
/// means no list out (the sensor hides instead of showing garbage).
fn parse_workspaces(json: &str) -> Option<Vec<Workspace>> {
    let list = serde_json::from_str::<Vec<serde_json::Value>>(json).ok()?;
    let mut items: Vec<Workspace> = list
        .iter()
        .filter_map(|w| {
            let id = w.get("id").and_then(|i| i.as_i64())?;
            let name = w.get("name").and_then(|n| n.as_str())?.to_string();
            (!name.starts_with("special:")).then_some(Workspace { id, name })
        })
        .collect();
    items.sort_by_key(|w| w.id);
    Some(items)
}

/// `activeworkspace -j` → its name.
fn parse_active(json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `workspaces -j` → ids ascending, `special:*` overlays left out,
    /// junk input yields no list.
    #[test]
    fn workspaces_drop_special_and_sort_ascending() {
        let list = r#"[{"id":10,"name":"10"},{"id":1,"name":"1"},{"id":-99,"name":"special:scratchpad"},{"id":268435455,"name":"mail"}]"#;
        let ws = parse_workspaces(list).unwrap();
        let names: Vec<&str> = ws.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, vec!["1", "10", "mail"]);
        assert!(parse_workspaces("not json at all").is_none());
        assert!(parse_workspaces("[]").unwrap().is_empty());
    }

    /// The snapshot check for auto-hide: the focused row must be
    /// fullscreen, a fullscreen window that is not focused must not hide
    /// the bar.
    #[test]
    fn focused_fullscreen_reads_the_focused_row() {
        let row = |focused: bool, fullscreen: bool| crate::windows::WinRow {
            address: "0x1".into(),
            class: "app".into(),
            title: "t".into(),
            workspace: "1".into(),
            focused,
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            floating: false,
            fullscreen,
            visible: true,
            pinned: false,
        };
        assert!(!focused_is_fullscreen(None));
        assert!(!focused_is_fullscreen(Some(&Snapshot {
            clients: Some(vec![row(true, false)]),
            ..Default::default()
        })));
        assert!(focused_is_fullscreen(Some(&Snapshot {
            clients: Some(vec![row(true, true)]),
            ..Default::default()
        })));
        assert!(
            !focused_is_fullscreen(Some(&Snapshot {
                clients: Some(vec![row(false, true)]),
                ..Default::default()
            })),
            "a fullscreen window without focus keeps the bar"
        );
    }

    /// Only the focused window's `fullscreen` matters: 1 and 2 both
    /// count, a missing key (no focused window) does not, and a
    /// document that is not JSON yields `None`.
    #[test]
    fn fullscreen_comes_from_the_active_window() {
        assert_eq!(parse_fullscreen(r#"{"fullscreen":0}"#), Some(false));
        assert_eq!(parse_fullscreen(r#"{"fullscreen":1}"#), Some(true));
        assert_eq!(parse_fullscreen(r#"{"fullscreen":2}"#), Some(true));
        assert_eq!(parse_fullscreen("{}"), Some(false), "no window");
        assert_eq!(parse_fullscreen("not json"), None);
    }

    /// `activeworkspace -j` → its name; junk yields none.
    #[test]
    fn active_workspace_name_comes_from_its_document() {
        let active = "{\n    \"id\": 1,\n    \"name\": \"1\"\n}";
        assert_eq!(parse_active(active).as_deref(), Some("1"));
        assert!(parse_active("nope").is_none());
    }
}
