//! The notification popup and its list: the stacked card window that
//! rides the bar's anchor (size, placement, dismiss) and the toggleable
//! list the notification sensor opens (rows, scroll, view).

use crate::menus::{menu_arrow_h, menu_row_h, MENU_PAD};
use crate::notifications;
use crate::text_width;
use crate::{
    config, panel_anchor, panel_font, popup_card_h, popup_output_option, popup_style, styled_box,
    Message, NotifList, Panel, DEFAULT_FONT_SIZE,
};
use iced::font::Font;
use iced::widget::image::Image;
use iced::widget::{container, scrollable, stack, text, Column, MouseArea, Row, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Task as Command};
use iced::widget::scrollable::{AutoScroll, Rail, Scroller};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, Margin, NewLayerShellSettings,
};
use iced_exwlshell::settings::LayerSize;

pub(crate) const POPUP_MAX_CARDS: usize = 5;

pub(crate) const POPUP_WIDTH: u32 = 360;

pub(crate) const POPUP_CARD_GAP: u32 = 8;

/// Gap between the screen edges and the popup corner.
pub(crate) const POPUP_MARGIN: i32 = 8;

/// The notification list popup (the sensor's click): entries in the
/// snapshot, rows shown at once (the wheel scrolls the rest) and the
/// width cap for the measured rows.
pub(crate) const NOTIF_LIST_MAX: usize = 30;

pub(crate) const NOTIF_LIST_ROWS: usize = 8;

pub(crate) const NOTIF_LIST_W_MAX: u32 = 460;

pub(crate) fn sync_popup(panel: &mut Panel, reposition: bool) -> Option<Command<Message>> {
    let enabled = panel.config.panel.notifications.is_some();
    let cards = if enabled {
        // Count only, and reap the icon handles that lost their card:
        // the old path cloned every (possibly image-bearing) card each
        // tick just to read `.len()`.
        notifications::prune_icon_handles();
        notifications::active_cards_len(POPUP_MAX_CARDS)
    } else {
        0
    };
    // A changed set invalidates the hover id: expiry reaches this path
    // without passing through popup_dismiss.
    if cards != panel.popup_cards {
        panel.popup_hover = None;
    }
    match (panel.popup, cards) {
        (None, 0) => None,
        (Some(id), 0) => {
            panel.popup = None;
            panel.popup_cards = 0;
            panel.popup_hover = None;
            Some(iced::window::close(id))
        }
        (None, n) => {
            let (anchor, margin) = popup_placement(&panel.config);
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: popup_size(panel, n),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: None,
                margin: Some(margin),
                keyboard_interactivity: KeyboardInteractivity::None,
                // The focused output, like every other popup (margins
                // are pure offsets — safe on any output).
                output_option: popup_output_option(),
                namespace: Some("piuma-notifications".to_string()),
                ..Default::default()
            });
            panel.popup = Some(id);
            panel.popup_cards = n;
            Some(open)
        }
        (Some(id), n) if reposition || n != panel.popup_cards => {
            panel.popup_cards = n;
            let (anchor, margin) = popup_placement(&panel.config);
            // The margin rides along: position and margins are config keys
            // (roadmap M4) and a reload can change them at any time.
            Some(Command::batch([
                Command::done(Message::LayoutChange {
                    id,
                    anchor,
                    size: popup_size(panel, n),
                }),
                Command::done(Message::MarginChange { id, margin }),
            ]))
        }
        _ => None,
    }
}

/// Surface size for a stack of `cards`: exact, so there is no dead
/// (click-blocking) surface below the last card. The card height comes
/// from the configured popup style (see `popup_card_h`).
pub(crate) fn popup_size(panel: &Panel, cards: usize) -> LayerSize {
    let n = cards as u32;
    let h = popup_card_h(&popup_style(&panel.config, panel.palette.as_ref()));
    LayerSize::px(POPUP_WIDTH, n * h + n.saturating_sub(1) * POPUP_CARD_GAP)
}

/// Right-hand corner opposite to the bar's edge: a bottom bar leaves
/// the top corner, a top bar the bottom one, side bars the top corner.
pub(crate) fn popup_anchor(config: &config::Config) -> Anchor {
    let bar = panel_anchor(config);
    if bar == Anchor::Top {
        Anchor::Bottom | Anchor::Right
    } else {
        Anchor::Top | Anchor::Right
    }
}

