//! Workspace buttons for the `workspace` sensor (roadmap2 #4).
//!
//! The sensor renders one button per available workspace — its name
//! shown, click to focus — instead of a text value, mirroring the
//! tray's icon row: every button carries its own mouse handling, so
//! the sensor box stays bare.

use crate::Message;
use iced::widget::{text, Column, MouseArea, Row};
use iced::{Alignment, Color, Element, Font, Length};

/// Gap between buttons and between wrapped lines (mirrors the tray's).
pub const ROW_GAP: u16 = 4;

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

/// One button per available workspace from the shared snapshot (the
/// list is already `special:*`-free and id-ordered, see `hypr.rs`):
/// the active one flagged so [`row`] can emphasise it. A failed
/// document yields nothing — the sensor then hides, exactly like a
/// tray with no items.
pub fn buttons(snap: Option<&crate::hypr::Snapshot>) -> Vec<WorkspaceButton> {
    let Some(snap) = snap else {
        return Vec::new();
    };
    let Some(list) = &snap.workspaces else {
        return Vec::new();
    };
    let active = snap.active_workspace.as_deref();
    list.iter()
        .map(|w| WorkspaceButton {
            name: w.name.clone(),
            active: Some(w.name.as_str()) == active,
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

/// Estimated width of the widest button — buttons are text-shaped, so
/// the width follows the longest name: its characters at ~0.6em each (a
/// deliberate overestimate for typical UI fonts — the wrapped lines
/// then never overflow the sensor box) plus the padding on both sides.
pub fn slot_width(buttons: &[WorkspaceButton], font_size: u32, padding: u16) -> u32 {
    let chars = buttons
        .iter()
        .map(|ws| ws.name.chars().count())
        .max()
        .unwrap_or(1)
        .max(1) as u32;
    2 * u32::from(padding) + (chars * font_size * 6).div_ceil(10)
}

/// The button lines: every button is the sensor's own box (same palette,
/// border and padding) with the active one inverted — the background
/// takes the text colour — and its own press to focus that workspace.
/// The buttons wrap into as many lines as needed (`per_line`, from the
/// widest name estimate in [`slot_width`]), mirroring the tray row, so
/// they never spill out of the sensor box.
pub fn row<'a>(
    buttons: &'a [WorkspaceButton],
    font_size: u32,
    font: Font,
    style: &Style,
    per_line: u16,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(f32::from(ROW_GAP));
    for chunk in buttons.chunks(usize::from(per_line.max(1))) {
        let mut row = Row::new()
            .spacing(f32::from(ROW_GAP))
            .align_y(Alignment::Center);
        for ws in chunk {
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
                iced::Padding::from(style.padding),
                Length::Shrink,
                Length::Shrink,
            );
            row = row.push(
                MouseArea::new(label)
                    .on_press(Message::WorkspaceClicked(ws.name.clone()))
                    .interaction(iced::mouse::Interaction::Pointer),
            );
        }
        col = col.push(row);
    }
    col.into()
}

// The row itself is a widget composition (covered by the bar's E2E);
// the parse is the logic worth pinning down.
#[cfg(test)]
mod tests {
    use super::*;

    /// The active workspace of the snapshot flags its button; a
    /// missing snapshot yields no buttons at all (the sensor hides).
    #[test]
    fn buttons_flag_the_active_workspace() {
        let snap = crate::hypr::Snapshot {
            clients: None,
            workspaces: Some(vec![
                crate::hypr::Workspace {
                    id: 1,
                    name: "1".into(),
                },
                crate::hypr::Workspace {
                    id: 10,
                    name: "10".into(),
                },
            ]),
            active_workspace: Some("1".into()),
        };
        let made = buttons(Some(&snap));
        assert_eq!(made.len(), 2);
        assert!(made[0].active, "the active workspace is flagged");
        assert!(!made[1].active);
        assert!(buttons(None).is_empty());
    }

    /// The estimated slot is padding plus the widest name's characters
    /// (~0.6em each, rounded up): a longer name widens it — the wrap
    /// maths in `icons_per_line` then packs fewer buttons per line.
    #[test]
    fn slot_width_follows_the_widest_name_and_padding() {
        let short = [WorkspaceButton {
            name: "1".into(),
            active: false,
        }];
        let long = [
            WorkspaceButton {
                name: "1".into(),
                active: false,
            },
            WorkspaceButton {
                name: "10".into(),
                active: false,
            },
        ];
        assert_eq!(crate::workspace::slot_width(&short, 12, 4), 16);
        assert_eq!(crate::workspace::slot_width(&long, 12, 4), 23);
    }
}
