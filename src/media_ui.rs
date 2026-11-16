//! The media sensor's transport row: previous, play/pause and next as
//! small buttons, then the truncated track text. A control the player
//! cannot perform is drawn at half alpha (its click is a no-op in
//! `mpris`, which checks the same capability).

use iced::widget::{button, text, Button, Row};
use iced::{Background, Border, Color, Element, Font};

use crate::mpris::{self, MediaState};
use crate::Message;

/// The row the sensor draws: `[⏮][⏵/⏸][⏭]  title — artist`.
pub(crate) fn row<'a>(
    media: &MediaState,
    font_size: u32,
    font: Font,
    color: Color,
) -> Element<'a, Message> {
    let play = if media.status == "Playing" {
        "⏸"
    } else {
        "⏵"
    };
    let size = font_size.saturating_add(1);
    Row::new()
        .spacing(2)
        .align_y(iced::Alignment::Center)
        .push(transport(
            "⏮",
            Message::MediaPrevious,
            media.can_previous,
            font,
            size,
            color,
        ))
        .push(transport(
            play,
            Message::MediaPlayPause,
            media.can_control,
            font,
            size,
            color,
        ))
        .push(transport(
            "⏭",
            Message::MediaNext,
            media.can_next,
            font,
            size,
            color,
        ))
        .push(
            text(mpris::media_label(media))
                .size(font_size)
                .font(font)
                .color(color),
        )
        .into()
}

/// One transport button: a faint surface always (so it reads as a
/// control, not a glyph), stronger while hovered/pressed; the glyph and
/// the surface dim to half alpha when the player disables the action.
fn transport<'a>(
    glyph: &'a str,
    message: Message,
    enabled: bool,
    font: Font,
    size: u32,
    color: Color,
) -> Button<'a, Message> {
    let alpha = if enabled { 1.0 } else { 0.5 };
    button(
        text(glyph)
            .size(size)
            .font(font)
            .color(Color { a: alpha, ..color }),
    )
    .on_press(message)
    .padding([1, 4])
    .style(move |_theme: &iced::Theme, status: button::Status| button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => Color { a: 0.22, ..color },
            button::Status::Disabled => Color { a: 0.05, ..color },
            _ => Color { a: 0.10, ..color },
        })),
        text_color: Color { a: alpha, ..color },
        border: Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..button::Style::default()
    })
}