/// Anchor + screen-edge margins for the notification popup (roadmap M4),
/// from `panel.notifications.popup_position` / `popup_margin_h` /
/// `popup_margin_v`. `auto` (absent/unknown key) reproduces the historic
/// layout: opposite the bar on the right, 8px from the top/right edges.
pub(crate) fn popup_placement(config: &config::Config) -> (Anchor, Margin) {
    let notify = config.panel.notifications.as_ref();
    let margin_h = notify
        .and_then(|n| n.popup_margin_h)
        .unwrap_or(POPUP_MARGIN);
    let margin_v = notify
        .and_then(|n| n.popup_margin_v)
        .unwrap_or(POPUP_MARGIN);
    let position = notify
        .and_then(|n| n.popup_position.as_deref())
        .unwrap_or("auto");
    // Margins on the two sides of a corner position (the others stay 0).
    let corner = |top: bool, left: bool| Margin {
        top: if top { margin_v } else { 0 },
        bottom: if top { 0 } else { margin_v },
        left: if left { margin_h } else { 0 },
        right: if left { 0 } else { margin_h },
    };
    // Centered positions anchor both horizontal edges (the compositor
    // centres the fixed-width surface); equal margins keep it centred.
    let center = |top: bool| Margin {
        top: if top { margin_v } else { 0 },
        bottom: if top { 0 } else { margin_v },
        left: margin_h,
        right: margin_h,
    };
    match position {
        "top-left" => (Anchor::Top | Anchor::Left, corner(true, true)),
        "top-center" => (Anchor::Top | Anchor::Left | Anchor::Right, center(true)),
        "top-right" => (Anchor::Top | Anchor::Right, corner(true, false)),
        "bottom-left" => (Anchor::Bottom | Anchor::Left, corner(false, true)),
        "bottom-center" => (Anchor::Bottom | Anchor::Left | Anchor::Right, center(false)),
        "bottom-right" => (Anchor::Bottom | Anchor::Right, corner(false, false)),
        // `auto` and anything unknown keep the historic layout.
        _ => (
            popup_anchor(config),
            Margin {
                top: POPUP_MARGIN,
                right: POPUP_MARGIN,
                bottom: 0,
                left: 0,
            },
        ),
    }
}

/// Dismiss a popup card: close the entry with the user's reason (and
/// its `NotificationClosed` signal), refresh the sensor text at once
/// and resize the stack.
pub(crate) fn popup_dismiss(panel: &mut Panel, id: u32) -> Command<Message> {
    if notifications::dismiss(id) {
        notifications::spawn_closed_signal(id);
        refresh_notification_sensor(panel);
    }
    sync_popup(panel, false).unwrap_or_else(Command::none)
}

/// Rewrite every `notifications` sensor's value in place after a UI
/// change (the next tick confirms it), like the scroll/click arms do.
pub(crate) fn refresh_notification_sensor(panel: &mut Panel) {
    let value = notifications::display_value();
    for (si, sec) in panel.config.panel.sections.iter().enumerate() {
        for (sni, s) in sec.sensors.iter().enumerate() {
            if matches!(s.source, config::SensorSource::Notifications {}) {
                if let Some(sv) = panel
                    .sections
                    .get_mut(si)
                    .and_then(|sec| sec.sensors.get_mut(sni))
                {
                    sv.value = value.clone();
                }
            }
        }
    }
}

