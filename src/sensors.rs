//! System sensors - collects various system information.
//!
//! All functions return a formatted string.

use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
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

/// Get audio volume (e.g. `50%`) of a mixer control; empty device = Master.
pub fn audio_volume(device: &str) -> String {
    let device = if device.trim().is_empty() { "Master" } else { device.trim() };
    let output = Command::new("amixer")
        .args(["get", device])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            for line in lines {
                if line.contains("Front Left") {
                    // The percentage lives in a bracketed token, e.g.
                    // "Front Left: Playback 32768 [50%] [on]" → "50%";
                    // the old parts[1] returned "Left:".
                    if let Some(pct) = line.split_whitespace().find(|t| t.contains('%')) {
                        return pct.trim_matches(|c| c == '[' || c == ']').to_string();
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

// ── disk (df) ───────────────────────────────────────────────────────

/// `df -h /` row fields: [device, size, used, avail, use%, mount].
/// Locale-independent: df column headers are localized, positions are not.
fn df_root() -> Option<Vec<String>> {
    let out = Command::new("df").args(["-h", "/"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<String> = stdout.lines().nth(1)?.split_whitespace().map(str::to_string).collect();
    if parts.len() >= 5 { Some(parts) } else { None }
}

/// Get disk usage percentage (e.g. `66%`).
pub fn disk_usage() -> String {
    df_root().map(|p| p[4].clone()).unwrap_or_else(|| "N/A".to_string())
}

/// Get used disk space in human units (e.g. `619G`).
pub fn disk_used() -> String {
    df_root().map(|p| p[2].clone()).unwrap_or_else(|| "N/A".to_string())
}

/// Get total disk size in human units (e.g. `950G`).
pub fn disk_total() -> String {
    df_root().map(|p| p[1].clone()).unwrap_or_else(|| "N/A".to_string())
}

/// Get disk usage as a ratio (e.g. `619G/950G`).
pub fn disk_detail() -> String {
    df_root().map(|p| format!("{}/{}", p[2], p[1])).unwrap_or_else(|| "N/A".to_string())
}

// ── /proc/meminfo (mem & swap) ──────────────────────────────────────

/// Values of interest in /proc/meminfo, all in kB.
struct MemInfo {
    mem_total: u64,
    mem_avail: u64,
    swap_total: u64,
    swap_free: u64,
}

/// Parse /proc/meminfo into the four fields the mem/swap sensors need.
fn meminfo() -> Option<MemInfo> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let field = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    Some(MemInfo {
        mem_total: field("MemTotal:"),
        mem_avail: field("MemAvailable:"),
        swap_total: field("SwapTotal:"),
        swap_free: field("SwapFree:"),
    })
}

/// Get memory usage percentage (e.g. `41%`).
pub fn mem_perc() -> String {
    match meminfo() {
        Some(m) if m.mem_total > 0 => format!("{}%", (m.mem_total - m.mem_avail) * 100 / m.mem_total),
        _ => "N/A".to_string(),
    }
}

/// Get used memory in MB (e.g. `13124 MB`).
pub fn mem_used() -> String {
    match meminfo() {
        Some(m) if m.mem_total > 0 => format!("{} MB", (m.mem_total - m.mem_avail) / 1024),
        _ => "N/A".to_string(),
    }
}

/// Get total memory in MB (e.g. `31808 MB`).
pub fn mem_total() -> String {
    match meminfo() {
        Some(m) if m.mem_total > 0 => format!("{} MB", m.mem_total / 1024),
        _ => "N/A".to_string(),
    }
}

/// Get swap usage percentage (e.g. `12%`).
pub fn swap_perc() -> String {
    match meminfo() {
        Some(m) if m.swap_total > 0 => format!("{}%", (m.swap_total - m.swap_free) * 100 / m.swap_total),
        _ => "N/A".to_string(),
    }
}

/// Get used swap in MB (e.g. `3740 MB`).
pub fn swap_used() -> String {
    match meminfo() {
        Some(m) if m.swap_total > 0 => format!("{} MB", (m.swap_total - m.swap_free) / 1024),
        _ => "N/A".to_string(),
    }
}

/// Get total swap in MB (e.g. `31807 MB`).
pub fn swap_total() -> String {
    match meminfo() {
        Some(m) if m.swap_total > 0 => format!("{} MB", m.swap_total / 1024),
        _ => "N/A".to_string(),
    }
}

/// Get swap usage as a ratio (e.g. `3740/31807 MB`).
pub fn swap_detail() -> String {
    match meminfo() {
        Some(m) if m.swap_total > 0 => format!(
            "{}/{} MB",
            (m.swap_total - m.swap_free) / 1024,
            m.swap_total / 1024
        ),
        _ => "N/A".to_string(),
    }
}

// ── battery / backlight (sysfs) ─────────────────────────────────────

/// Read a sysfs numeric attribute (trailing newline tolerated).
fn read_num(path: impl AsRef<std::path::Path>) -> Option<u64> {
    fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok())
}

/// Battery levels as (now, full, unit, decimals): energy in Wh when the
/// battery exposes energy_*, else charge in mAh.
fn battery_levels() -> Option<(f64, f64, &'static str, usize)> {
    let dir = "/sys/class/power_supply/BAT0";
    let read = |name: &str| read_num(format!("{}/{}", dir, name)).map(|v| v as f64);
    if let (Some(now), Some(full)) = (read("energy_now"), read("energy_full")) {
        Some((now / 1_000_000.0, full / 1_000_000.0, "Wh", 1))
    } else if let (Some(now), Some(full)) = (read("charge_now"), read("charge_full")) {
        Some((now / 1_000.0, full / 1_000.0, "mAh", 0))
    } else {
        None
    }
}

/// Get battery charge percentage (e.g. `80%`).
pub fn battery_perc() -> String {
    match battery_levels() {
        Some((now, full, _, _)) if full > 0.0 => format!("{:.0}%", now * 100.0 / full),
        _ => "N/A".to_string(),
    }
}

/// Get current battery energy/charge (e.g. `64.0 Wh`).
pub fn battery_used() -> String {
    match battery_levels() {
        Some((now, _, unit, decimals)) => format!("{:.*} {}", decimals, now, unit),
        _ => "N/A".to_string(),
    }
}

/// Get full battery energy/charge (e.g. `80.0 Wh`).
pub fn battery_total() -> String {
    match battery_levels() {
        Some((_, full, unit, decimals)) => format!("{:.*} {}", decimals, full, unit),
        _ => "N/A".to_string(),
    }
}

/// Get backlight brightness percentage (e.g. `31%`) from the first
/// backlight device under /sys/class/backlight.
pub fn backlight_perc() -> String {
    let Ok(dir) = fs::read_dir("/sys/class/backlight") else {
        return "N/A".to_string();
    };
    for entry in dir.flatten() {
        let base = entry.path();
        if let (Some(cur), Some(max)) = (read_num(base.join("brightness")), read_num(base.join("max_brightness")))
        {
            if max > 0 {
                return format!("{}%", cur * 100 / max);
            }
        }
    }
    "N/A".to_string()
}

// ── temperature (hwmon / thermal zone) ──────────────────────────────

/// Read `attr` from the hwmon device whose `name` matches (millidegrees).
fn hwmon_temp(dev_name: &str, attr: &str) -> Option<u64> {
    let dir = fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        let name = fs::read_to_string(path.join("name")).unwrap_or_default();
        if name.trim() == dev_name {
            return read_num(path.join(attr));
        }
    }
    None
}

/// Read `temp` from the thermal zone whose `type` matches (millidegrees).
fn thermal_zone_temp(zone_type: &str) -> Option<u64> {
    let dir = fs::read_dir("/sys/class/thermal").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        let kind = fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() == zone_type {
            return read_num(path.join("temp"));
        }
    }
    None
}

/// Get CPU package temperature (e.g. `61°C`): hwmon `coretemp` first
/// (temp1 = package id 0), `x86_pkg_temp` thermal zone as fallback.
pub fn cpu_temp() -> String {
    let millideg = hwmon_temp("coretemp", "temp1_input")
        .or_else(|| thermal_zone_temp("x86_pkg_temp"));
    match millideg {
        Some(t) => format!("{}°C", t / 1000),
        None => "N/A".to_string(),
    }
}

// ── network rates ───────────────────────────────────────────────────

/// Last rx/tx counters per interface with their own timestamps, so the
/// two directions of one interface can be sampled independently.
struct NetSample {
    rx: u64,
    tx: u64,
    rx_at: Instant,
    tx_at: Instant,
}

static NET_LAST: OnceLock<Mutex<HashMap<String, NetSample>>> = OnceLock::new();

/// Current rx/tx byte counters of an interface from /proc/net/dev.
fn net_counters(interface: &str) -> Option<(u64, u64)> {
    let text = fs::read_to_string("/proc/net/dev").ok()?;
    let prefix = format!("{}:", interface);
    let line = text.lines().find(|l| l.trim_start().starts_with(&prefix))?;
    let cols: Vec<&str> = line.split(':').nth(1)?.split_whitespace().collect();
    Some((cols.first()?.parse().ok()?, cols.get(8)?.parse().ok()?))
}

/// Bytes per second for one direction since its previous sample; None on
/// the first call (baseline) or when the counter went backwards.
fn net_rate(interface: &str, up: bool) -> Option<f64> {
    let (rx, tx) = net_counters(interface)?;
    let now = Instant::now();
    let mut last = NET_LAST.get_or_init(Default::default).lock().ok()?;
    let sample = last
        .entry(interface.to_string())
        .or_insert(NetSample { rx, tx, rx_at: now, tx_at: now });
    let (prev, at) = if up { (sample.tx, sample.tx_at) } else { (sample.rx, sample.rx_at) };
    let cur = if up { tx } else { rx };
    if up {
        sample.tx = tx;
        sample.tx_at = now;
    } else {
        sample.rx = rx;
        sample.rx_at = now;
    }
    let dt = now.duration_since(at).as_secs_f64();
    if dt < 0.2 || cur < prev {
        return None;
    }
    Some((cur - prev) as f64 / dt)
}

/// Format a byte rate: `812 B/s`, `1.4 KB/s`, `12.3 MB/s`.
fn fmt_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 {
        format!("{:.0} B/s", bytes_per_sec)
    } else if bytes_per_sec < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    }
}

/// Get network download rate (e.g. `1.4 MB/s`); `N/A` on the first tick.
pub fn net_down(interface: &str) -> String {
    net_rate(interface, false).map(fmt_rate).unwrap_or_else(|| "N/A".to_string())
}

/// Get network upload rate (e.g. `12.3 KB/s`); `N/A` on the first tick.
pub fn net_up(interface: &str) -> String {
    net_rate(interface, true).map(fmt_rate).unwrap_or_else(|| "N/A".to_string())
}