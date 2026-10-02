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
        SensorSource::NvidiaGpu {} => sensors::gpu_load(),
        SensorSource::ProcCpu {} => sensors::cpu_load().unwrap_or_else(|| "N/A".to_string()),
        SensorSource::ProcMem {} => sensors::mem_usage(),
        SensorSource::ProcDisk {} => sensors::disk_usage(),
        SensorSource::Uptime {} => sensors::uptime(),
        SensorSource::Network { interface } => sensors::network_info(interface),
        SensorSource::Audio { device } => sensors::audio_volume(),
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