//! System sensors - collects various system information.
//!
//! All functions return a formatted string.

use std::fs;
use std::process::Command;
use serde_json;

/// Get CPU load percentage.
pub fn cpu_load() -> Option<String> {
    let output = Command::new("sh")
        .args(["-c", "top -bn1 | grep 'Cpu(s)' | awk '{print $2}' | sed 's/%us,//'"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(stdout.trim().to_string())
}

/// Get GPU load percentage (NVIDIA only).
pub fn gpu_load() -> String {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim().to_string()
        }
        _ => "N/A".to_string(),
    }
}

/// Get memory usage in MB.
pub fn mem_usage() -> String {
    let output = Command::new("free")
        .args(["-m"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            if lines.len() > 1 {
                let parts: Vec<&str> = lines[1].split_whitespace().collect();
                if parts.len() >= 3 {
                    format!("{}/{} MB", parts[2], parts[1])
                } else {
                    "N/A".to_string()
                }
            } else {
                "N/A".to_string()
            }
        }
        _ => "N/A".to_string(),
    }
}

/// Get disk usage.
pub fn disk_usage() -> String {
    let output = Command::new("df")
        .args(["-h", "/"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            if lines.len() > 1 {
                let parts: Vec<&str> = lines[1].split_whitespace().collect();
                if parts.len() >= 5 {
                    parts[4].to_string()
                } else {
                    "N/A".to_string()
                }
            } else {
                "N/A".to_string()
            }
        }
        _ => "N/A".to_string(),
    }
}

/// Get system uptime.
pub fn uptime() -> String {
    let output = Command::new("uptime")
        .args(["-p"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim().to_string()
        }
        _ => "N/A".to_string(),
    }
}

/// Get network information for a specific interface.
pub fn network_info(interface: &str) -> String {
    let output = Command::new("ip")
        .args(["addr", "show", interface])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("UP") {
                "connected".to_string()
            } else {
                "disconnected".to_string()
            }
        }
        _ => "N/A".to_string(),
    }
}

/// Get audio volume.
pub fn audio_volume() -> String {
    let output = Command::new("amixer")
        .args(["get", "Master"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            for line in lines {
                if line.contains("Front Left") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        return parts[1].to_string();
                    }
                }
            }
            "N/A".to_string()
        }
        _ => "N/A".to_string(),
    }
}

/// Get current time.
pub fn get_time() -> String {
    let output = Command::new("date")
        .args(["+%H:%M:%S"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim().to_string()
        }
        _ => "N/A".to_string(),
    }
}

/// Get current date.
pub fn get_date() -> String {
    let output = Command::new("date")
        .args(["+%Y-%m-%d"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim().to_string()
        }
        _ => "N/A".to_string(),
    }
}

/// Get the current focused window title (Hyprland specific).
pub fn get_window_title() -> String {
    let output = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Parse JSON to extract window title
            // Example output: {"id":1234,"title":"Firefox","class":"firefox","initialClass":"firefox","workspace":{"id":1,"name":"1"}}
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if let Some(title) = json.get("title") {
                    return title.as_str().unwrap_or("No Title").to_string();
                }
            }
            "No Title".to_string()
        }
        _ => "No Focus".to_string(),
    }
}

/// Get the current workspace (Hyprland specific).
pub fn get_workspace() -> String {
    let output = Command::new("hyprctl")
        .args(["activeworkspace", "-j"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Parse JSON to extract workspace info
            // Example output: {"id":1,"name":"1","monitor":"eDP-1","windows":1,"lastwindow":"Firefox","lastwindowtitle":"Firefox"}
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if let Some(name) = json.get("name") {
                    return name.as_str().unwrap_or("1").to_string();
                }
            }
            "1".to_string()
        }
        _ => "1".to_string(),
    }
}