//! Sensor trend graphs (hover popup, roadmap2 #11).
//!
//! The samples of every `draw_graph` sensor live in **one dense
//! column-major ring** allocated per config: a single `Vec<f32>` of
//! `columns × SAMPLES` values, versus one growable queue per sensor
//! (per-queue struct overhead, capacity slack and — if keyed in a map —
//! key entries on top of the data). E graphable sensors cost one
//! allocation of `E × 120 × 4 B` (≈ 4.8 KB for six sensors) and
//! nothing at all for the rest of the config.

use iced::Color;
use iced::mouse::Cursor;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};

use crate::config;

/// Samples kept per column: the tick is 1 Hz, so this is the trend of
/// the last two minutes.
pub const SAMPLES: usize = 120;

/// Y-axis policy of a graph: percentages keep the fixed 0..=100 axis,
/// everything else follows the visible window (`used_*` normalizes to
/// percent-of-total or the base unit — see `sensor_runner`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scale {
    Percent,
    Auto,
}

/// The Y-scale a source's graph uses, or `None` when the source has no
/// numeric trend and the `draw_graph` toggle is inert. Single source of
/// truth for "graphable": the history builder, the runner (which
/// attaches scales to samples) and the hover wiring all consult it.
pub fn scale_of(source: &config::SensorSource) -> Option<Scale> {
    use config::SensorSource as S;
    Some(match source {
        S::PercCpu {}
        | S::PercGpu {}
        | S::PercMem {}
        | S::PercSwap {}
        | S::PercDisk {}
        | S::PercBattery {}
        | S::PercBacklight {}
        | S::PercVolume { .. } => Scale::Percent,
        S::UsedMem {}
        | S::UsedSwap {}
        | S::UsedDisk {}
        | S::UsedBattery {}
        | S::UsedNetdown { .. }
        | S::UsedNetup { .. }
        | S::TempCpu {} => Scale::Auto,
        _ => return None,
    })
}

/// Semi-transparent alpha of the quarter grid under the data.
const GRID_ALPHA: f32 = 0.25;
/// Alpha of the average guide line (stronger than the grid, softer
/// than the trend).
const AVG_ALPHA: f32 = 0.55;
/// Quarter steps per axis: 4 divisions = the 0/25/50/75/100% lines.
const GRID_STEPS: u32 = 4;

/// y of a 0..1 fraction of the axis: 0 = bottom (minimum), 1 = top
/// (maximum). The single source for grid, data, guide line and the
/// label's position — the view calls it too.
pub fn axis_y(h: f32, frac: f32) -> f32 {
    h * (1.0 - frac)
}

/// Arithmetic mean of a window — the average guide's value; `None`
/// while nothing was recorded yet (no line, no label).
pub fn mean(window: &[f32]) -> Option<f32> {
    if window.is_empty() {
        return None;
    }
    Some(window.iter().sum::<f32>() / window.len() as f32)
}

/// Label suffix of a graphed source's values: the engine plots the
/// bare number, the popup's average label carries the unit.
pub fn unit_of(source: &config::SensorSource) -> &'static str {
    use config::SensorSource as S;
    match source {
        S::PercCpu {}
        | S::PercGpu {}
        | S::PercMem {}
        | S::PercSwap {}
        | S::PercDisk {}
        | S::PercBattery {}
        | S::PercBacklight {}
        | S::PercVolume { .. }
        | S::UsedMem {}
        | S::UsedSwap {}
        | S::UsedDisk {}
        | S::UsedBattery {} => "%",
        S::UsedNetdown { .. } | S::UsedNetup { .. } => "B/s",
        S::TempCpu {} => "°C",
        _ => "",
    }
}

/// The average guide's label: `avg 4.8%`, `avg 61.5°C`,
/// `avg 9318 B/s` (rates drop the decimal — bytes have none).
pub fn avg_label(avg: f32, unit: &str) -> String {
    match unit {
        "B/s" => format!("avg {avg:.0} {unit}"),
        "" => format!("avg {avg:.1}"),
        _ => format!("avg {avg:.1}{unit}"),
    }
}

/// One recorded column: which sensor it belongs to and its axis policy.
#[derive(Clone)]
struct Column {
    section: usize,
    sensor: usize,
    scale: Scale,
}

/// The history of every graphable sensor, rebuilt on config reload.
#[derive(Clone)]
pub struct History {
    columns: Vec<Column>,
    /// `columns.len() × SAMPLES` values, column-major; `f32::NAN` = no
    /// sample yet.
    data: Vec<f32>,
    /// Ring write position, shared by every column so they stay
    /// time-aligned.
    head: usize,
    /// Ticks recorded so far, capped at `SAMPLES` (the ring is only
    /// "full" once it wrapped).
    filled: usize,
}