/// The popup stack: one card per active notification (newest on top).
/// The content block is one MouseArea (click → `PopupCardClicked` =
/// default action + dismiss) and the ✕ a separate MouseArea inside the
/// same card, so the two targets never overlap. Both targets also feed
/// the hover id, and the card's look comes from the configured style
/// through the shared `styled_box` helper (the sensors' border machinery).
pub(crate) fn popup_view(panel: &Panel) -> Element<'_, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let card_h = popup_card_h(&st);
    // Font audit (roadmap M2): every popup text takes the configured family;
    // the summary renders bold on top of it, body and ✕ as configured.
    let font = panel_font(&panel.config);
    let bold = Font {
        weight: iced::font::Weight::Bold,
        ..font
    };

    let mut stack: Column<'_, Message> = Column::new().spacing(POPUP_CARD_GAP as f32);
    for card in notifications::active_cards(POPUP_MAX_CARDS) {
        // Hovered cards swap text/bg/border for their hover counterparts.
        let (fg, bg, border_c) = if panel.popup_hover == Some(card.id) {
            (st.text_hover, st.bg_hover, st.border_hover)
        } else {
            (st.text, st.bg, st.border)
        };
        // Icon: image-data wins (handle cached per notification — one
        // decode, not one per frame), a real file path follows; anything
        // else (e.g. a themed icon name) renders without an icon.
        let icon: Option<Image> = match &card.icon {
            Some(notifications::CardIcon::Image) => {
                notifications::icon_handle(card.id).map(|handle| {
                    Image::new(handle)
                        .width(32)
                        .height(32)
                        .content_fit(iced::ContentFit::Contain)
                })
            }
            Some(notifications::CardIcon::Path(p)) => std::path::Path::new(p).exists().then(|| {
                Image::new(iced::widget::image::Handle::from_path(p.clone()))
                    .width(32)
                    .height(32)
                    .content_fit(iced::ContentFit::Contain)
            }),
            None => None,
        };
        let mut lines: Column<'_, Message> = Column::new().spacing(2).push(
            text(card.summary)
                .size(st.font_size + 3)
                .font(bold)
                .color(fg)
                .width(Length::Fill),
        );
        if !card.body.is_empty() {
            lines = lines.push(
                text(card.body)
                    .size(st.font_size)
                    .color(fg)
                    .font(font)
                    .width(Length::Fill),
            );
        }
        let mut content = Row::new().spacing(8).align_y(Alignment::Center);
        if let Some(icon) = icon {
            content = content.push(icon);
        }
        // The text area scrolls inside the card: an overlong body (or
        // one with line breaks) reads through the wheel instead of
        // being clipped, while the card — and the popup surface —
        // keeps the style's height. The budget is what `popup_card_h`
        // leaves between the two paddings (8 lines of font). `lines`
        // sits in a `padding`-inset container so text and the scrollbar
        // don't hug the card edges. The scrollbar follows the bar's own
        // colours (not iced's default) and appears only once the content
        // overflows (the vertical rail is disabled until it does).
        let lines = container(lines)
            .padding(iced::Padding {
                top: 0.0,
                right: st.padding.min(u16::MAX as u32) as f32,
                bottom: 0.0,
                left: 0.0,
            })
            .width(Length::Fill)
            .align_y(Alignment::Center);
        let scroll_id = iced::widget::Id::from(format!("notif-card-{}", card.id));
        let text_area = scrollable(lines.width(Length::Fill))
            .id(scroll_id)
            .on_scroll(move |viewport| {
                let offset = viewport.absolute_offset().y;
                let max = (viewport.content_bounds().height - viewport.bounds().height).max(0.0);
                Message::NotifCardScrolled(card.id, offset, max)
            })
            .width(Length::Fill)
            .height(Length::Fixed((7 * st.font_size) as f32))
            .style(move |_, _| scrollable::Style {
                container: container::Style::default(),
                // No rail: the two centre arrows replace the scrollbar.
                vertical_rail: Rail {
                    background: None,
                    border: Border::default(),
                    scroller: Scroller {
                        background: Background::Color(Color::TRANSPARENT),
                        border: Border::default(),
                    },
                },
                horizontal_rail: Rail {
                    background: None,
                    border: Border::default(),
                    scroller: Scroller {
                        background: Background::Color(Color::TRANSPARENT),
                        border: Border::default(),
                    },
                },
                gap: None,
                auto_scroll: AutoScroll {
                    background: Background::Color(Color::TRANSPARENT),
                    border: Border::default(),
                    shadow: iced::Shadow::default(),
                    icon: Color::TRANSPARENT,
                },
            });
        let (offset, max) = panel.notif_scroll.get(&card.id).copied().unwrap_or((0.0, 0.0));
        let arrow_h = (st.font_size + 2) as f32;
        let arrow = |up: bool, visible: bool| -> Element<'_, Message> {
            if !visible {
                return container(text(""))
                    .width(Length::Fill)
                    .height(Length::Fixed(arrow_h))
                    .into();
            }
            MouseArea::new(
                container(
                    text(if up { "▲" } else { "▼" })
                        .size(st.font_size.saturating_sub(2))
                        .color(fg)
                        .font(font.clone()),
                )
                .width(Length::Fill)
                .height(Length::Fixed(arrow_h))
                .center_x(Length::Fill)
                .align_y(Alignment::Center)
                .style(move |_| container::Style {
                    background: Some(Background::Color(bg)),
                    ..container::Style::default()
                }),
            )
            .on_press(Message::NotifCardArrow(card.id, up))
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
        };
        let nav = Column::new()
            .push(arrow(true, offset > 0.5))
            .push(Space::new().height(Length::Fill))
            .push(arrow(false, max > 0.5 && offset < max - 0.5))
            .width(Length::Fill)
            .height(Length::Fill);
        let text_area = stack![
            MouseArea::new(text_area)
                .on_press(Message::PopupCardClicked(card.id))
                .on_enter(Message::PopupCardEnter(card.id))
                .on_exit(Message::PopupCardLeave),
            nav
        ]
        .width(Length::Fill)
        .height(Length::Fixed((7 * st.font_size) as f32));
        let content = content.push(text_area).width(Length::Fill);
        let card_row: Row<'_, Message> = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .push(content)
            // The ✕ gets 8px of padding: a small glyph alone is too small
            // a target, and its MouseArea must not overlap the card's.
            .push(
                MouseArea::new(
                    container(text("✕").size(st.font_size + 2).color(fg).font(font)).padding(8),
                )
                .on_press(Message::PopupCardDismissed(card.id))
                .on_enter(Message::PopupCardEnter(card.id))
                .on_exit(Message::PopupCardLeave),
            );
        let styled = styled_box(
            card_row.into(),
            Some(bg),
            Some(border_c),
            st.border_width.min(u16::MAX as u32) as u16,
            st.border_radius.min(u16::MAX as u32) as u16,
            &st.border_type,
            [false, false, false, false],
            iced::Padding::from(st.padding.min(u16::MAX as u32) as u16),
            Length::Fill,
            Length::Fixed(card_h as f32),
        );
        // The clip keeps an overlong body inside the card box; the outer
        // size is the same `card_h` the surface math uses.
        stack = stack.push(
            container(styled)
                .width(Length::Fill)
                .height(Length::Fixed(card_h as f32))
                .clip(true),
        );
    }
    container(stack)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Vertical rhythm of one list row: a double-height row (summary +
/// body, spaced) — `menu_row_h`'s maths for two stacked lines.
pub(crate) fn notif_list_row_h(font_size: u32) -> u32 {
    menu_row_h(font_size) * 2 + 4
}

/// The wheel's effect on the scroll offset: whole rows towards the
/// oldest, clamped so the last page lands exactly on the bottom rows.
pub(crate) fn scroll_rows(offset: u32, delta: i32, len: usize, visible: usize) -> u32 {
    let max = len.saturating_sub(visible);
    (offset as i32 + delta).clamp(0, max as i32) as u32
}

/// Surface size for `rows`: exact height (one `notif_list_row_h` per
/// shown row plus padding — nothing clips, every row stays hit-exact)
/// and the width from the widest truncated string, capped so a long
/// body does not take half the screen.
pub(crate) fn notif_list_size(
    rows: &[notifications::NotifRow],
    font: &Font,
    font_size: u32,
) -> (u32, u32) {
    let patterns: Vec<String> = rows
        .iter()
        .flat_map(|r| [r.summary.clone(), r.body.clone()])
        .collect();
    let text_w = u32::from(text_width::widest(font, font_size, &patterns));
    let w = text_w
        .saturating_add(2 * MENU_PAD)
        .min(NOTIF_LIST_W_MAX.saturating_add(2 * MENU_PAD));
    let shown = rows.len().min(NOTIF_LIST_ROWS) as u32;
    let arrows_h = if rows.len() > NOTIF_LIST_ROWS {
        2 * menu_arrow_h(font_size)
    } else {
        0
    };
    let h = shown
        .saturating_mul(notif_list_row_h(font_size))
        .saturating_add(arrows_h)
        .saturating_add(2 * MENU_PAD);
    (w, h)
}

/// Toggle the notification history list (the notifications sensor's
/// click): closes it when open; opens it at the notification cards'
/// corner with the newest entries — nothing to reread → nothing to
/// open.
pub(crate) fn toggle_notif_list(panel: &mut Panel) -> Command<Message> {
    if let Some((id, _)) = panel.notif_list.take() {
        return iced::window::close(id);
    }
    let rows = notifications::history_rows(NOTIF_LIST_MAX);
    if rows.is_empty() {
        return Command::none();
    }
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let (w, h) = notif_list_size(&rows, &font, font_size);
    // The cards' own corner: the list belongs to the same notifications.
    let (anchor, margin) = popup_placement(&panel.config);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(w, h),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: popup_output_option(),
        namespace: Some("piuma-notif-list".to_string()),
        ..Default::default()
    });
    panel.notif_list = Some((id, NotifList { offset: 0 }));
    open
}

