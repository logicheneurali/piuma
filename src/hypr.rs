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

impl Snapshot {
    /// Whether the focused window is fullscreen and its rect covers the
    /// global point `at`: such a window sits above the bar's layer, so a
    /// surface the bar still shows there (its tooltip) is really hidden.
    pub fn focused_covers(&self, at: (i32, i32)) -> bool {
        self.clients
            .as_deref()
            .and_then(|rows| rows.iter().find(|row| row.focused))
            .is_some_and(|row| {
                row.fullscreen
                    && (row.x..row.x + row.w).contains(&at.0)
                    && (row.y..row.y + row.h).contains(&at.1)
            })
    }
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

    /// The shared snapshot's focused window covers a point only when it
    /// is fullscreen and its own rect contains it: another output, a
    /// maximized window (the bar's strip stays clear) and an unfocused
    /// fullscreen window do not count.
    #[test]
    fn focused_covers_only_a_fullscreen_rect_over_the_point() {
        let clients = |json: &str| Snapshot {
            clients: Some(windows::parse_clients(json)),
            ..Default::default()
        };
        let game = clients(
            r#"[{"address":"0x1","class":"game","title":"game","workspace":{"name":"1"},
                 "focusHistoryID":0,"at":[0,0],"size":[1920,1080],"fullscreen":2},
                {"address":"0x2","class":"app","title":"app","workspace":{"name":"2"},
                 "focusHistoryID":1,"at":[1920,0],"size":[2560,1600],"fullscreen":0}]"#,
        );
        assert!(game.focused_covers((960, 500)), "point on the game's output");
        assert!(!game.focused_covers((2000, 500)), "point on the other output");

        // Mode 1 (within the gaps) leaves the bar's strip clear.
        let maximized = clients(
            r#"[{"address":"0x1","class":"app","title":"app","workspace":{"name":"1"},
                 "focusHistoryID":0,"at":[0,24],"size":[1920,1056],"fullscreen":1}]"#,
        );
        assert!(maximized.focused_covers((960, 500)));
        assert!(!maximized.focused_covers((960, 12)), "bar strip not covered");

        let other = clients(
            r#"[{"address":"0x1","class":"game","title":"game","workspace":{"name":"1"},
                 "focusHistoryID":0,"at":[0,0],"size":[1920,1080],"fullscreen":0},
                {"address":"0x2","class":"app","title":"app","workspace":{"name":"2"},
                 "focusHistoryID":1,"at":[0,0],"size":[1920,1080],"fullscreen":2}]"#,
        );
        assert!(!other.focused_covers((960, 500)), "unfocused fullscreen");
        assert!(!Snapshot::default().focused_covers((960, 500)), "no clients");
    }

    /// `activeworkspace -j` → its name; junk yields none.
    #[test]
    fn active_workspace_name_comes_from_its_document() {
        let active = "{\n    \"id\": 1,\n    \"name\": \"1\"\n}";
        assert_eq!(parse_active(active).as_deref(), Some("1"));
        assert!(parse_active("nope").is_none());
    }
}
