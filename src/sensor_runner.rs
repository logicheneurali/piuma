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
        Some((text, value)) => measured(text, source, value),
        None => plain("N/A".to_string()),
    }
}

/// Run a sensor source and return its value (text + optional sample).
pub fn run(source: &SensorSource) -> Read {
    match source {
        SensorSource::Command { command } => {
            plain(match std::process::Command::new("sh")
                .args(["-c", command])
                .output()
            {
                Ok(out) if out.status.success() => {
                    String::from(String::from_utf8_lossy(&out.stdout).trim())
                }
                _ => "ERROR".to_string(),
            })
        }
        SensorSource::EnvVar { env_var } => {
            plain(std::env::var(env_var).unwrap_or("N/A".to_string()))
        }
        SensorSource::StaticText { static_text } => plain(static_text.clone()),
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
        SensorSource::Window {} => plain(sensors::get_window_title()),
        SensorSource::Workspace {} => plain(sensors::get_workspace()),
        // Tray items carry no text: the icons come from the shared tray
        // state (see `tray.rs`), rendered by the view directly.
        SensorSource::Tray {} => plain(String::new()),
        // Newest active notification's summary (see `notifications.rs`).
        SensorSource::Notifications {} => plain(notifications::display_value()),
    }
}
