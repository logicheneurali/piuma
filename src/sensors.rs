//! System sensors - collects various system information.
//!
//! All functions return a formatted string.

use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use serde_json;

// ── CPU / GPU ───────────────────────────────────────────────────────

/// Last aggregate /proc/stat sample for the CPU delta: (total, idle, at).
static CPU_LAST: OnceLock<Mutex<Option<(u64, u64, Instant)>>> = OnceLock::new();

/// Read the aggregate `cpu` line of /proc/stat as (total, idle) jiffies.
/// idle includes iowait; guest fields are skipped (already in user).
fn proc_stat_cpu() -> Option<(u64, u64)> {
    let text = fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line.split_whitespace().skip(1).take(8)
        .filter_map(|v| v.parse().ok())
        .collect();
    let total = fields.iter().sum();
    let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

/// Get CPU load percentage between two /proc/stat samples (e.g. `4.8`).
/// No external commands; None on the first tick, rendered as N/A.
pub fn cpu_load() -> Option<String> {
    let (total, idle) = proc_stat_cpu()?;
    let now = Instant::now();
    let mut last = CPU_LAST.get_or_init(Default::default).lock().ok()?;
    let prev = *last;
    *last = Some((total, idle, now));
    let (prev_total, prev_idle, prev_at) = prev?;
    let dt = now.duration_since(prev_at).as_secs_f64();
    let d_total = total.saturating_sub(prev_total);
    let d_idle = idle.saturating_sub(prev_idle);
    if dt < 0.2 || d_total == 0 {
        return None;
    }
    Some(format!("{:.1}", (d_total - d_idle.min(d_total)) as f64 * 100.0 / d_total as f64))
}

/// GPU busy percentage from the amdgpu hwmon sysfs (AMD), if present.
fn amdgpu_load() -> Option<String> {
    let cards = fs::read_dir("/sys/class/drm").ok()?;
    for card in cards.flatten() {
        let name = card.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("card") || name.contains('-') {
            continue; // card0, card1… not connector entries like card0-DP-1
        }
        if let Ok(hwmons) = fs::read_dir(card.path().join("device").join("hwmon")) {
            for hwmon in hwmons.flatten() {
                if let Some(v) = read_num(hwmon.path().join("gpu_busy_percent")) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Get GPU load percentage (e.g. `12`): NVIDIA via nvidia-smi, then the
/// amdgpu sysfs counter (AMD); N/A elsewhere (e.g. Intel has no standard
/// interface). No hard dependency on the NVIDIA driver.
pub fn gpu_load() -> String {
    if let Ok(out) = Command::new("nvidia-smi")
        .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
        .output()
    {
        if out.status.success() {
            let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    amdgpu_load().unwrap_or_else(|| "N/A".to_string())
}

/// Get system uptime (e.g. `up 3 hours, 0 minute`) from /proc/uptime,
/// matching the wording of procps `uptime -p` without needing procps.
pub fn uptime() -> String {
    let text = match fs::read_to_string("/proc/uptime") {
        Ok(text) => text,
        Err(_) => return "N/A".to_string(),
    };
    let Some(secs) = text.split('.').next().and_then(|v| v.parse::<u64>().ok()) else {
        return "N/A".to_string();
    };
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3_600, (secs % 3_600) / 60);
    // procps prints days/hours only when non-zero, minutes always, and a
    // singular unit for 0 or 1: "up 3 hours, 0 minute".
    let plural = |n: u64| if n > 1 { "s" } else { "" };
    let mut out = String::from("up ");
    if d > 0 {
        out.push_str(&format!("{} day{}, ", d, plural(d)));
    }
    if h > 0 {
        out.push_str(&format!("{} hour{}, ", h, plural(h)));
    }
    out.push_str(&format!("{} minute{}", m, plural(m)));
    out
}

/// Get network state for an interface (`connected`/`disconnected`) from
/// sysfs — no iproute2 needed. N/A when the interface does not exist.
pub fn network_info(interface: &str) -> String {
    let Ok(state) = fs::read_to_string(format!("/sys/class/net/{}/operstate", interface)) else {
        return "N/A".to_string();
    };
    let state = state.trim();
    if state == "up" {
        return "connected".to_string();
    }
    if state == "unknown" {
        // Devices like loopback report operstate "unknown": trust the
        // IFF_UP bit (0x1) of /sys/class/net/<if>/flags instead.
        // flags is hexadecimal ("0x9"), so it needs its own parser —
        // read_num only accepts decimal.
        if let Ok(flags) = fs::read_to_string(format!("/sys/class/net/{}/flags", interface)) {
            let flags = flags.trim().trim_start_matches("0x");
            if let Ok(flags) = u64::from_str_radix(flags, 16) {
                if flags & 1 == 1 {
                    return "connected".to_string();
                }
            }
        }
    }
    "disconnected".to_string()
}

/// Audio backend, tried in this order; the first one that returns a
/// value is cached and reused on the following ticks.
#[derive(Clone, Copy, PartialEq)]
enum VolumeBackend {
    Amixer,  // ALSA (alsa-utils)
    Wpctl,   // PipeWire (wireplumber)
    Pactl,   // PulseAudio (pulseaudio-utils)
}

/// Backend chosen at runtime once one proved to work.
static VOLUME_BACKEND: OnceLock<Mutex<Option<VolumeBackend>>> = OnceLock::new();

impl VolumeBackend {
    /// Run this backend's command and parse its percentage (`50%`).
    fn read(self, device: &str) -> String {
        let output = match self {
            VolumeBackend::Amixer => Command::new("amixer").args(["get", device]).output(),
            VolumeBackend::Wpctl => Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output(),
            VolumeBackend::Pactl => Command::new("pactl").args(["get-sink-volume", "@DEFAULT_AUDIO_SINK@"]).output(),
        };
        let Ok(out) = output else { return "N/A".to_string(); };
        if !out.status.success() {
            return "N/A".to_string(); // command missing or server absent
        }
        let text = String::from_utf8_lossy(&out.stdout);
        match self {
            VolumeBackend::Amixer => {
                // A playback line carries the percentage: stereo
                // "Front Left: Playback 32768 [50%] [on]" or mono
                // "Mono: Playback 32768 [50%] [on]".
                for line in text.lines() {
                    if line.contains('%') && (line.contains("Front Left") || line.contains("Mono")) {
                        if let Some(pct) = line.split_whitespace().find(|t| t.contains('%')) {
                            return pct.trim_matches(|c| c == '[' || c == ']').to_string();
                        }
                    }
                }
                "N/A".to_string()
            }
            VolumeBackend::Wpctl => {
                // "Volume: 0.50 [MUTED]" → "50%"
                text.split_whitespace()
                    .find_map(|t| t.parse::<f64>().ok())
                    .map(|v| format!("{:.0}%", v * 100.0))
                    .unwrap_or_else(|| "N/A".to_string())
            }
            VolumeBackend::Pactl => {
                // "front-left: 32768 / 50% / -18.06 dB, …" → "50%"
                text.split_whitespace()
                    .find(|t| t.contains('%'))
                    .map(|t| t.trim_matches(|c: char| !c.is_ascii_digit() && c != '%').to_string())
                    .filter(|t| t != "%")
                    .unwrap_or_else(|| "N/A".to_string())
            }
        }
    }

    /// Set the volume to `pct` percent (0..=100) through this backend;
    /// returns true when the command ran successfully.
    fn set(self, device: &str, pct: f32) -> bool {
        let result = match self {
            // amixer sets a percentage on the control name (e.g. Master).
            VolumeBackend::Amixer => Command::new("amixer")
                .args(["set", device, &format!("{pct:.0}%")])
                .status(),
            // wpctl takes a fraction of full scale on the default sink.
            VolumeBackend::Wpctl => Command::new("wpctl")
                .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{:.3}", pct / 100.0)])
                .status(),
            // pactl takes a percentage on the default sink.
            VolumeBackend::Pactl => Command::new("pactl")
                .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{pct:.0}%")])
                .status(),
        };
        matches!(result, Ok(status) if status.success())
    }
}

/// Percentage step applied per wheel notch on a `perc_volume` sensor.
const VOLUME_STEP: f32 = 5.0;

/// Normalize the `device` parameter: empty = Master (amixer control name).
fn volume_device(device: &str) -> &str {
    let device = device.trim();
    if device.is_empty() { "Master" } else { device }
}

/// Backend order for the next operation: the cached backend first (it
/// proved to work), then the fixed probe order amixer → wpctl → pactl.
fn backend_order() -> Vec<VolumeBackend> {
    let mut order = vec![VolumeBackend::Amixer, VolumeBackend::Wpctl, VolumeBackend::Pactl];
    if let Ok(guard) = VOLUME_BACKEND.get_or_init(Default::default).lock() {
        if let Some(cached) = *guard {
            order.retain(|b| *b != cached);
            order.insert(0, cached);
        }
    }
    order
}

/// Remember the backend that worked so the next call tries it first.
fn cache_backend(backend: VolumeBackend) {
    if let Ok(mut guard) = VOLUME_BACKEND.get_or_init(Default::default).lock() {
        *guard = Some(backend);
    }
}

/// Parse a `50%`-style value; `N/A` and anything unparsable → None.
fn parse_percent(value: &str) -> Option<f32> {
    value.trim().trim_end_matches('%').parse::<f32>().ok()
}

/// Get audio volume (e.g. `50%`): tries amixer → wpctl → pactl (ALSA,
/// PipeWire, PulseAudio) and remembers the first that works; empty
/// device = Master (amixer control name; the other backends use the
/// default sink).
pub fn audio_volume(device: &str) -> String {
    let device = volume_device(device);
    for backend in backend_order() {
        let value = backend.read(device);
        if value != "N/A" {
            cache_backend(backend);
            return value;
        }
    }
    "N/A".to_string()
}

/// Serializes the read-modify-write of `adjust_volume` so concurrent
/// wheel events can't read the same value and lose updates.
static VOLUME_ADJUST_LOCK: Mutex<()> = Mutex::new(());

/// Volume after one wheel notch: current ± `VOLUME_STEP` clamped to
/// 0..=100; None when the current value isn't a percentage. Shared by
/// the backend adjust and the immediate display update.
pub fn next_volume(current: &str, up: bool) -> Option<f32> {
    let current = parse_percent(current)?;
    Some((current + if up { VOLUME_STEP } else { -VOLUME_STEP }).clamp(0.0, 100.0))
}

/// Raise/lower the volume by `VOLUME_STEP` (wheel over a `perc_volume`
/// sensor): the current value is read with the working backend, the
/// target clamped to 0..=100 and set through the same backend — the
/// next ones are tried when the set itself fails. No backend at all is
/// reported instead of swallowed.
pub fn adjust_volume(device: &str, up: bool) {
    // Poison-tolerant: a panic in one adjustment must not disable the wheel.
    let _guard = VOLUME_ADJUST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let device = volume_device(device);
    for backend in backend_order() {
        let Some(target) = next_volume(&backend.read(device), up) else { continue };
        if backend.set(device, target) {
            cache_backend(backend);
            return;
        }
    }
    eprintln!("[piuma] volume adjust failed: no working backend (amixer/wpctl/pactl)");
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

/// Get memory usage as a ratio (e.g. `14353/31807 MB`), from the same
/// /proc/meminfo parser as the other mem sensors (no `free` needed).
pub fn mem_usage() -> String {
    match meminfo() {
        Some(m) if m.mem_total > 0 => format!(
            "{}/{} MB",
            (m.mem_total - m.mem_avail) / 1024,
            m.mem_total / 1024
        ),
        _ => "N/A".to_string(),
    }
}

// ── battery / backlight (sysfs) ─────────────────────────────────────

/// Read a sysfs numeric attribute (trailing newline tolerated).
fn read_num(path: impl AsRef<std::path::Path>) -> Option<u64> {
    fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok())
}

/// Directory of the first installed battery (`type` == "Battery"):
/// names vary between machines (BAT0, BAT1, …) so they are discovered,
/// not hardcoded.
fn battery_dir() -> Option<String> {
    let dir = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        let kind = fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() == "Battery" {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

/// Battery levels as (now, full, unit, decimals): energy in Wh when the
/// battery exposes energy_*, else charge in mAh.
fn battery_levels() -> Option<(f64, f64, &'static str, usize)> {
    let dir = battery_dir()?;
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

/// Get CPU temperature (e.g. `61°C`): hwmon first — Intel `coretemp`,
/// AMD `k10temp` or `zenpower` (temp1 = Tctl/package) — then the
/// `x86_pkg_temp` thermal zone; N/A when none of them exists.
pub fn cpu_temp() -> String {
    for dev in ["coretemp", "k10temp", "zenpower"] {
        if let Some(t) = hwmon_temp(dev, "temp1_input") {
            return format!("{}°C", t / 1000);
        }
    }
    if let Some(t) = thermal_zone_temp("x86_pkg_temp") {
        return format!("{}°C", t / 1000);
    }
    "N/A".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Wheel math: ±5 clamped to the 0..=100 range.
    #[test]
    fn volume_step_is_clamped() {
        assert_eq!((50.0_f32 + VOLUME_STEP).clamp(0.0, 100.0), 55.0);
        assert_eq!((98.0_f32 + VOLUME_STEP).clamp(0.0, 100.0), 100.0);
        assert_eq!((2.0_f32 - VOLUME_STEP).clamp(0.0, 100.0), 0.0);
    }

    /// Values read from the three backends parse; `N/A` doesn't.
    #[test]
    fn percent_values_parse() {
        assert_eq!(parse_percent("50%"), Some(50.0));
        assert_eq!(parse_percent("0%"), Some(0.0));
        assert_eq!(parse_percent("N/A"), None);
    }

    /// Empty device falls back to the amixer `Master` control.
    #[test]
    fn volume_device_defaults_to_master() {
        assert_eq!(volume_device(""), "Master");
        assert_eq!(volume_device(" Speaker "), "Speaker");
    }
}