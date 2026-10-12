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
        // The user-authored text sources: a number can come out of
        // any of them, taken by the runner's strict whole-string parse
        // — a non-numeric answer simply records nothing (see
        // `sensor_runner::strict_number`).
        S::Command { .. } | S::EnvVar { .. } | S::StaticText { .. } => Scale::Auto,
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

/// The average guide's label: `avg 5%`, `avg 61.5°C`,
/// `avg 9318 B/s` — percentages round to whole percents like their
/// sensors, rates drop the decimal (bytes have none).
pub fn avg_label(avg: f32, unit: &str) -> String {
    match unit {
        "B/s" => format!("avg {avg:.0} {unit}"),
        "%" => format!("avg {avg:.0}{unit}"),
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
    /// At least one real sample recorded: the hover popup stays closed
    /// until then — `draw_graph` on a source that never answered with
    /// a number (or a sensor `N/A` since boot) has no trend to show.
    sampled: bool,
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
                        sampled: false,
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
        for (ci, col) in self.columns.iter_mut().enumerate() {
            let fresh = samples
                .iter()
                .find(|((section, sensor), _)| (*section, *sensor) == (col.section, col.sensor))
                .map(|(_, value)| *value as f32);
            if fresh.is_some() {
                col.sampled = true;
            }
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

    /// Whether a column recorded at least one sample: the hover gate's
    /// "no data, no popup" check (a never-numeric `command` with
    /// `draw_graph` on, or a sensor reading `N/A` since boot).
    pub fn has_samples(&self, section: usize, sensor: usize) -> bool {
        self.columns
            .iter()
            .any(|c| c.section == section && c.sensor == sensor && c.sampled)
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

// ── connector (sensor box ↔ popup) ─────────────────────────────────

/// End-dot radius of the connector: a small filled circle on the
/// sensor's edge and on the popup's edge.
pub const CONNECTOR_DOT_R: f32 = 3.0;

/// Geometry of the connector joining a sensor box to its graph popup
/// (both given as output-space rects `(x0, y0, x1, y1)` of their real
/// surfaces): the dot on the sensor's edge facing the popup, the dot on
/// the popup's edge facing the sensor, and the polyline between them —
/// an organ-chart run of right angles only: straight away from the
/// sensor, one rail half-way across the gap, straight into the popup.
/// Aligned ends collapse the rail into a plain straight line.
pub fn connector_geometry(
    sensor: (i32, i32, i32, i32),
    popup: (i32, i32, i32, i32),
) -> (iced::Point, iced::Point, Vec<iced::Point>) {
    let sc = (
        (sensor.0 + sensor.2) as f32 / 2.0,
        (sensor.1 + sensor.3) as f32 / 2.0,
    );
    let pc = (
        (popup.0 + popup.2) as f32 / 2.0,
        (popup.1 + popup.3) as f32 / 2.0,
    );
    // The popup sits entirely on one side of the sensor box (a top bar
    // opens below, a left bar to the right): that side picks the edges
    // the two dots land on and the axis of the corridor.
    let (d1, d2, rail, vertical) = if popup.1 >= sensor.3 {
        // Below: the sensor's bottom edge → the popup's top edge.
        (
            iced::Point::new(sc.0, sensor.3 as f32),
            iced::Point::new(pc.0, popup.1 as f32),
            (sensor.3 + popup.1) as f32 / 2.0,
            true,
        )
    } else if popup.3 <= sensor.1 {
        (
            iced::Point::new(sc.0, sensor.1 as f32),
            iced::Point::new(pc.0, popup.3 as f32),
            (sensor.1 + popup.3) as f32 / 2.0,
            true,
        )
    } else if popup.0 >= sensor.2 {
        // Right: the sensor's right edge → the popup's left edge.
        (
            iced::Point::new(sensor.2 as f32, sc.1),
            iced::Point::new(popup.0 as f32, pc.1),
            (sensor.2 + popup.0) as f32 / 2.0,
            false,
        )
    } else {
        (
            iced::Point::new(sensor.0 as f32, sc.1),
            iced::Point::new(popup.2 as f32, pc.1),
            (sensor.0 + popup.2) as f32 / 2.0,
            false,
        )
    };
    let mut path = if vertical {
        vec![
            d1,
            iced::Point::new(d1.x, rail),
            iced::Point::new(d2.x, rail),
            d2,
        ]
    } else {
        vec![
            d1,
            iced::Point::new(rail, d1.y),
            iced::Point::new(rail, d2.y),
            d2,
        ]
    };
    // Aligned ends put the rail on the straight line: the duplicate
    // point (and only it) goes.
    path.dedup_by(|a, b| a == b);
    (d1, d2, path)
}

/// The connector's drawing: the polyline and its two end dots in the
/// link surface's own coordinates (the opener sizes that surface to
/// this drawing's bounding box plus a padding). Same border colour and
/// width as the popup box it belongs to.
pub struct Connector {
    pub d1: iced::Point,
    pub d2: iced::Point,
    pub path: Vec<iced::Point>,
    pub color: Color,
    pub width: f32,
}

/// The connector surface's whole view: one canvas filling the window.
pub fn link_widget(conn: Connector) -> iced::Element<'static, crate::Message> {
    canvas::Canvas::new(conn)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}

impl<Message> canvas::Program<Message> for Connector {
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
        // Between the popup's death and the tick's window close the
        // surface still renders: draw nothing rather than something.
        if self.path.len() < 2 {
            return vec![frame.into_geometry()];
        }
        let line = Path::new(|path| {
            for (i, p) in self.path.iter().enumerate() {
                if i == 0 {
                    path.move_to(*p);
                } else {
                    path.line_to(*p);
                }
            }
        });
        frame.stroke(
            &line,
            Stroke::default().with_width(self.width).with_color(self.color),
        );
        for dot in [self.d1, self.d2] {
            frame.fill(&Path::circle(dot, CONNECTOR_DOT_R), self.color);
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

    /// The user-authored text sources take an Auto column (their
    /// sample comes from the runner's strict parse), the semantic text
    /// sources take none, and a column reports data only after its
    /// first recorded sample.
    #[test]
    fn text_sources_take_a_column_once_they_carry_data() {
        let sec = section(
            r#"{"sensors":[
                {"name":"CMD","source":{"type":"command","command":"echo 42"},"draw_graph":true},
                {"name":"TXT","source":{"type":"static_text","static_text":"hi"},"draw_graph":true},
                {"name":"WIN","source":{"type":"window"},"draw_graph":true},
                {"name":"CLK","source":{"type":"clock"}}
            ]}"#,
        );
        let mut h = History::new(std::slice::from_ref(&sec));
        assert_eq!(
            h.columns.len(),
            2,
            "command and static_text take columns, window and clock don't"
        );
        assert!(
            h.columns.iter().all(|c| c.scale == Scale::Auto),
            "text sources follow the window"
        );
        assert!(!h.has_samples(0, 0), "no data yet: the popup stays closed");
        h.record(&[((0, 0), 42.0)]);
        assert!(h.has_samples(0, 0), "the sample flags the column");
        assert!(!h.has_samples(0, 1), "the other column has no data");
        assert!(!h.has_samples(0, 2), "window never takes a column");
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

    /// The average label carries the source's unit; percentages and
    /// rates show whole numbers, bytes have no decimal.
    #[test]
    fn average_labels_carry_the_units() {
        assert_eq!(avg_label(4.84, "%"), "avg 5%");
        assert_eq!(avg_label(61.54, "°C"), "avg 61.5°C");
        assert_eq!(avg_label(9318.4, "B/s"), "avg 9318 B/s");
        assert_eq!(avg_label(1.0, ""), "avg 1.0");
    }

    /// The connector drops from the sensor's bottom edge, rails across
    /// the half-way point of the gap and lands on the popup's top edge;
    /// aligned ends collapse the rail onto the straight line.
    #[test]
    fn connector_follows_an_organ_chart_path() {
        let (d1, d2, path) = connector_geometry((800, 4, 880, 24), (840, 42, 1100, 182));
        assert_eq!(d1, iced::Point::new(840.0, 24.0));
        assert_eq!(d2, iced::Point::new(970.0, 42.0));
        assert_eq!(
            path,
            vec![
                iced::Point::new(840.0, 24.0),
                iced::Point::new(840.0, 33.0),
                iced::Point::new(970.0, 33.0),
                iced::Point::new(970.0, 42.0),
            ]
        );

        let (d1, d2, path) = connector_geometry((800, 4, 880, 24), (800, 42, 880, 182));
        assert_eq!(d1, iced::Point::new(840.0, 24.0));
        assert_eq!(d2, iced::Point::new(840.0, 42.0));
        assert_eq!(path.len(), 3, "collinear run: {path:?}");
    }

    /// A popup to the sensor's right rails across the vertical gap
    /// instead (left/right bar layouts).
    #[test]
    fn connector_rails_across_a_side_popup() {
        let (d1, d2, path) = connector_geometry((4, 200, 24, 280), (60, 240, 320, 380));
        assert_eq!(d1, iced::Point::new(24.0, 240.0));
        assert_eq!(d2, iced::Point::new(60.0, 310.0));
        assert_eq!(
            path,
            vec![
                iced::Point::new(24.0, 240.0),
                iced::Point::new(42.0, 240.0),
                iced::Point::new(42.0, 310.0),
                iced::Point::new(60.0, 310.0),
            ]
        );
    }
}
