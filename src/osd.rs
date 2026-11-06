//! The volume/brightness on-screen display: a small centred card that
//! appears when a reading changes (its own wheel, the volume keys, the
//! brightness keys) and goes away after a moment. One surface, created
//! on the first change and closed when it expires — nothing runs while
//! no value moves.

use std::time::{Duration, Instant};

use iced::widget::{container, text, Column, Row, Space};
use iced::{Alignment, Element, Length};
use iced_exwlshell::reexport::{
    KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;

use crate::text_util::panel_font;
use crate::{popup_style, text_width, Command, Message, Panel};

/// How long the card stays after the last change.
const LINGER: Duration = Duration::from_millis(1200);
/// Same width as the notification popup, so a corner/centre position
/// lines the two cards up edge to edge.
/// Card width: the longest label plus a "100%" reading, one gap and the
/// card padding either side — a compact card, not the notification
/// popup's text width.
fn card_width(panel: &Panel, style: &crate::PopupStyle, padding: u32) -> u32 {
    let font = panel_font(&panel.config);
    let label = text_width::widest(
        &font,
        style.font_size,
        &["Volume".to_string(), "Brightness".to_string()],
    );
    let value = text_width::widest(&font, style.font_size, &["100%".to_string()]);
    (u32::from(label) + u32::from(value) + 8 + 2 * padding + 16).clamp(160, 320)
}

/// Card height: the text line, the bar and the gaps, plus the padding.
fn card_height(style: &crate::PopupStyle, padding: u32) -> u32 {
    2 * padding + (style.font_size as f32 * 1.2).ceil() as u32 + 8 + 6
}

pub(crate) struct Osd {
    pub(crate) id: iced::window::Id,
    until: Instant,
    label: String,
    text: String,
    fraction: f32,
}

/// Show (or refresh) the card. The surface is created on the first use
/// and reused while it stays up.
pub(crate) fn show(panel: &mut Panel, label: &str, text: &str, fraction: f32) -> Command<Message> {
    let until = Instant::now() + LINGER;
    if let Some(osd) = panel.osd.as_mut() {
        osd.until = until;
        osd.label = label.to_string();
        osd.text = text.to_string();
        osd.fraction = fraction.clamp(0.0, 1.0);
        return Command::none();
    }
    // Same screen position as the notification popup (roadmap M4): a
    // configured corner/center, or its auto rule (opposite corner of the
    // bar, right side). The OSD is wider, the anchors and margins are the
    // popup's own.
    let (anchor, margin) = crate::popup_notifications::popup_placement(&panel.config);
    let st = popup_style(&panel.config, panel.palette.as_ref());
    // The notification padding can be tiny (4 px); a card with a bar
    // needs a little more air, or the content hugs the border.
    let padding = st.padding.max(10);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(card_width(panel, &st, padding), card_height(&st, padding)),
        layer: Layer::Overlay,
        anchor,
        // No `-1`: the compositor must apply the same reserved strip as
        // the notification popup, so a top/bottom anchor lands on the
        // same screen line instead of over the bar.
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: OutputOption::Active,
        events_transparent: true,
        namespace: Some("piuma-osd".to_string()),
        blur_option: crate::glass_blur(&panel.config),
    });
    panel.osd = Some(Osd {
        id,
        until,
        label: label.to_string(),
        text: text.to_string(),
        fraction: fraction.clamp(0.0, 1.0),
    });
    open
}

/// Close the card once its linger elapsed.
pub(crate) fn sync(panel: &mut Panel) -> Command<Message> {
    if panel.osd.as_ref().is_some_and(|osd| Instant::now() >= osd.until) {
        return close(panel);
    }
    Command::none()
}

pub(crate) fn close(panel: &mut Panel) -> Command<Message> {
    panel
        .osd
        .take()
        .map(|osd| iced::window::close(osd.id))
        .unwrap_or_else(Command::none)
}

pub(crate) fn is_window(panel: &Panel, window: iced::window::Id) -> bool {
    panel.osd.as_ref().is_some_and(|osd| osd.id == window)
}

/// The card, in the notification popup's style (`panel.notifications`):
/// same font, colours, border, radius and padding; the progress bar uses
/// the card's text tone so nothing introduces a second palette.
pub(crate) fn view(panel: &Panel) -> Element<'_, Message> {
    let Some(osd) = panel.osd.as_ref() else {
        return container(Space::new().width(Length::Fill).height(Length::Fill)).into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let filled = ((osd.fraction * 100.0).round() as u16).max(1);
    let empty = 100u16.saturating_sub(filled).max(1);
    let track = Row::new()
        .push(
            container(Space::new())
                .width(Length::FillPortion(filled))
                .height(Length::Fixed(6.0))
                .style(move |_t| container::Style {
                    background: Some(iced::Background::Color(st.text)),
                    border: iced::Border {
                        radius: 3.0.into(),
                        ..Default::default()
                    },
                    ..container::Style::default()
                }),
        )
        .push(
            container(Space::new())
                .width(Length::FillPortion(empty))
                .height(Length::Fixed(6.0)),
        )
        .width(Length::Fill);
    container(
        Column::new()
            .push(
                container(
                    Row::new()
                        .push(
                            text(&osd.label)
                                .size(st.font_size)
                                .font(font)
                                .color(st.text),
                        )
                        .push(Space::new().width(Length::Fixed(8.0)))
                        .push(
                            text(&osd.text)
                                .size(st.font_size)
                                .font(font)
                                .color(st.text),
                        )
                        .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .align_x(Alignment::Center),
            )
            .push(track)
            .spacing(10)
            .width(Length::Fill),
    )
    .padding(st.padding.max(10) as f32)
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Alignment::Center)
    .style(move |_t| container::Style {
        background: Some(iced::Background::Color(st.bg)),
        border: iced::Border {
            width: st.border_width as f32,
            color: st.border,
            radius: (st.border_radius as f32).into(),
        },
        ..container::Style::default()
    })
    .into()
}

