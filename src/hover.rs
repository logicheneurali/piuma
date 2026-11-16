//! Hover arming and the sensor-box query.
//!
//! A sensor hover is not instantaneous: the enter is armed (`HoverGate`),
//! confirmed only by a real pointer motion, and the popups follow it.
//! The widget tree answers "where is that sensor's box" through
//! `FindSensorRect`, so an enter whose bounds moved under a still pointer
//! cannot open a popup nobody hovered.

use std::time::Instant;

use iced::advanced::widget::Operation;
use iced_runtime::core::widget::operation::Outcome;

use crate::popup_calendar::calendar_hover_sync;
use crate::popup_graph::graph_hover_sync;
use crate::sensor_runner;
use crate::mpris;
use crate::tooltip::{tooltip_close, tooltip_open_owned};
use crate::{config, Command, Message, Panel};

#[derive(Default)]
pub(crate) struct HoverGate {
    /// The armed enter (section, sensor, sequence) and when it armed,
    /// `None` while idle.
    pending: Option<(usize, usize, u64, Instant)>,
    /// Bumped on every enter: a confirmation carrying an older one is
    /// stale and must not fire.
    seq: u64,
}

impl HoverGate {
    /// Arm an enter; the returned token is what its confirmation must
    /// carry to still be accepted.
    pub(crate) fn enter(&mut self, section: usize, sensor: usize) -> (usize, usize, u64) {
        self.seq += 1;
        let token = (section, sensor, self.seq);
        self.pending = Some((section, sensor, self.seq, Instant::now()));
        token
    }

    /// Cancel the armed enter of this sensor — a leave must not drop
    /// another sensor's pending arm (enter/leave can arrive reordered).
    pub(crate) fn leave(&mut self, section: usize, sensor: usize) {
        if matches!(self.pending, Some((s, i, _, _)) if (s, i) == (section, sensor)) {
            self.pending = None;
        }
    }

    /// The arm's instant, exactly once, for a confirmation no leave or
    /// newer enter has superseded.
    pub(crate) fn confirm(&mut self, section: usize, sensor: usize, seq: u64) -> Option<Instant> {
        match self.pending {
            Some((s, i, n, at)) if (s, i, n) == (section, sensor, seq) => {
                self.pending = None;
                Some(at)
            }
            _ => None,
        }
    }
}

/// How close to the arm a pointer motion must be for the enter to count
/// as real. iced's `MouseArea` fires `on_enter` also when the widget's
/// *bounds* change under a still pointer (`bounds != state.bounds`), so a
/// pill that grows or shifts with its next reading could open a popup
/// nobody hovered. The motion that really enters a widget is the same
/// event as the enter (or a few messages away), hence the small window.
pub(crate) const HOVER_MOTION_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

/// Whether the arm at `armed` was accompanied by a pointer motion
/// (`last_motion`, the last `CursorMoved` on the bar). A synthetic enter
/// from a bounds change has no motion next to it: the pointer has been
/// still, possibly for seconds.
pub(crate) fn hover_is_fresh(armed: Instant, last_motion: Option<Instant>) -> bool {
    last_motion.is_some_and(|motion| armed.saturating_duration_since(motion) <= HOVER_MOTION_GRACE)
}

// ── sensor box query ───────────────────────────────────────────────

/// Widget id of a sensor box in the bar view: a hover asks the widget
/// tree for the box the pointer must be inside (`FindSensorRect`).
pub(crate) fn sensor_box_id(section: usize, sensor: usize) -> iced::widget::Id {
    format!("piuma-sensor-{section}-{sensor}").into()
}

/// Runtime query of a sensor box's rect: the operation walks the widget
/// trees (on every window — only the bar carries the tagged container)
/// and reports the bounds of the container tagged with the sensor's id.
pub(crate) struct FindSensorRect {
    section: usize,
    sensor: usize,
    target: iced::widget::Id,
    found: Option<(i32, i32, i32, i32)>,
}

impl FindSensorRect {
    pub(crate) fn new(section: usize, sensor: usize) -> Self {
        Self {
            section,
            sensor,
            target: sensor_box_id(section, sensor),
            found: None,
        }
    }
}

impl Operation<Message> for FindSensorRect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&iced::widget::Id>, bounds: iced::Rectangle) {
        if self.found.is_none() && id == Some(&self.target) {
            self.found = Some((
                bounds.x as i32,
                bounds.y as i32,
                (bounds.x + bounds.width) as i32,
                (bounds.y + bounds.height) as i32,
            ));
        }
    }

    fn finish(&self) -> Outcome<Message> {
        match self.found {
            Some(rect) => Outcome::Some(Message::SensorRectFound(self.section, self.sensor, rect)),
            None => Outcome::None,
        }
    }
}

/// Whether the bar-relative `cursor` sits inside `rect` (the layout is in
/// bar coordinates too). `None` (no motion seen yet) never matches.
pub(crate) fn cursor_in_rect(cursor: Option<(f32, f32)>, rect: (i32, i32, i32, i32)) -> bool {
    let Some((x, y)) = cursor else {
        return false;
    };
    x >= rect.0 as f32 && x < rect.2 as f32 && y >= rect.1 as f32 && y < rect.3 as f32
}

/// Both hover popups in step with one sensor hover: the trend popup
/// (its own sensor type) and the calendar one — each opens only for
/// its own source, so they share no condition beyond the hover.
pub(crate) fn hover_sync(panel: &mut Panel, section: usize, sensor: usize, hovered: bool) -> Command<Message> {
    Command::batch([
        graph_hover_sync(panel, section, sensor, hovered),
        calendar_hover_sync(panel, section, sensor, hovered),
        list_hover_sync(panel, section, sensor, hovered),
        media_hover_sync(panel, section, sensor, hovered),
    ])
}

/// A list sensor's hover popup: the command's lines (at most
/// `max_lines`) in the tooltip surface — the same small overlay the
/// taskbar uses, so no new window type and the same leave rule.
pub(crate) fn list_hover_sync(
    panel: &mut Panel,
    section: usize,
    sensor: usize,
    hovered: bool,
) -> Command<Message> {
    let id = format!("list:{section}:{sensor}");
    if !hovered {
        return tooltip_close(panel, id);
    }
    let Some(source) = panel
        .config
        .panel
        .sections
        .get(section)
        .and_then(|sec| sec.sensors.get(sensor))
        .map(|cfg_s| &cfg_s.source)
    else {
        return Command::none();
    };
    let config::SensorSource::List { command, max_lines, .. } = source else {
        return Command::none();
    };
    let lines: Vec<String> = sensor_runner::list_lines(command)
        .into_iter()
        .take(*max_lines as usize)
        .collect();
    if lines.is_empty() {
        return Command::none();
    }
    tooltip_open_owned(panel, id, lines.join("\n"), (section, sensor), None)
}

/// The media sensor's hover popup: the full artist - title plus the
/// position and total length when the player reports them. Same surface
/// and leave rule as the list popups.
pub(crate) fn media_hover_sync(
    panel: &mut Panel,
    section: usize,
    sensor: usize,
    hovered: bool,
) -> Command<Message> {
    let id = format!("media:{section}:{sensor}");
    if !hovered {
        return tooltip_close(panel, id);
    }
    let Some(media) = panel
        .sections
        .get(section)
        .and_then(|sec| sec.sensors.get(sensor))
        .and_then(|value| value.media.as_ref())
    else {
        return Command::none();
    };
    tooltip_open_owned(
        panel,
        id,
        mpris::media_details(media),
        (section, sensor),
        mpris::media_progress(media),
    )
}

















