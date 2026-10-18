//! The bar tooltip: a small overlay layer window that follows the
//! cursor, offset from it, never over the bar itself, and measuring
//! its text line by line (a tray tooltip carries a title and a body
//! on two lines).

use crate::menus::{menu_row_h, MENU_PAD};
use crate::text_width;
use crate::tray;
use crate::{
    panel_anchor, panel_font, panel_thickness, popup_output_option, popup_style, styled_box,
    BarTooltip, DEFAULT_FONT_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH,
};
use crate::{Message, Panel};
use iced::font::Font;
use iced::widget::{container, text};
use iced::{Alignment, Element, Length, Task as Command};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, Margin, NewLayerShellSettings,
};
use iced_exwlshell::settings::LayerSize;
/// Distance between a tooltip and the widget it describes (icons and
/// tray items): enough to read it as attached, never on the widget.
pub(crate) const TOOLTIP_GAP: f32 = 8.0;

/// Anchor + edge margins for a tooltip window at the cursor point:
/// just off the bar's anchored edge on the screen's inner side (a top
/// bar's tooltip drops below the surface, never onto it — the bar
/// surface is one `thickness` tall, there is no room inside it),
/// centred on the cursor along the bar and clamped inside the output.
pub(crate) fn tooltip_placement(
    anchor: Anchor,
    at: (i32, i32),
    w: u32,
    h: u32,
    thickness: i32,
    sw: i32,
    sh: i32,
) -> (Anchor, Margin) {
    let (w, h) = (w as i32, h as i32);
    let gap = TOOLTIP_GAP as i32;
    if anchor.contains(Anchor::Top) {
        let left = (at.0 - w / 2).clamp(0, (sw - w).max(0));
        (
            Anchor::Left | Anchor::Top,
            Margin {
                left,
                top: thickness + gap,
                right: 0,
                bottom: 0,
            },
        )
    } else if anchor.contains(Anchor::Bottom) {
        let left = (at.0 - w / 2).clamp(0, (sw - w).max(0));
        (
            Anchor::Left | Anchor::Bottom,
            Margin {
                left,
                top: 0,
                right: 0,
                bottom: thickness + gap,
            },
        )
    } else if anchor.contains(Anchor::Left) {
        let top = (at.1 - h / 2).clamp(0, (sh - h).max(0));
        (
            Anchor::Left | Anchor::Top,
            Margin {
                left: thickness + gap,
                top,
                right: 0,
                bottom: 0,
            },
        )
    } else {
        let top = (at.1 - h / 2).clamp(0, (sh - h).max(0));
        (
            Anchor::Right | Anchor::Top,
            Margin {
                left: 0,
                top,
                right: thickness + gap,
                bottom: 0,
            },
        )
    }
}

/// The cursor point for tooltip placement, output-relative: the
/// compositor's own view (`hyprctl cursorpos`) — `last_cursor` is
/// stale when the pointer enters the bar straight from a window below
/// (a layer surface only receives motion once the cursor is inside),
/// and the tooltip would open at the previous hover's spot.
pub(crate) fn tooltip_cursor_point(panel: &Panel) -> (i32, i32) {
    if let Some((gx, gy)) = hyprctl_cursor_global() {
        let (ox, oy) = tray::output_origin();
        return (gx - ox, gy - oy);
    }
    tray::cursor_output_point(panel).unwrap_or((0, 0))
}

/// The cursor's global position straight from the compositor.
pub(crate) fn hyprctl_cursor_global() -> Option<(i32, i32)> {
    let out = std::process::Command::new("hyprctl")
        .args(["cursorpos", "-j"])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some((v["x"].as_i64()? as i32, v["y"].as_i64()? as i32))
}

/// The tooltip surface metrics for a text: every `\n`-separated line
/// is fitted to `budget` (like the menu labels), the surface is as
/// wide as the widest fitted line and one `menu_row_h` per line tall —
/// so a multi-line SNI tooltip (`title\nbody`) is measured, not cut to
/// one line's height. Returns (width, height, fitted text).
pub(crate) fn tooltip_fit(
    font: &Font,
    font_size: u32,
    text: &str,
    budget: u32,
) -> (u32, u32, String) {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    let jobs: Vec<(String, u32)> = lines.into_iter().map(|line| (line, budget)).collect();
    let fitted = text_width::fit(font, font_size, &jobs);
    let width = u32::from(text_width::widest(font, font_size, &fitted));
    let height = menu_row_h(font_size) * fitted.len() as u32;
    (width, height, fitted.join("\n"))
}

