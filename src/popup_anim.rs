//! A one-shot slide for the bar's cursor popups: the surface opens a few
//! pixels nearer the screen edge it is anchored to and reaches its final
//! margin in 120 ms. A centred surface (no single vertical anchor) opts
//! out: its margins do not move it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use iced::Task;
use iced_exwlshell::reexport::{Anchor, Margin};

use crate::Message;

/// One animation frame while a slide is alive.
pub(crate) const FRAME: Duration = Duration::from_millis(16);
/// Total slide time: short enough to read as the popup appearing.
const SLIDE: Duration = Duration::from_millis(120);
/// How far from its resting place the surface starts.
const OFFSET: i32 = 6;

struct Slide {
    id: iced::window::Id,
    from: Margin,
    to: Margin,
    started: Instant,
}

static SLIDES: Mutex<Vec<Slide>> = Mutex::new(Vec::new());

/// The margin a popup opens with: `to` shifted `OFFSET` towards the
/// anchor's screen edge (top or bottom). Anchors with both vertical
/// edges (a centred menu) or none return `to` unchanged.
pub(crate) fn opening_margin(anchor: Anchor, to: Margin) -> Margin {
    let mut from = to;
    match (anchor.contains(Anchor::Top), anchor.contains(Anchor::Bottom)) {
        (true, false) => from.top -= OFFSET,
        (false, true) => from.bottom -= OFFSET,
        _ => return to,
    }
    from
}

/// Register the slide for a surface that just opened with
/// [`opening_margin`].
pub(crate) fn start(id: iced::window::Id, anchor: Anchor, to: Margin) {
    let from = opening_margin(anchor, to);
    if from == to {
        return;
    }
    let mut slides = SLIDES.lock().unwrap_or_else(|e| e.into_inner());
    slides.push(Slide {
        id,
        from,
        to,
        started: Instant::now(),
    });
}

/// Whether the frame timer must run.
pub(crate) fn active() -> bool {
    SLIDES
        .lock()
        .map(|slides| !slides.is_empty())
        .unwrap_or(false)
}

/// One animation frame: send every slide's current margin and drop the
/// ones that landed.
pub(crate) fn frame() -> Task<Message> {
    let now = Instant::now();
    let mut slides = SLIDES.lock().unwrap_or_else(|e| e.into_inner());
    let mut cmds = Vec::with_capacity(slides.len());
    slides.retain(|slide| {
        let t = (now.saturating_duration_since(slide.started).as_secs_f32()
            / SLIDE.as_secs_f32())
        .clamp(0.0, 1.0);
        // Smoothstep: no visible snap at either end.
        let eased = t * t * (3.0 - 2.0 * t);
        cmds.push(Task::done(Message::MarginChange {
            id: slide.id,
            margin: lerp(slide.from, slide.to, eased),
        }));
        t < 1.0
    });
    Task::batch(cmds)
}

fn lerp(from: Margin, to: Margin, t: f32) -> Margin {
    let mix = |a: i32, b: i32| (a as f32 + (b - a) as f32 * t).round() as i32;
    Margin {
        top: mix(from.top, to.top),
        right: mix(from.right, to.right),
        bottom: mix(from.bottom, to.bottom),
        left: mix(from.left, to.left),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_margin_moves_towards_the_anchored_edge() {
        let to = Margin {
            top: 40,
            right: 0,
            bottom: 10,
            left: 5,
        };
        let top = opening_margin(Anchor::Top | Anchor::Left, to);
        assert_eq!((top.top, top.bottom), (to.top - OFFSET, to.bottom));
        let bottom = opening_margin(Anchor::Bottom | Anchor::Right, to);
        assert_eq!((bottom.top, bottom.bottom), (to.top, to.bottom - OFFSET));
        assert_eq!(opening_margin(Anchor::all(), to), to);
    }

    #[test]
    fn the_lerp_lands_exactly_on_the_target() {
        let from = Margin {
            top: 34,
            right: 0,
            bottom: 10,
            left: 5,
        };
        let to = Margin {
            top: 40,
            right: 0,
            bottom: 10,
            left: 5,
        };
        assert_eq!(lerp(from, to, 0.0), from);
        assert_eq!(lerp(from, to, 1.0), to);
    }
}
