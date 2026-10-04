//! Generic sensor runner — executes sensor sources and returns their values.

use crate::config::SensorSource;
use crate::sensors;

/// Run a sensor source and return its formatted value.
pub fn run(source: &SensorSource) -> String {
    match source {
        SensorSource::Command { command } => {
            match std::process::Command::new("sh")
                .args(["-c", command])
                .output()
            {
                Ok(out) if out.status.success() => {
                    String::from(String::from_utf8_lossy(&out.stdout).trim())
                }
                _ => "ERROR".to_string(),
            }
        }
        SensorSource::EnvVar { env_var } => {
            std::env::var(env_var).unwrap_or("N/A".to_string())
        }
        SensorSource::StaticText { static_text } => static_text.clone(),
        // perc_ — percentage values
        SensorSource::PercCpu {} => sensors::cpu_load().unwrap_or_else(|| "N/A".to_string()),
        SensorSource::PercGpu {} => sensors::gpu_load(),
        SensorSource::PercMem {} => sensors::mem_perc(),
        SensorSource::PercSwap {} => sensors::swap_perc(),
        SensorSource::PercDisk {} => sensors::disk_usage(),
        SensorSource::PercBattery {} => sensors::battery_perc(),
        SensorSource::PercBacklight {} => sensors::backlight_perc(),
        SensorSource::PercVolume { device } => sensors::audio_volume(device),
        // used_ — numeric consumption
        SensorSource::UsedMem {} => sensors::mem_used(),
        SensorSource::UsedSwap {} => sensors::swap_used(),
        SensorSource::UsedDisk {} => sensors::disk_used(),
        SensorSource::UsedBattery {} => sensors::battery_used(),
        SensorSource::UsedNetdown { interface } => sensors::net_down(interface),
        SensorSource::UsedNetup { interface } => sensors::net_up(interface),
        // total_ — full values
        SensorSource::TotalMem {} => sensors::mem_total(),
        SensorSource::TotalSwap {} => sensors::swap_total(),
        SensorSource::TotalDisk {} => sensors::disk_total(),
        SensorSource::TotalBattery {} => sensors::battery_total(),
        // detail_ — used/total ratios
        SensorSource::DetailMem {} => sensors::mem_usage(),
        SensorSource::DetailSwap {} => sensors::swap_detail(),
        SensorSource::DetailDisk {} => sensors::disk_detail(),
        // temp_ — temperatures
        SensorSource::TempCpu {} => sensors::cpu_temp(),
        // everything else (no numeric category)
        SensorSource::Uptime {} => sensors::uptime(),
        SensorSource::NetState { interface } => sensors::network_info(interface),
        SensorSource::Clock { format } => {
            match format.as_ref().map(|f| f.as_str()) {
                Some("date") => sensors::get_date(),
                _ => sensors::get_time(),
            }
        }
        SensorSource::Window {} => sensors::get_window_title(),
        SensorSource::Workspace {} => sensors::get_workspace(),
    }
}