/// Open (or retarget) the tooltip window for a bar source: the text is
/// fitted to half the output width, so a long title is cut like the
/// tray menu's labels instead of stretching off-screen.
pub(crate) fn tooltip_open(panel: &mut Panel, id: String, text: String) -> Command<Message> {
    if text.is_empty() {
        return Command::none();
    }
    // Already this source's tooltip: nothing to rebuild.
    if panel
        .bar_tooltip
        .as_ref()
        .is_some_and(|(_, tip)| tip.id == id)
    {
        return Command::none();
    }
    let mut cmds: Vec<Command<Message>> = Vec::new();
    if let Some((win, _)) = panel.bar_tooltip.take() {
        cmds.push(iced::window::close(win));
    }
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
    let budget = (sw as u32 / 2).saturating_sub(2 * MENU_PAD);
    let (text_w, text_h, fitted) = tooltip_fit(&font, font_size, &text, budget);
    let w = text_w.saturating_add(2 * MENU_PAD).max(48);
    let h = text_h.saturating_add(2 * MENU_PAD);
    let at = tooltip_cursor_point(panel);
    let thickness = panel_thickness(&panel.config) as i32;
    let (anchor, margin) =
        tooltip_placement(panel_anchor(&panel.config), at, w, h, thickness, sw, sh);
    let (win, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(w, h),
        layer: Layer::Overlay,
        anchor,
        // `-1` = ignore other surfaces' exclusive zones: the default
        // (`None` → protocol default 0) had the tooltip land *below*
        // the bar's own 30 px reservation — `thickness + gap` turned
        // into `thickness + gap + thickness`.
        exclusive_zone: Some(-1),
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: popup_output_option(),
        namespace: Some("piuma-tooltip".to_string()),
        ..Default::default()
    });
    panel.bar_tooltip = Some((win, BarTooltip { id, text: fitted }));
    cmds.push(open);
    Command::batch(cmds)
}

/// Close the tooltip — only if `id` still owns it: an exit delivered
/// after another source's enter (widget order) is stale.
pub(crate) fn tooltip_close(panel: &mut Panel, id: String) -> Command<Message> {
    if let Some((win, tip)) = panel.bar_tooltip.take() {
        if tip.id == id {
            return iced::window::close(win);
        }
        panel.bar_tooltip = Some((win, tip));
    }
    Command::none()
}

/// The tooltip window: one fitted line in the popup style — a label
/// just off the bar (see [`tooltip_placement`]), never a strip inside
/// the bar surface itself.
pub(crate) fn tooltip_window_view(panel: &Panel) -> Element<'_, Message> {
    let Some((_, tip)) = panel.bar_tooltip.as_ref() else {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let body = container(
        text(&tip.text)
            .size(font_size)
            .color(st.text)
            .font(font)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .align_y(Alignment::Center)
    .padding(iced::Padding::from(MENU_PAD as f32));
    styled_box(
        body.into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        0,
        Length::Fill,
        Length::Fill,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `\n`-separated line is fitted and measured: the surface
    /// is as wide as the widest line, one row per line tall, and an
    /// over-budget line is cut on its own.
    #[test]
    fn tooltip_fit_measures_every_line() {
        let font = Font::with_name("Noto Sans");
        let (w, h, fitted) = tooltip_fit(&font, 14, "Title\nSome longer body line", 500);
        assert_eq!(fitted, "Title\nSome longer body line");
        assert_eq!(h, 2 * menu_row_h(14));
        let widest = text_width::widest(&font, 14, &["Some longer body line".to_string()]);
        assert_eq!(w, u32::from(widest));
        // The long line is cut, the short one keeps; two rows still.
        let long = "x".repeat(400);
        let (w2, h2, fitted2) = tooltip_fit(&font, 14, &format!("ok\n{long}"), 60);
        assert_eq!(h2, 2 * menu_row_h(14));
        assert!(w2 <= 60);
        let (first, second) = fitted2.split_once('\n').unwrap();
        assert_eq!(first, "ok");
        assert!(second.ends_with('…'));
    }
    /// A tooltip window sits just off the bar's anchored edge on the
    /// inner side of the screen — a top bar's drops below the surface —
    /// centred on the cursor and clamped inside the output.
    #[test]
    fn tooltip_placement_sits_off_the_bar_and_inside_the_output() {
        // Top bar: below the surface (bar 30 + gap 8), centred on x.
        let (a, m) = tooltip_placement(Anchor::Top, (100, 15), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Left | Anchor::Top);
        assert_eq!((m.left, m.top), (60, 38));
        // Near the left edge the centring clamps to the output edge…
        let (_, m) = tooltip_placement(Anchor::Top, (10, 15), 80, 20, 30, 1920, 1080);
        assert_eq!(m.left, 0);
        // …and near the right edge to its far edge.
        let (_, m) = tooltip_placement(Anchor::Top, (1915, 15), 80, 20, 30, 1920, 1080);
        assert_eq!(m.left, 1920 - 80);
        // Bottom bar: above the surface.
        let (a, m) = tooltip_placement(Anchor::Bottom, (100, 1065), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Left | Anchor::Bottom);
        assert_eq!(m.bottom, 38);
        // Side bars push inward.
        let (a, m) = tooltip_placement(Anchor::Left, (15, 500), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Left | Anchor::Top);
        assert_eq!(m.left, 38);
        let (a, m) = tooltip_placement(Anchor::Right, (1905, 500), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Right | Anchor::Top);
        assert_eq!(m.right, 38);
    }
}
