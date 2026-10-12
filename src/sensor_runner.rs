//! Generic sensor runner — executes sensor sources and returns their values.

use crate::config::SensorSource;
use crate::graphs;
use crate::notifications;
use crate::sensors;

/// What one sensor run produced: the text for the bar and — for the
/// graphable sources — the very same reading as a number with its
/// Y-scale policy (see `graphs::scale_of`). One read per tick feeds
/// both consumers: no second pass over /proc, no extra command spawn,
/// and the stateful CPU delta can only be sampled once.
pub struct Read {
    pub text: String,
    pub graph: Option<(f64, graphs::Scale)>,
}

/// A reading without a graph sample.
fn plain(text: String) -> Read {
    Read {
        text,
        graph: None,
    }
}

/// A numeric reading: text for the bar, value + the source's scale
/// policy for the graph (`None` scale = source not graphable).
fn measured(text: String, source: &SensorSource, value: f64) -> Read {
    Read {
        text,
        graph: graphs::scale_of(source).map(|scale| (value, scale)),
    }
}

/// An optional reading rendered as `N/A` when absent.
fn optional(reading: Option<(String, f64)>, source: &SensorSource) -> Read {
    match reading {
        Some((text, value)) => {
            // perc_ — the bar text carries the `%` symbol (an answer
            // that already ends in it is not doubled); the number that
            // feeds the graph keeps its bare value.
            let text = if is_percent(source) && !text.ends_with('%') {
                format!("{text}%")
            } else {
                text
            };
            measured(text, source, value)
        }
        None => plain("N/A".to_string()),
    }
}

/// Whether a source's reading is a percentage: the family whose bar
/// text carries the `%` symbol.
fn is_percent(source: &SensorSource) -> bool {
    matches!(
        source,
        SensorSource::PercCpu {}
            | SensorSource::PercGpu {}
            | SensorSource::PercMem {}
            | SensorSource::PercSwap {}
            | SensorSource::PercDisk {}
            | SensorSource::PercBattery {}
            | SensorSource::PercBacklight {}
            | SensorSource::PercVolume { .. }
    )
}