/// The history list: newest on top, the wheel scrolls by whole rows, a
/// row invokes its notification's spec `default` action and closes the
/// popup (the sensor's old click). Closed entries render dimmed; the
/// rows follow the notification popup style (palette included).
pub(crate) fn notif_list_view(panel: &Panel) -> Element<'_, Message> {
    let Some((_, state)) = panel.notif_list.as_ref() else {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let row_h = notif_list_row_h(font_size) as f32;
    let dim = Color::from_rgba(st.text.r, st.text.g, st.text.b, 0.45);
    let dim_closed = Color::from_rgba(st.text.r, st.text.g, st.text.b, 0.30);
    // Bound so the rows outlive the loop (the elements borrow them).
    let rows = notifications::history_rows(NOTIF_LIST_MAX);
    let scrollable = rows.len() > NOTIF_LIST_ROWS;
    let arrow_h = menu_arrow_h(font_size) as f32;
    let arrow = |up: bool, visible: bool| -> Element<'_, Message> {
        if !visible {
            return container(text(""))
                .width(Length::Fill)
                .height(Length::Fixed(arrow_h))
                .into();
        }
        MouseArea::new(
            container(
                text(if up { "▲" } else { "▼" })
                    .size(font_size.saturating_sub(2))
                    .color(st.text)
                    .font(font),
            )
            .width(Length::Fill)
            .height(Length::Fixed(arrow_h))
            .center_x(Length::Fill)
            .align_y(Alignment::Center),
        )
        .on_press(Message::NotifListArrow(up))
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
    };
    let mut list: Column<'_, Message> = Column::new().spacing(2.0);
    if scrollable {
        list = list.push(arrow(true, state.offset > 0));
    }
    for row in rows
        .iter()
        .skip(state.offset as usize)
        .take(NOTIF_LIST_ROWS)
    {
        let (summary_fg, body_fg) = if row.closed {
            (dim_closed, dim_closed)
        } else {
            (st.text, dim)
        };
        let inner = Column::new()
            .push(
                text(row.summary.clone())
                    .size(font_size)
                    .color(summary_fg)
                    .font(font),
            )
            .push(
                text(row.body.clone())
                    .size(font_size.saturating_sub(2))
                    .color(body_fg)
                    .font(font),
            )
            .spacing(2);
        let styled = container(inner)
            .width(Length::Fill)
            .height(Length::Fixed(row_h))
            .align_y(Alignment::Center)
            .padding(iced::Padding::from(MENU_PAD as f32));
        list = list.push(
            MouseArea::new(styled)
                .on_press(Message::NotifListClicked(row.id))
                .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    if scrollable {
        list = list.push(arrow(
            false,
            state.offset as usize + NOTIF_LIST_ROWS < rows.len(),
        ));
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
        iced::Padding::ZERO,
        Length::Fill,
        Length::Fill,
    );
    // Only non-zero pixel deltas act (one adjustment per notch: the
    // line form always accompanies the pixel one — see the sensors).
    MouseArea::new(surface)
        .on_scroll(move |delta| match delta {
            iced::mouse::ScrollDelta::Pixels { y, .. } if y != 0.0 => Message::NotifListScrolled(y),
            _ => Message::NotifListScrolled(0.0),
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `auto` (no `popup_position`) keeps the historic layout exactly: the
    /// corner opposite the bar on the right, 8px from the top/right edges.
    #[test]
    fn popup_placement_auto_keeps_the_original_layout() {
        let cfg = crate::config::default_config(); // horizontal bar, top
        let (anchor, margin) = popup_placement(&cfg);
        assert_eq!(anchor, Anchor::Bottom | Anchor::Right);
        assert_eq!(
            (margin.top, margin.right, margin.bottom, margin.left),
            (8, 8, 0, 0)
        );
    }
    /// An explicit position anchors the popup on that corner and only the
    /// two relevant margins carry the configured values (roadmap M4).
    #[test]
    fn popup_placement_honours_explicit_position_and_margins() {
        let cfg: crate::config::Config = serde_json::from_str(
            r##"{
                "panel": {
                    "anchor": "top",
                    "section_layout": "horizontal",
                    "notifications": {
                        "popup_position": "bottom-left",
                        "popup_margin_h": 20,
                        "popup_margin_v": 4
                    },
                    "sections": []
                }
            }"##,
        )
        .expect("config must parse");
        let (anchor, margin) = popup_placement(&cfg);
        assert_eq!(anchor, Anchor::Bottom | Anchor::Left);
        assert_eq!(
            (margin.top, margin.right, margin.bottom, margin.left),
            (0, 0, 4, 20)
        );
    }
    /// An unknown position key falls back to `auto` (bar-opposite layout).
    #[test]
    fn popup_placement_falls_back_to_auto_on_unknown_keys() {
        let cfg: crate::config::Config = serde_json::from_str(
            r##"{
                "panel": {
                    "anchor": "top",
                    "section_layout": "horizontal",
                    "notifications": { "popup_position": "diagonal" },
                    "sections": []
                }
            }"##,
        )
        .expect("config must parse");
        let (anchor, _) = popup_placement(&cfg);
        assert_eq!(anchor, Anchor::Bottom | Anchor::Right);
    }
    /// The list popup's wheel: whole rows towards the oldest, clamped
    /// at both ends (the last page lands on the bottom rows) and a
    /// history shorter than a page never scrolls.
    #[test]
    fn notif_list_scroll_clamps_by_whole_rows() {
        assert_eq!(scroll_rows(0, 3, 20, 8), 3);
        assert_eq!(scroll_rows(3, -5, 20, 8), 0, "up clamps at the newest");
        assert_eq!(
            scroll_rows(10, 4, 20, 8),
            12,
            "down clamps at the last page"
        );
        assert_eq!(scroll_rows(0, 2, 5, 8), 0, "a short history never scrolls");
    }
    /// The list popup's surface: exact height for the shown rows (the
    /// cap at `NOTIF_LIST_ROWS`) and the measured width inside the cap.
    #[test]
    fn notif_list_size_is_exact_and_capped() {
        let font = Font::with_name("Noto Sans");
        let row = |i: usize| notifications::NotifRow {
            id: i as u32,
            summary: format!("summary {i}"),
            body: "a body line".to_string(),
            closed: false,
        };
        let rows: Vec<_> = (0..10)
            .map(|i| notifications::NotifRow {
                id: i as u32,
                summary: format!("summary {i} with a tail long enough to widen the box"),
                body: "a body line".to_string(),
                closed: false,
            })
            .collect();
        let (w, h) = notif_list_size(&rows, &font, 14);
        assert_eq!(
            h,
            8 * notif_list_row_h(14) + 2 * menu_arrow_h(14) + 2 * MENU_PAD,
            "only NOTIF_LIST_ROWS rows are shown, plus both arrow bands"
        );
        assert!(
            w <= NOTIF_LIST_W_MAX + 2 * MENU_PAD,
            "the measured width stays capped, got {w}"
        );
        let (w_short, h_short) = notif_list_size(&[row(0)], &font, 14);
        assert_eq!(h_short, notif_list_row_h(14) + 2 * MENU_PAD);
        assert!(w_short < w, "fewer rows measure narrower");
    }
}
