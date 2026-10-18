//! The popup menus: the tray app menu (dbusmenu rows) and the
//! taskbar window menu, plus the row metrics they share.

use crate::dbusmenu;
use crate::text_width;
use crate::{corner_radius, panel_font, popup_style, styled_box, DEFAULT_FONT_SIZE};
use crate::{Message, Panel};
use iced::font::Font;
use iced::widget::container::Style as ContainerStyle;
use iced::widget::{container, text, Column, MouseArea, Space};
use iced::{Alignment, Background, Color, Element, Length};
use iced_exwlshell::reexport::{Anchor, Margin};
/// Padding around the entry list (all four sides).
pub(crate) const MENU_PAD: u32 = 6;

/// Indent per submenu level (rows and the width estimate agree on it).
pub(crate) const MENU_INDENT: u32 = 14;

/// Width bounds: a one-item menu stays grabbable, a long label does
/// not take half the screen (it gets clipped instead).
pub(crate) const MENU_MIN_W: u32 = 120;

pub(crate) const MENU_MAX_W: u32 = 420;

/// Vertical rhythm of one menu row: the text line (~1.2× the font)
/// plus a little air, so centered rows never clip at any font size.
pub(crate) fn menu_row_h(font_size: u32) -> u32 {
    font_size + font_size / 2 + 4
}

/// Surface size for `entries`: the height is exact (one `menu_row_h`
/// per row plus padding — nothing clips, every row stays hit-exact);
/// the width measures the widest label with the same shaper iced
/// renders with (`text_width::widest`), adds the deepest indent and
/// the horizontal margins on both sides — two `MENU_PAD` layers per
/// side (the list container's and each row's own) — and is clamped so
/// a one-item menu is still grabbable and a long label does not take
/// half the screen.
pub(crate) fn menu_size(
    entries: &[dbusmenu::MenuEntry],
    font: &Font,
    font_size: u32,
) -> (u32, u32) {
    let patterns: Vec<String> = entries
        .iter()
        .map(|e| format!("{}{}", dbusmenu::toggle_glyph(e), e.label))
        .collect();
    let text_w = u32::from(text_width::widest(font, font_size, &patterns));
    let max_depth = entries.iter().map(|e| e.depth).max().unwrap_or(0);
    let w = text_w
        .saturating_add(max_depth * MENU_INDENT)
        .saturating_add(4 * MENU_PAD)
        .clamp(MENU_MIN_W, MENU_MAX_W);
    let h = entries.len() as u32 * menu_row_h(font_size) + 2 * MENU_PAD;
    (w, h)
}

/// Cut every label to its row budget inside the finished surface width `w`
/// (the deepest indent narrows the text area of its own row, and the
/// budget keeps both `MENU_PAD` layers per side — the list container's
/// and the row's own — so a cut label ends with the same margin as a
/// short one): the toggle glyph measures with the label, so it is fitted
/// too and then stripped back off. Runs once when the menu loads —
/// [`tray_menu_view`] renders the result on a single line, so a too-long
/// label ends with an ellipsis instead of spilling over the next row.
pub(crate) fn fit_menu_labels(
    entries: &mut [dbusmenu::MenuEntry],
    w: u32,
    font: &Font,
    font_size: u32,
) {
    let jobs: Vec<(String, u32)> = entries
        .iter()
        .map(|e| {
            (
                format!("{}{}", dbusmenu::toggle_glyph(e), e.label),
                w.saturating_sub(4 * MENU_PAD + e.depth * MENU_INDENT),
            )
        })
        .collect();
    for (entry, fitted) in entries
        .iter_mut()
        .zip(text_width::fit(font, font_size, &jobs))
    {
        let glyph = dbusmenu::toggle_glyph(entry);
        entry.label = fitted.strip_prefix(glyph).unwrap_or_default().to_string();
    }
}