/// A numeric sample out of a text answer (the user-authored sources):
/// the WHOLE trimmed answer must be a finite number, optionally
/// followed by one digit-free suffix ("55%", "49°C", "1.9 GB"). A
/// timestamp, an IP, a version, a locale decimal, "N/A" or NaN/inf
/// parse to nothing — a wrong point would ride the Auto axis for the
/// whole window, a refused one just repeats the previous value.
fn strict_number(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Ok(value) = text.parse::<f64>() {
        return value.is_finite().then_some(value);
    }
    // The first char outside the number grammar ends the number: the
    // head must parse as finite and the suffix must carry no digit, so
    // "16:45:30" (suffix holds digits) is refused while "55%" and
    // "49°C" pass.
    let cut =
        text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E')))?;
    let (head, tail) = text.split_at(cut);
    if tail.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    head.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A text reading that may carry the graph's sample: the graph takes
/// the number only when the whole answer is one (`strict_number`),
/// anything else stays plain text with no graph.
fn maybe_measured(text: String, source: &SensorSource) -> Read {
    match strict_number(&text) {
        Some(value) => measured(text, source, value),
        None => plain(text),
    }
}

/// Run a sensor source and return its value (text + optional sample).
pub fn run(source: &SensorSource) -> Read {
    match source {
        // The user-authored text sources: the answer feeds the graph
        // sample when the whole thing is a number (see `strict_number`).
        SensorSource::Command { command } => maybe_measured(
            match std::process::Command::new("sh")
                .args(["-c", command])
                .output()
            {
                Ok(out) if out.status.success() => {
                    String::from(String::from_utf8_lossy(&out.stdout).trim())
                }
                _ => "ERROR".to_string(),
            },
            source,
        ),
        SensorSource::EnvVar { env_var } => {
            maybe_measured(std::env::var(env_var).unwrap_or("N/A".to_string()), source)
        }
        SensorSource::StaticText { static_text } => maybe_measured(static_text.clone(), source),
        // perc_ — percentage values
        SensorSource::PercCpu {} => optional(sensors::cpu_load(), source),
        SensorSource::PercGpu {} => optional(sensors::gpu_load(), source),
        SensorSource::PercMem {} => optional(sensors::mem_perc(), source),
        SensorSource::PercSwap {} => optional(sensors::swap_perc(), source),
        SensorSource::PercDisk {} => optional(sensors::disk_usage(), source),
        SensorSource::PercBattery {} => optional(sensors::battery_perc(), source),
        SensorSource::PercBacklight {} => optional(sensors::backlight_perc(), source),
        SensorSource::PercVolume { device } => optional(sensors::audio_volume(device), source),
        // used_ — numeric consumption (graphs plot percent-of-total)
        SensorSource::UsedMem {} => optional(sensors::mem_used(), source),
        SensorSource::UsedSwap {} => optional(sensors::swap_used(), source),
        SensorSource::UsedDisk {} => optional(sensors::disk_used(), source),
        SensorSource::UsedBattery {} => optional(sensors::battery_used(), source),
        SensorSource::UsedNetdown { interface } => optional(sensors::net_down(interface), source),
        SensorSource::UsedNetup { interface } => optional(sensors::net_up(interface), source),
        // total_ — full values (no numeric category to graph)
        SensorSource::TotalMem {} => plain(sensors::mem_total()),
        SensorSource::TotalSwap {} => plain(sensors::swap_total()),
        SensorSource::TotalDisk {} => plain(sensors::disk_total()),
        SensorSource::TotalBattery {} => plain(sensors::battery_total()),
        // detail_ — used/total ratios
        SensorSource::DetailMem {} => plain(sensors::mem_usage()),
        SensorSource::DetailSwap {} => plain(sensors::swap_detail()),
        SensorSource::DetailDisk {} => plain(sensors::disk_detail()),
        // temp_ — temperatures
        SensorSource::TempCpu {} => optional(sensors::cpu_temp(), source),
        // everything else (no numeric category)
        SensorSource::Uptime {} => plain(sensors::uptime()),
        SensorSource::NetState { interface } => plain(sensors::network_info(interface)),
        SensorSource::Clock { format } => {
            plain(match format.as_ref().map(|f| f.as_str()) {
                Some("date") => sensors::get_date(),
                _ => sensors::get_time(),
            })
        }
        // The calendar sensor's bar text: a full strftime format (its
        // hover popup lives in main.rs, see `calendar_hover_sync`).
        SensorSource::Calendar { format } => plain(sensors::clock_formatted(
            format.as_deref().unwrap_or("%H:%M:%S"),
        )),
        SensorSource::Window {} => plain(sensors::get_window_title()),
        SensorSource::Workspace {} => plain(sensors::get_workspace()),
        // Tray items carry no text: the icons come from the shared tray
        // state (see `tray.rs`), rendered by the view directly.
        SensorSource::Tray {} => plain(String::new()),
        // Newest active notification's summary (see `notifications.rs`).
        SensorSource::Notifications {} => plain(notifications::display_value()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// perc_ readings carry the `%` symbol in their bar text; the
    /// number that feeds the graph keeps its bare value, other sources
    /// and the `N/A` fallback don't change, and an answer that already
    /// ends in `%` is not doubled.
    #[test]
    fn percent_readings_carry_the_symbol() {
        let cpu = optional(Some(("5".into(), 5.5)), &SensorSource::PercCpu {});
        assert_eq!(cpu.text, "5%");
        assert_eq!(cpu.graph.map(|(v, _)| v), Some(5.5), "the sample stays bare");

        let mem = optional(Some(("5".into(), 5.0)), &SensorSource::UsedMem {});
        assert_eq!(mem.text, "5");

        assert_eq!(optional(None, &SensorSource::PercMem {}).text, "N/A");

        let vol = optional(
            Some(("70%".into(), 70.0)),
            &SensorSource::PercVolume {
                device: String::new(),
            },
        );
        assert_eq!(vol.text, "70%", "already suffixed: no doubling");
    }

    /// The graph sample of a text source is the whole answer as a
    /// finite number: clean numbers and single-suffix ones pass,
    /// ambiguous or non-numeric answers record nothing — and the bar
    /// text never changes either way.
    #[test]
    fn text_sources_graph_only_whole_numbers() {
        for (text, expected) in [
            ("42", Some(42.0)),
            (" 42  ", Some(42.0)),
            ("-3.5", Some(-3.5)),
            ("1e3", Some(1000.0)),
            ("55%", Some(55.0)),
            ("49°C", Some(49.0)),
            ("1.9 GB", Some(1.9)),
            ("16:45:30", None),
            ("192.168.1.1", None),
            ("1.2.3", None),
            ("1,5", None),
            ("N/A", None),
            ("ERROR", None),
            ("nan", None),
            ("inf", None),
            ("3 g 4", None),
            ("exit code 5", None),
        ] {
            assert_eq!(strict_number(text), expected, "{text:?}");
        }

        let source = SensorSource::Command {
            command: String::new(),
        };
        let read = maybe_measured("49°C".into(), &source);
        assert_eq!(read.graph.map(|(v, _)| v), Some(49.0));
        assert_eq!(read.text, "49°C", "the bar text stays as-is");
        let read = maybe_measured("hello".into(), &source);
        assert!(read.graph.is_none());
        assert_eq!(read.text, "hello");
    }
}