impl History {
    /// One column per config sensor with `draw_graph` on a graphable
    /// source; any other sensor costs no memory.
    pub fn new(sections: &[config::SectionConfig]) -> Self {
        let mut columns = Vec::new();
        let mut data = Vec::new();
        for (section, sec) in sections.iter().enumerate() {
            for (sensor, s) in sec.sensors.iter().enumerate() {
                if let Some(scale) = s.draw_graph.then(|| scale_of(&s.source)).flatten() {
                    columns.push(Column {
                        section,
                        sensor,
                        scale,
                    });
                    data.resize(data.len() + SAMPLES, f32::NAN);
                }
            }
        }
        Self {
            columns,
            data,
            head: 0,
            filled: 0,
        }
    }

    /// Record one tick: every known `(section, sensor)` in `samples`
    /// gets its value, the columns not sampled repeat their previous one
    /// (a missing reading must not shift the time axis), and the ring
    /// advances once. Unknown keys are ignored (a reload rebuilt the
    /// columns under the running tick).
    pub fn record(&mut self, samples: &[((usize, usize), f64)]) {
        if self.columns.is_empty() {
            return;
        }
        let prev = (self.head + SAMPLES - 1) % SAMPLES;
        for (ci, col) in self.columns.iter().enumerate() {
            let fresh = samples
                .iter()
                .find(|((section, sensor), _)| (*section, *sensor) == (col.section, col.sensor))
                .map(|(_, value)| *value as f32);
            let previous = self.data[ci * SAMPLES + prev];
            self.data[ci * SAMPLES + self.head] = fresh.unwrap_or(previous);
        }
        self.head = (self.head + 1) % SAMPLES;
        self.filled = (self.filled + 1).min(SAMPLES);
    }

    /// `(scale, samples oldest → newest)` of a sensor's column; the
    /// still-unrecorded tail renders as zeros (the popup can open
    /// before the first tick fills the ring).
    pub fn window(&self, section: usize, sensor: usize) -> Option<(Scale, Vec<f32>)> {
        let ci = self
            .columns
            .iter()
            .position(|c| c.section == section && c.sensor == sensor)?;
        let start = (self.head + SAMPLES - self.filled) % SAMPLES;
        let mut out = Vec::with_capacity(self.filled);
        for i in 0..self.filled {
            let v = self.data[ci * SAMPLES + (start + i) % SAMPLES];
            out.push(if v.is_finite() { v } else { 0.0 });
        }
        Some((self.columns[ci].scale, out))
    }
}

/// Y bounds of a window: percentages keep the fixed 0..=100 axis; the
/// rest follows the window min..max, padded so tiny wiggles stay
/// visible and widened around a flat line so it still has a middle to
/// draw (never a zero span).
pub fn y_bounds(scale: Scale, window: &[f32]) -> (f32, f32) {
    match scale {
        Scale::Percent => (0.0, 100.0),
        Scale::Auto => {
            let min = window.iter().copied().fold(f32::INFINITY, f32::min);
            let max = window.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if !min.is_finite() || !max.is_finite() {
                return (0.0, 1.0);
            }
            if max - min < f32::EPSILON {
                return (min - 1.0, max + 1.0);
            }
            let pad = (max - min) * 0.05;
            (min - pad, max + pad)
        }
    }
}

/// The trend line drawn by the popup: plain data, the canvas machinery
/// lives in `Program::draw`.
pub struct Graph {
    samples: Vec<f32>,
    scale: Scale,
    color: Color,
}

/// The popup's canvas widget for one sensor's window: it fills the
/// space the popup view gives it (the window size rules the canvas
/// bounds, the draw pass reads them from `bounds`).
pub fn widget(samples: Vec<f32>, scale: Scale, color: Color) -> iced::Element<'static, crate::Message> {
    canvas::Canvas::new(Graph {
        samples,
        scale,
        color,
    })
    .width(iced::Length::Fill)
    .height(iced::Length::Fill)
    .into()
}