/// Anchor + edge margins for the menu at the output-relative point
/// (x, y): it opens right/down from the cursor — the natural quadrant
/// under a top/left bar — and flips to the opposite side when the
/// surface would leave the output, with the flipped edge **clamped to
/// the output**: a menu that doesn't fit on that side is cut at the
/// output edge instead of being pinned at the cursor and leaving the
/// screen (the 116-row Cachy-Update menu used to open with its base at
/// the top bar and ~2300 px of it above the screen). Cursor-driven
/// callers pin the surface to the bar's own output via
/// [`popup_output_option`], since `OutputOption::LastOutput` lands on
/// another output once a second monitor joins.
pub(crate) fn menu_placement(x: i32, y: i32, w: u32, h: u32, sw: i32, sh: i32) -> (Anchor, Margin) {
    let (w, h) = (w as i32, h as i32);
    // Fits: anchored at the point. Overflows: the far edge sticks to the
    // output and the surface ends at the point — unless that would push
    // the near edge off-screen, in which case the surface sits flush
    // with the output edge (cut, not lost).
    let (anchor_h, left, right) = if x + w <= sw {
        (Anchor::Left, x, 0)
    } else {
        (Anchor::Right, 0, (sw - x).clamp(0, (sw - w).max(0)))
    };
    let (anchor_v, top, bottom) = if y + h <= sh {
        (Anchor::Top, y, 0)
    } else {
        (Anchor::Bottom, 0, (sh - y).clamp(0, (sh - h).max(0)))
    };
    (
        anchor_h | anchor_v,
        Margin {
            top,
            right,
            bottom,
            left,
        },
    )
}

