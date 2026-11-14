//! A one-shot slide for the bar's cursor popups: the surface opens a few
//! pixels nearer the screen edge it is anchored to and reaches its final
//! margin in 120 ms; closing runs the same slide backwards while a ghost
//! of the popup keeps drawing its last content. A centred surface (no
//! single vertical anchor) does not move: it is not animated.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use iced::{Element, Task};
use iced_exwlshell::reexport::{Anchor, Margin};

use crate::popup_calendar::calendar_popup_view;
use crate::popup_graph::graph_popup_view;
use crate::popup_notifications::{notif_list_view_with, popup_view};
use crate::menus::{taskbar_menu_view_with, tray_menu_view_with};
use crate::{CalendarPopup, GraphPopup, Message, NotifList, Panel, TrayMenu};

/// One animation frame while a slide is alive.
pub(crate) const FRAME: Duration = Duration::from_millis(16);
/// Total slide time: short enough to read as the popup appearing.
const SLIDE: Duration = Duration::from_millis(120);
/// How far from its resting place the surface starts (and ends, closing).
const OFFSET: i32 = 6;

/// The last content of a popup that is sliding out: the window outlives
/// its state for the closing animation, and the view renders this ghost.
pub(crate) enum Ghost {
    Graph(GraphPopup),
    Calendar(CalendarPopup),
    NotifCards,
    NotifList(NotifList),
    Tray(TrayMenu),
    Taskbar,
}

struct Slide {
    id: iced::window::Id,
    from: Margin,
    to: Margin,
    started: Instant,
    /// Closing slides send `PopupClosed` at the end instead of stopping.
    closing: bool,
}

static SLIDES: Mutex<Vec<Slide>> = Mutex::new(Vec::new());
/// Where each popup rests and which edge it is anchored to: closing
/// needs the target to slide away from the same edge.
static TARGETS: OnceLock<Mutex<HashMap<iced::window::Id, (Anchor, Margin)>>> = OnceLock::new();

fn targets() -> &'static Mutex<HashMap<iced::window::Id, (Anchor, Margin)>> {
    TARGETS.get_or_init(Default::default)
}

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

/// Register the surface that just opened, and start its slide.
pub(crate) fn start(id: iced::window::Id, anchor: Anchor, to: Margin) {
    if let Ok(mut targets) = targets().lock() {
        targets.insert(id, (anchor, to));
    }
    let from = opening_margin(anchor, to);
    if from == to {
        return;
    }
    let mut slides = SLIDES.lock().unwrap_or_else(|e| e.into_inner());
    slides.retain(|slide| slide.id != id);
    slides.push(Slide {
        id,
        from,
        to,
        started: Instant::now(),
        closing: false,
    });
}

/// Start the closing slide and hand the view a ghost of the state that
/// is about to be cleared. Without a registered target (a centred menu)
/// the surface is closed at once.
pub(crate) fn close(panel: &mut Panel, id: iced::window::Id, ghost: Ghost) -> Task<Message> {
    let target = targets()
        .lock()
        .ok()
        .and_then(|targets| targets.get(&id).copied());
    let Some((anchor, to)) = target else {
        return iced::window::close(id);
    };
    panel.closing.push((id, ghost));
    let mut slides = SLIDES.lock().unwrap_or_else(|e| e.into_inner());
    slides.retain(|slide| slide.id != id);
    slides.push(Slide {
        id,
        from: to,
        to: opening_margin(anchor, to),
        started: Instant::now(),
        closing: true,
    });
    Task::none()
}

/// Whether the frame timer must run.
pub(crate) fn active() -> bool {
    SLIDES
        .lock()
        .map(|slides| !slides.is_empty())
        .unwrap_or(false)
}

/// One animation frame: send every slide's current margin, and at the
/// end of a closing one ask the app to destroy the surface.
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
        if t < 1.0 {
            return true;
        }
        if slide.closing {
            cmds.push(Task::done(Message::PopupClosed(slide.id)));
            if let Ok(mut targets) = targets().lock() {
                targets.remove(&slide.id);
            }
        }
        false
    });
    Task::batch(cmds)
}

/// The view of a popup that is sliding out: its state was already taken
/// by the close path, so the cloned ghost is the only source.
pub(crate) fn ghost_view<'a>(panel: &'a Panel, ghost: &'a Ghost) -> Element<'a, Message> {
    match ghost {
        Ghost::Graph(gp) => graph_popup_view(panel, gp),
        Ghost::Calendar(cp) => calendar_popup_view(panel, cp),
        Ghost::NotifCards => popup_view(panel),
        Ghost::NotifList(state) => notif_list_view_with(panel, state),
        Ghost::Tray(menu) => tray_menu_view_with(panel, menu),
        Ghost::Taskbar => taskbar_menu_view_with(panel),
    }
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
