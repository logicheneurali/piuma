//! The graphical volume meter: a fixed-width bar plus the hover-only
//! percentage. The iced slider already owns click, drag and release
//! semantics, so this module only maps the sensor text to a level and
//! paints the meter in the sensor's own colours.

use crate::{sensors, Message};
use iced::widget::slider::{self, Handle, HandleShape, Rail};
use iced::widget::{container, text};
use iced::{Alignment, Background, Border, Color, Element, Font, Length};

/// Bar length: the compact meter proposed for the bar. Fixed so the
/// sensor box never jumps when the percentage appears on hover.
const BAR_W: f32 = 64.0;

/// The slider's hit/layout height; the rail itself stays thin.
const BAR_H: f32 = 16.0;

/// Width of the percentage while it is shown: fixed so the sensor does
/// not jitter between `9%`, `50%` and `100%` during a drag.
const LABEL_W: f32 = 30.0;

/// Current level from the sensor text (`50`, `50%`, `N/A`).
pub(crate) fn level(value: &str) -> f32 {
    sensors::leading_number(value)
        .unwrap_or(0.0)
        .clamp(0.0, 100.0) as f32
}

/// The percentage shown on hover; an unparsable value stays honest.
fn percent_text(value: &str) -> String {
    match sensors::leading_number(value) {
        Some(percent) => format!("{percent:.0}%"),
        None => "N/A".to_string(),
    }
}

/// The interactive bar. Drag and click are provided by iced's slider;
/// the backend write waits for `on_release` (see `VolumeCommitted`).
pub(crate) fn bar<'a>(
    section: usize,
    sensor: usize,
    value: &str,
    color: Color,
) -> Element<'a, Message> {
    let fill = color;
    let track = Color { a: 0.18, ..color };
    slider::Slider::new(0.0..=100.0, level(value), move |percent| {
        Message::VolumeChanged(section, sensor, percent)
    })
    .width(Length::Fixed(BAR_W))
    .height(BAR_H)
    .step(1.0_f32)
    .on_release(Message::VolumeCommitted(section, sensor))
    .style(move |_theme, _status| slider::Style {
        rail: Rail {
            backgrounds: (Background::Color(fill), Background::Color(track)),
            width: 4.0,
            border: Border {
                radius: 2.0.into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
        },
        handle: Handle {
            shape: HandleShape::Circle { radius: 4.0 },
            background: Background::Color(fill),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    })
    .into()
}

/// The hover-only percentage. The caller adds this widget only while
/// the sensor is hovered, so the normal sensor box carries no empty
/// slot; the fixed width only prevents jitter once it is shown.
pub(crate) fn percent<'a>(
    value: &str,
    color: Color,
    font: Font,
    font_size: u32,
) -> Element<'a, Message> {
    container(
        text(percent_text(value))
            .size(font_size.saturating_sub(1))
            .color(color)
            .font(font),
    )
    .width(Length::Fixed(LABEL_W))
    .align_x(Alignment::End)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_reads_bare_and_suffixed_values_and_handles_na() {
        assert_eq!(level("50"), 50.0);
        assert_eq!(level("50%"), 50.0);
        assert_eq!(level("N/A"), 0.0);
        assert_eq!(level("100%"), 100.0);
        assert_eq!(level("150%"), 100.0);
    }

    #[test]
    fn hover_percentage_is_formatted_only_when_requested() {
        assert_eq!(percent_text("50"), "50%");
        assert_eq!(percent_text("50%"), "50%");
        assert_eq!(percent_text("N/A"), "N/A");
    }
}