/// The tray menu surface: one row per entry inside the styled box
/// (shared with the sensors and the notification cards), the whole list
/// wrapped in a MouseArea whose exit dismisses the menu (the approved
/// auto-close). Clickable rows carry their own MouseArea — press reports
/// the entry, enter/leave drive the hover style.
pub(crate) fn tray_menu_view(panel: &Panel) -> Element<'_, Message> {
    let Some((_, menu)) = panel.tray_menu.as_ref() else {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let row_h = menu_row_h(font_size) as f32;
    let dim = Color::from_rgba(st.text.r, st.text.g, st.text.b, 0.45);
    // The configured hover colors equal the base ones — a color swap on
    // hover would be invisible, so the feedback is an underline bar under
    // the label (plus a hand pointer on the row itself).
    let underline_hover = st.text_hover == st.text && st.bg_hover == st.bg;
    let mut list: Column<'_, Message> = Column::new();
    for entry in &menu.entries {
        let padding = iced::Padding {
            top: 0.0,
            right: MENU_PAD as f32,
            bottom: 0.0,
            left: (MENU_PAD + entry.depth * MENU_INDENT) as f32,
        };
        let row: Element<'_, Message> = match entry.kind {
            dbusmenu::EntryKind::Separator => {
                container(container(Space::new().width(Length::Fill).height(1)).style(
                    move |_theme| ContainerStyle {
                        background: Some(Background::Color(st.border)),
                        ..ContainerStyle::default()
                    },
                ))
                .width(Length::Fill)
                .height(Length::Fixed(row_h))
                .align_y(Alignment::Center)
                .padding(padding)
                .into()
            }
            // A submenu header is a section label, never a target.
            dbusmenu::EntryKind::Header => container(
                text(&entry.label)
                    .size(font_size)
                    .color(st.text)
                    .font(font)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .width(Length::Fill)
            .height(Length::Fixed(row_h))
            .align_y(Alignment::Center)
            .padding(padding)
            .into(),
            dbusmenu::EntryKind::Item => {
                let hovered = entry.enabled && panel.tray_menu_hover == Some(entry.id);
                let fg = if !entry.enabled {
                    dim
                } else if hovered {
                    st.text_hover
                } else {
                    st.text
                };
                let label = text(format!("{}{}", dbusmenu::toggle_glyph(entry), entry.label))
                    .size(font_size)
                    .color(fg)
                    .font(font)
                    .wrapping(iced::widget::text::Wrapping::None);
                // In underline mode the column shrinks to the label and the
                // bar fills it — the text's exact width without measuring,
                // and the bar is laid out even while transparent (only its
                // color changes on hover), so nothing jumps.
                let inner: Element<'_, Message> = if underline_hover {
                    Column::new()
                        .push(label.width(Length::Shrink))
                        .push(
                            container(Space::new().width(Length::Fill).height(Length::Fixed(2.0)))
                                .style(move |_theme| ContainerStyle {
                                    background: hovered.then(|| Background::Color(fg)),
                                    ..ContainerStyle::default()
                                }),
                        )
                        .spacing(1)
                        .into()
                } else {
                    label.width(Length::Fill).into()
                };
                let body = container(inner)
                    .width(Length::Fill)
                    .height(Length::Fixed(row_h))
                    .align_y(Alignment::Center)
                    .padding(padding);
                let styled = if hovered {
                    body.style(move |_theme| ContainerStyle {
                        background: Some(Background::Color(st.bg_hover)),
                        ..ContainerStyle::default()
                    })
                } else {
                    body.style(move |_theme| ContainerStyle::default())
                };
                if entry.enabled {
                    MouseArea::new(styled)
                        .on_press(Message::TrayMenuClicked(entry.id))
                        .on_enter(Message::TrayMenuEnter(entry.id))
                        .on_exit(Message::TrayMenuLeave(entry.id))
                        .interaction(iced::mouse::Interaction::Pointer)
                        .into()
                } else {
                    styled.into()
                }
            }
        };
        list = list.push(row);
    }
    let surface = styled_box(
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(MENU_PAD as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The list carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    MouseArea::new(surface)
        .on_exit(Message::TrayMenuClosed)
        .into()
}

/// Surface size for the taskbar menu: one `menu_row_h` row per
/// workspace (1..9), width from the widest label — exact, so every
/// row stays hit-exact.
pub(crate) fn taskbar_menu_size(font: &Font, font_size: u32) -> (u32, u32) {
    let labels: Vec<String> = (1..=9).map(|n| n.to_string()).collect();
    let text_w = u32::from(text_width::widest(font, font_size, &labels));
    let w = text_w.saturating_add(2 * MENU_PAD).max(48);
    let h = 9u32
        .saturating_mul(menu_row_h(font_size))
        .saturating_add(2 * MENU_PAD);
    (w, h)
}

/// The taskbar's move menu: workspace rows 1..9, the hovered one
/// highlighted; a press moves the menu's window there silently and
/// closes the menu, and leaving the surface closes without acting
/// (same exit rule as the tray menu).
pub(crate) fn taskbar_menu_view(panel: &Panel) -> Element<'_, Message> {
    if panel.taskbar_menu.is_none() {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let row_h = menu_row_h(font_size);
    // A hover row rounds at most to half its own height.
    let radius = (st.border_radius as f32).min(row_h as f32 / 2.0);
    let mut list: Column<'_, Message> = Column::new();
    for workspace in 1..=9u32 {
        let hovered = panel.taskbar_menu_hover == Some(workspace);
        // The hover fill follows the menu's own corners: the top row
        // rounds up, the last rounds down, the middle rows meet their
        // neighbours square (`corner_radius`: touching sides are flat).
        let touch = if workspace == 1 {
            [false, false, true, false]
        } else if workspace == 9 {
            [true, false, false, false]
        } else {
            [true, true, true, true]
        };
        let cell = container(
            // Centred in the cell (width Fill only on the cell): the
            // label keeps its own width and sits mid-menu instead of
            // glued to the left edge.
            text(workspace.to_string())
                .size(font_size)
                .color(st.text)
                .font(font),
        )
        .width(Length::Fill)
        .height(Length::Fixed(row_h as f32))
        .center_x(Length::Fill)
        .align_y(Alignment::Center)
        .style(move |_theme| ContainerStyle {
            background: hovered.then(|| Background::Color(st.bg_hover)),
            border: iced::Border {
                radius: corner_radius(touch, radius),
                ..Default::default()
            },
            ..ContainerStyle::default()
        });
        list = list.push(
            MouseArea::new(cell)
                .on_press(Message::TaskbarMenuWs(workspace))
                .on_enter(Message::TaskbarMenuHover(Some(workspace)))
                // The exit names its row: a stale one (arriving after
                // the next row's enter) must not clear the new hover.
                .on_exit(Message::TaskbarMenuLeft(workspace))
                .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    let surface = styled_box(
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(MENU_PAD as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The list carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    MouseArea::new(surface)
        .on_exit(Message::TaskbarMenuClosed)
        .into()
}
