//! Workspace buttons for the `workspace` sensor (roadmap2 #4).
//!
//! The sensor renders one button per available workspace — its name
//! shown, click to focus — instead of a text value, mirroring the
//! tray's icon row: every button carries its own mouse handling, so
//! the sensor box stays bare.

use crate::Message;
use iced::widget::{text, MouseArea};
use iced::{Color, Element, Font, Length};

/// One available workspace as the bar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceButton {
    /// Workspace name as hyprland reports it (what the dispatch takes).
    pub name: String,
    /// Whether this workspace was active when the list was built.
    pub active: bool,
}

/// Colours and border of the sensor around the buttons (mirrors
/// `tray::Style`); the colours arrive concrete so the active inversion
/// never has to guess a fallback.
pub struct Style {
    pub bg_color: Color,
    pub text_color: Color,
    pub border_color: Option<Color>,
    pub border_width: u16,
    pub border_radius: u16,
    /// `full`, `corners`, `underline`, `upperline`, `left` or `right`.
    pub border_type: String,
    pub padding: u16,
}

/// One button per available workspace: normal workspaces only (the
/// `special:*` overlays are toggled, not focused), ids in ascending
/// order, the active one flagged so [`row`] can emphasise it. Any
/// hyprctl failure yields nothing — the sensor then hides, exactly
/// like a tray with no items.
pub fn buttons() -> Vec<WorkspaceButton> {
    let Ok(list) = hyprctl(["workspaces", "-j"]) else {
        return Vec::new();
    };
    let active = hyprctl(["activeworkspace", "-j"]).unwrap_or_default();
    buttons_from(&list, &active)
}

/// `hyprctl <args…>` stdout; a missing command or a non-zero exit is
/// an error the caller turns into "no buttons".
fn hyprctl(args: [&str; 2]) -> Result<String, ()> {
    let out = std::process::Command::new("hyprctl").args(args).output().map_err(|_| ())?;
    if !out.status.success() {
        return Err(());
    }
    String::from_utf8(out.stdout).map_err(|_| ())
}

/// Pure parse of the two hyprctl documents (testable without a
/// compositor): `special:*` entries drop out, ids sort ascending and
/// the active workspace's name flags its button.
fn buttons_from(workspaces: &str, active: &str) -> Vec<WorkspaceButton> {
    let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(workspaces) else {
        return Vec::new();
    };
    let active_name: String = serde_json::from_str::<serde_json::Value>(active)
        .ok()
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .unwrap_or_default();
    let mut items: Vec<(i64, String)> = list
        .iter()
        .filter_map(|w| {
            let id = w.get("id").and_then(|i| i.as_i64())?;
            let name = w.get("name").and_then(|n| n.as_str())?.to_string();
            if name.starts_with("special:") {
                return None;
            }
            Some((id, name))
        })
        .collect();
    items.sort_by_key(|(id, _)| *id);
    items
        .into_iter()
        .map(|(_, name)| WorkspaceButton {
            active: name == active_name,
            name,
        })
        .collect()
}

/// Focus a workspace through hyprland 0.56's Lua dispatch — the
/// classic `hyprctl dispatch <name> <arg>` form no longer parses (the
/// CLI builds a Lua snippet; their own keybinds use `hl.dsp.focus`).
/// Runs in the background like every sensor action; the name is quoted
/// so an odd workspace name cannot break the snippet.
pub fn focus(name: &str) {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    let dispatch = format!("hl.dsp.focus({{ workspace = \"{escaped}\" }})");
    std::thread::spawn(move || {
        let _ = std::process::Command::new("hyprctl").args(["dispatch", &dispatch]).spawn();
    });
}

/// The button row: every button is the sensor's own box (same palette,
/// border and padding) with the active one inverted — the background
/// takes the text colour — and its own press to focus that workspace.
pub fn row<'a>(
    buttons: &'a [WorkspaceButton],
    font_size: u32,
    font: Font,
    style: &Style,
) -> Element<'a, Message> {
    let mut row = iced::widget::Row::new().spacing(4);
    for ws in buttons {
        let (bg, fg) = if ws.active {
            (Some(style.text_color), style.bg_color)
        } else {
            (Some(style.bg_color), style.text_color)
        };
        let label = crate::styled_box(
            text(&ws.name).color(fg).size(font_size).font(font).into(),
            bg,
            style.border_color,
            style.border_width,
            style.border_radius,
            &style.border_type,
            // Sensors sit inside sections: never on a screen edge.
            [false, false, false, false],
            style.padding,
            Length::Shrink,
            Length::Shrink,
        );
        row = row.push(
            MouseArea::new(label)
                .on_press(Message::WorkspaceClicked(ws.name.clone()))
                .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    row.into()
}

// The row itself is a widget composition (covered by the bar's E2E);
// the parse is the logic worth pinning down.
#[cfg(test)]
mod tests {
    use super::*;

    /// Roadmap2 #4: ids ascending, `special:*` overlays left out, the
    /// active workspace flagged; junk input yields no buttons (the
    /// sensor then hides instead of showing a stale list).
    #[test]
    fn buttons_from_hyprctl_json() {
        let list = r#"[{"id":10,"name":"10"},{"id":1,"name":"1"},{"id":-99,"name":"special:scratchpad"},{"id":268435455,"name":"mail"}]"#;
        let active = "{\n    \"id\": 1,\n    \"name\": \"1\"\n}";
        let buttons = buttons_from(list, active);
        let names: Vec<&str> = buttons.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["1", "10", "mail"]);
        assert!(buttons[0].active, "the active workspace is flagged");
        assert!(!buttons[1].active);

        assert!(buttons_from("not json at all", active).is_empty());
        assert!(buttons_from("[]", active).is_empty());
    }
}