impl<Message> canvas::Program<Message> for Graph {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        if w <= 0.0 || h <= 0.0 {
            return vec![frame.into_geometry()];
        }
        // Grid under the data: five quarter lines of the axis — the
        // literal 0/25/50/75/100% on a percent scale, quarters of the
        // visible min..max range on an auto one.
        for step in 0..=GRID_STEPS {
            let y = axis_y(h, step as f32 / GRID_STEPS as f32);
            let line = Path::new(|path| {
                path.move_to(iced::Point::new(0.0, y));
                path.line_to(iced::Point::new(w, y));
            });
            frame.stroke(
                &line,
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color {
                        a: GRID_ALPHA,
                        ..self.color
                    }),
            );
        }
        // The trend itself.
        if self.samples.len() >= 2 {
            let (lo, hi) = y_bounds(self.scale, &self.samples);
            let last = (self.samples.len() - 1) as f32;
            let line = Path::new(|path| {
                for (i, v) in self.samples.iter().enumerate() {
                    let x = w * i as f32 / last;
                    let y = axis_y(h, (v - lo) / (hi - lo));
                    if i == 0 {
                        path.move_to(iced::Point::new(x, y));
                    } else {
                        path.line_to(iced::Point::new(x, y));
                    }
                }
            });
            frame.stroke(
                &line,
                Stroke::default().with_width(2.0).with_color(self.color),
            );
        }
        // Average guide over the whole window; the popup view labels it
        // through the same `axis_y` math (see `avg_label`).
        if let Some(avg) = mean(&self.samples) {
            let (lo, hi) = y_bounds(self.scale, &self.samples);
            let y = axis_y(h, (avg - lo) / (hi - lo));
            let line = Path::new(|path| {
                path.move_to(iced::Point::new(0.0, y));
                path.line_to(iced::Point::new(w, y));
            });
            frame.stroke(
                &line,
                Stroke::default().with_width(1.0).with_color(Color {
                    a: AVG_ALPHA,
                    ..self.color
                }),
            );
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(json: &str) -> config::SectionConfig {
        serde_json::from_str(json).expect("valid section")
    }

    /// Columns come only from `draw_graph` on graphable sources, and the
    /// flag round-trips through the config JSON (absent = off).
    #[test]
    fn columns_follow_the_toggle_and_the_source_policy() {
        let sec = section(
            r#"{"sensors":[
                {"name":"CPU","source":{"type":"perc_cpu"},"draw_graph":true},
                {"name":"MEM","source":{"type":"used_mem"},"draw_graph":true},
                {"name":"UP","source":{"type":"uptime"},"draw_graph":true},
                {"name":"GPU","source":{"type":"perc_gpu"}}
            ]}"#,
        );
        let h = History::new(std::slice::from_ref(&sec));
        assert_eq!(h.columns.len(), 2, "uptime and the plain GPU sensor take no column");
        assert_eq!(h.columns[0].scale, Scale::Percent, "perc_ keeps the fixed axis");
        assert_eq!(h.columns[1].scale, Scale::Auto, "used_ follows the window");
        // Serde: absent flag parses as off.
        assert!(!sec.sensors[3].draw_graph);
    }

    /// The ring wraps, missing readings repeat the previous value and
    /// the window unwraps oldest → newest.
    #[test]
    fn ring_wraps_and_stays_time_aligned() {
        let sec = section(r#"{"sensors":[{"name":"M","source":{"type":"used_mem"},"draw_graph":true}]}"#);
        let mut h = History::new(std::slice::from_ref(&sec));
        h.record(&[((0, 0), 10.0)]);
        h.record(&[((0, 0), 20.0)]);
        h.record(&[]); // no reading this tick: the last value holds
        let (scale, w) = h.window(0, 0).expect("column");
        assert_eq!(scale, Scale::Auto);
        assert_eq!(w, vec![10.0, 20.0, 20.0]);
        // Fill past the ring end: order must survive the wrap.
        for i in 4..SAMPLES as u64 + 3 {
            h.record(&[((0, 0), i as f64)]);
        }
        let (_, w) = h.window(0, 0).expect("column");
        assert_eq!(w.len(), SAMPLES, "capped");
        assert_eq!(w[SAMPLES - 2], (SAMPLES as u64 + 1) as f32, "wrapped tail keeps order");
        assert_eq!(w[SAMPLES - 1], (SAMPLES as u64 + 2) as f32);
        assert_eq!(h.window(5, 5), None, "unknown sensor");
    }

    /// Percentages span 0..=100; auto windows pad around min..max and a
    /// flat line gets a non-zero span to draw in the middle.
    #[test]
    fn y_bounds_follow_the_policy() {
        assert_eq!(y_bounds(Scale::Percent, &[3.0, 97.0]), (0.0, 100.0));
        let (lo, hi) = y_bounds(Scale::Auto, &[10.0, 14.0]);
        assert!(lo < 10.0 && hi > 14.0, "padded: {lo}..{hi}");
        let (lo, hi) = y_bounds(Scale::Auto, &[7.0, 7.0]);
        assert!(lo < hi, "flat window keeps a span: {lo}..{hi}");
        assert_eq!(y_bounds(Scale::Auto, &[]), (0.0, 1.0), "empty window");
    }

    /// The mean drives the guide (empty window = no line) and the grid
    /// spans bottom → top in five exact quarters.
    #[test]
    fn mean_and_quarter_lines() {
        assert_eq!(mean(&[]), None, "no samples → no guide line");
        assert_eq!(mean(&[10.0, 20.0, 60.0]), Some(30.0));
        let grid: Vec<f32> = (0..=GRID_STEPS)
            .map(|step| axis_y(90.0, step as f32 / GRID_STEPS as f32))
            .collect();
        assert_eq!(grid, vec![90.0, 67.5, 45.0, 22.5, 0.0], "0/25/50/75/100%");
    }

    /// The average label carries the source's unit, rates without a
    /// decimal (bytes are whole).
    #[test]
    fn average_labels_carry_the_units() {
        assert_eq!(avg_label(4.84, "%"), "avg 4.8%");
        assert_eq!(avg_label(61.54, "°C"), "avg 61.5°C");
        assert_eq!(avg_label(9318.4, "B/s"), "avg 9318 B/s");
        assert_eq!(avg_label(1.0, ""), "avg 1.0");
    }
}
