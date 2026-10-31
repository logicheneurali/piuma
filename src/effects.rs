//! Lightweight value-change effects: a short fade plus either an
//! underline sweep or a sheen over the value. They run only while an
//! active effect exists (see the `EffectFrame` subscription in main).

use crate::Message;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path};
use iced::widget::Stack;
use iced::{Color, Element, Length, Point, Size};
use std::time::{Duration, Instant};

/// Total effect lifetime. Short enough to read as a pulse, not an
/// animation loop.
pub const DURATION: Duration = Duration::from_millis(240);

/// Animation cadence while an effect is alive: 30 fps is enough for a
/// 240 ms fade and keeps the idle bar at zero animation frames.
pub const FRAME: Duration = Duration::from_millis(33);

pub const UNDERLINE: &str = "underline";
pub const SHEEN: &str = "sheen";

/// 0..1 progress of an effect, or `None` once it has expired.
pub fn progress(start: Instant) -> Option<f32> {
    let t = start.elapsed().as_secs_f32() / DURATION.as_secs_f32();
    (t < 1.0).then_some(t.clamp(0.0, 1.0))
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// A sheen must contrast with the value's text colour: white on a dark
/// theme, black on a light one.
fn highlight(color: Color) -> Color {
    let luminance = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
    if luminance > 0.5 {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

/// The text fade shared by both modes: the new value starts dim and
/// reaches its normal colour in the same 240 ms window.
pub fn fade(color: Color, t: f32) -> Color {
    Color {
        a: color.a * (0.62 + 0.38 * ease_out(t)),
        ..color
    }
}

/// A real value change worth animating: no pulse for the first fill
/// (old value absent/empty), no pulse when the text is unchanged.
pub fn value_changed(old: Option<&str>, new: &str) -> bool {
    old.is_some_and(|old| !old.is_empty() && old != new)
}

/// Underline growth and alpha: grow to full width by 55% of the effect,
/// then fade out.
fn underline_parts(t: f32) -> (f32, f32) {
    let grow = ease_out((t / 0.55).min(1.0));
    let alpha = if t <= 0.55 {
        1.0
    } else {
        (1.0 - (t - 0.55) / 0.45).max(0.0)
    };
    (grow, alpha)
}

/// Wrap a value with the underline slot. The slot is always present for
/// an opted sensor so the sensor height does not jump when the effect
/// starts; only the line's width and alpha animate.
pub fn underline<'a>(
    value: Element<'a, Message>,
    width: f32,
    height: f32,
    color: Color,
    t: Option<f32>,
) -> Element<'a, Message> {
    if width <= 0.0 || height <= 0.0 {
        return value;
    }
    let (fraction, alpha) = t.map(underline_parts).unwrap_or((0.0, 0.0));
    let line_color = Color {
        a: color.a * alpha,
        ..color
    };
    let canvas = Canvas::new(Underline { fraction, color: line_color })
        .width(Length::Fixed(width))
        .height(Length::Fixed(height));
    Stack::new()
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .push(value)
        .push(canvas)
        .into()
}

struct Underline {
    fraction: f32,
    color: Color,
}

impl<Message> canvas::Program<Message> for Underline {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let line_w = bounds.width * self.fraction.clamp(0.0, 1.0);
        if line_w > 0.0 && self.color.a > 0.0 {
            let x = (bounds.width - line_w) / 2.0;
            frame.fill(
                &Path::rectangle(
                    Point::new(x, bounds.height - 1.5),
                    Size::new(line_w, 1.5),
                ),
                self.color,
            );
        }
        vec![frame.into_geometry()]
    }
}

/// Wrap a value with a transparent-overlay canvas: the band sweeps over
/// the text at 30 fps while the effect is alive.
pub fn sheen<'a>(
    value: Element<'a, Message>,
    width: f32,
    height: f32,
    color: Color,
    t: Option<f32>,
) -> Element<'a, Message> {
    if width <= 0.0 || height <= 0.0 {
        return value;
    }
    let canvas = Canvas::new(Sheen {
        progress: t.unwrap_or(0.0),
        color: highlight(color),
        active: t.is_some(),
    })
    .width(Length::Fixed(width))
    .height(Length::Fixed(height));
    Stack::new()
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .push(canvas)
        .push(value)
        .into()
}

struct Sheen {
    progress: f32,
    color: Color,
    active: bool,
}

impl<Message> canvas::Program<Message> for Sheen {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        if !self.active || bounds.width <= 0.0 || bounds.height <= 0.0 {
            return vec![frame.into_geometry()];
        }
        let t = self.progress.clamp(0.0, 1.0);
        let band = (bounds.width * 0.7).max(10.0);
        let x = -band + (bounds.width + band) * ease_out(t);
        let alpha = self.color.a
            * 0.35
            * (std::f32::consts::PI * t).sin().max(0.0);
        if alpha > 0.0 {
            frame.fill(
                &Path::rectangle(Point::new(x, 0.0), Size::new(band, bounds.height)),
                Color { a: alpha, ..self.color },
            );
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_reaches_full_opacity_at_the_end() {
        let c = Color::from_rgb(0.5, 0.5, 0.5);
        assert!((fade(c, 0.0).a - 0.62).abs() < 0.001);
        assert!((fade(c, 1.0).a - 1.0).abs() < 0.001);
    }

    #[test]
    fn underline_grows_then_fades() {
        assert_eq!(underline_parts(0.0), (0.0, 1.0));
        let (width, alpha) = underline_parts(0.55);
        assert!((width - 1.0).abs() < 0.001);
        assert!((alpha - 1.0).abs() < 0.001);
        let (_, alpha_end) = underline_parts(1.0);
        assert_eq!(alpha_end, 0.0);
    }

    #[test]
    fn progress_expires_after_the_window() {
        assert!(progress(Instant::now() - DURATION - Duration::from_millis(1)).is_none());
        assert!(progress(Instant::now()).is_some());
    }

    #[test]
    fn only_real_changes_trigger_the_effect() {
        assert!(value_changed(Some("3%"), "4%"));
        assert!(!value_changed(Some("4%"), "4%"));
        assert!(!value_changed(None, "4%"));
        assert!(!value_changed(Some(""), "4%"));
    }
}
