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

/// Get CPU load percentage between two /proc/stat samples as (text,
/// value) (e.g. `5`): the one sampling call feeds both the bar and
/// the graph engine — the delta is stateful and can only be consumed
/// once per tick. No external commands; None on the first tick. The
/// text rounds to a whole percent like every `perc_*` sensor; the
/// plotted value keeps its precision.
pub fn cpu_load() -> Option<(String, f64)> {
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
    let load = (d_total - d_idle.min(d_total)) as f64 * 100.0 / d_total as f64;
    Some((format!("{load:.0}"), load))
}

/// GPU busy percentage from the amdgpu hwmon sysfs (AMD), if present.
fn amdgpu_load() -> Option<f64> {
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
                    return Some(v as f64);
                }
            }
        }
    }
    None
}

/// Get GPU load percentage (e.g. `12`) as (text, value): NVIDIA via
/// nvidia-smi, then the amdgpu sysfs counter (AMD); None elsewhere
/// (e.g. Intel has no standard interface). No hard dependency on the
/// NVIDIA driver.
pub fn gpu_load() -> Option<(String, f64)> {
    if let Ok(out) = Command::new("nvidia-smi")
        .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
        .output()
    {
        if out.status.success() {
            let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !value.is_empty() {
                // A non-empty answer ends the search (no AMD fallback),
                // as before; only a parseable one is a reading.
                return value.parse::<f64>().ok().map(|perc| (value, perc));
            }
        }
    }
    amdgpu_load().map(|perc| (format!("{perc}"), perc))
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
    /// Run this backend's command and parse its percentage from the
    /// output (the commands print `50%`); the returned value is bare —
    /// like every other `perc_*` sensor (roadmap2 #5).
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
                            // "[50%]" → bare "50" (roadmap2 #5).
                            return pct
                                .trim_matches(|c| c == '[' || c == ']')
                                .trim_end_matches('%')
                                .to_string();
                        }
                    }
                }
                "N/A".to_string()
            }
            VolumeBackend::Wpctl => {
                // "Volume: 0.50 [MUTED]" → "50"
                text.split_whitespace()
                    .find_map(|t| t.parse::<f64>().ok())
                    .map(|v| format!("{:.0}", v * 100.0))
                    .unwrap_or_else(|| "N/A".to_string())
            }
            VolumeBackend::Pactl => {
                // "front-left: 32768 / 50% / -18.06 dB, …" → "50"
                text.split_whitespace()
                    .find(|t| t.contains('%'))
                    .map(|t| t.trim_matches(|c: char| !c.is_ascii_digit()).to_string())
                    .filter(|t| !t.is_empty())
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

/// Parse a bare or `50%`-style value (the bar shows bare values since
/// roadmap2 #5, while command outputs and old configs may carry the
/// suffix); `N/A` and anything unparsable → None.
fn parse_percent(value: &str) -> Option<f32> {
    value.trim().trim_end_matches('%').parse::<f32>().ok()
}

/// Get audio volume as (text, percent value) (e.g. `50`): tries
/// amixer → wpctl → pactl (ALSA, PipeWire, PulseAudio) and remembers
/// the first that works; empty device = Master (amixer control name;
/// the other backends use the default sink). The backends hand back
/// the bare percent already, so the graph value is the same string
/// parsed — a non-numeric answer counts as no reading (N/A).
pub fn audio_volume(device: &str) -> Option<(String, f64)> {
    let device = volume_device(device);
    for backend in backend_order() {
        let value = backend.read(device);
        if value != "N/A" {
            cache_backend(backend);
            return value.parse::<f64>().ok().map(|perc| (value, perc));
        }
    }
    None
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
/// A `date` reading for an arbitrary strftime format: the `calendar`
/// sensor's bar text (and the fixed readings below). The format is one
/// argv — never a shell — an optional leading `+` is accepted, and a
/// failing `date` shows `N/A` like every other reading.
pub fn clock_formatted(format: &str) -> String {
    let fmt = format.strip_prefix('+').unwrap_or(format);
    let output = Command::new("date").arg(format!("+{fmt}")).output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim().to_string()
        }
        _ => "N/A".to_string(),
    }
}

pub fn get_time() -> String {
    clock_formatted("%H:%M:%S")
}

/// Get current date.
pub fn get_date() -> String {
    clock_formatted("%Y-%m-%d")
}

/// Longest window title the sensor shows (roadmap M1): longer text is cut
/// at a char boundary (multibyte-safe) and marked with a trailing "...".
const WINDOW_TITLE_MAX: usize = 80;

/// Cut `title` to [`WINDOW_TITLE_MAX`] chars, appending "..." when cut.
fn truncate_title(title: &str) -> String {
    if title.chars().count() <= WINDOW_TITLE_MAX {
        return title.to_string();
    }
    let cut: String = title.chars().take(WINDOW_TITLE_MAX).collect();
    format!("{cut}...")
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
                    return truncate_title(title.as_str().unwrap_or("No Title"));
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

/// Get disk usage percentage as (text, bare value) (e.g. `66`); df's
/// own `66%` column loses the suffix, roadmap2 #5. One `df` spawn
/// feeds both the bar and the graph engine.
pub fn disk_usage() -> Option<(String, f64)> {
    df_root().and_then(|p| {
        let bare = p[4].trim_end_matches('%').to_string();
        let perc = bare.parse::<f64>().ok()?;
        Some((bare, perc))
    })
}

/// Get used disk space as (text, percent of total) (e.g. `619G`): the
/// graph plots df's percent from the same spawn.
pub fn disk_used() -> Option<(String, f64)> {
    df_root().and_then(|p| {
        let perc = p[4].trim_end_matches('%').parse::<f64>().ok()?;
        Some((p[2].clone(), perc))
    })
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

/// Get memory usage percentage as (text, percent value) (e.g. `41`):
/// one /proc/meminfo read feeds both the bar and the graph engine.
pub fn mem_perc() -> Option<(String, f64)> {
    meminfo().filter(|m| m.mem_total > 0).map(|m| {
        let used = m.mem_total - m.mem_avail;
        (
            format!("{}", used * 100 / m.mem_total),
            used as f64 * 100.0 / m.mem_total as f64,
        )
    })
}

/// Get used memory as (text, percent of total) (e.g. `13124 MB`): the
/// graph plots the share of the installed RAM from the same read.
pub fn mem_used() -> Option<(String, f64)> {
    meminfo().filter(|m| m.mem_total > 0).map(|m| {
        let used = m.mem_total - m.mem_avail;
        (
            format!("{} MB", used / 1024),
            used as f64 * 100.0 / m.mem_total as f64,
        )
    })
}

/// Get total memory in MB (e.g. `31808 MB`).
pub fn mem_total() -> String {
    match meminfo() {
        Some(m) if m.mem_total > 0 => format!("{} MB", m.mem_total / 1024),
        _ => "N/A".to_string(),
    }
}

/// Get swap usage percentage as (text, percent value) (e.g. `12`):
/// one /proc/meminfo read feeds both the bar and the graph engine.
pub fn swap_perc() -> Option<(String, f64)> {
    meminfo().filter(|m| m.swap_total > 0).map(|m| {
        let used = m.swap_total - m.swap_free;
        (
            format!("{}", used * 100 / m.swap_total),
            used as f64 * 100.0 / m.swap_total as f64,
        )
    })
}

/// Get used swap as (text, percent of total) (e.g. `3740 MB`): the
/// graph plots the share of the swap from the same read.
pub fn swap_used() -> Option<(String, f64)> {
    meminfo().filter(|m| m.swap_total > 0).map(|m| {
        let used = m.swap_total - m.swap_free;
        (
            format!("{} MB", used / 1024),
            used as f64 * 100.0 / m.swap_total as f64,
        )
    })
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

/// Get battery charge as (text, percent value) (e.g. `80`): one sysfs
/// read feeds both the bar and the graph engine.
pub fn battery_perc() -> Option<(String, f64)> {
    match battery_levels() {
        Some((now, full, _, _)) if full > 0.0 => {
            let perc = now * 100.0 / full;
            Some((format!("{perc:.0}"), perc))
        }
        _ => None,
    }
}

/// Get current battery energy/charge as (text, percent of full)
/// (e.g. `64.0 Wh`); the graph needs a full > 0 (a battery that can't
/// report its capacity has no share to plot, `battery_perc`'s rule).
pub fn battery_used() -> Option<(String, f64)> {
    match battery_levels() {
        Some((now, full, unit, decimals)) if full > 0.0 => Some((
            format!("{:.*} {}", decimals, now, unit),
            now * 100.0 / full,
        )),
        _ => None,
    }
}

/// Get full battery energy/charge (e.g. `80.0 Wh`).
pub fn battery_total() -> String {
    match battery_levels() {
        Some((_, full, unit, decimals)) => format!("{:.*} {}", decimals, full, unit),
        _ => "N/A".to_string(),
    }
}

/// Get backlight brightness as (text, percent value) (e.g. `31`,
/// roadmap2 #5) from the first backlight device under
/// /sys/class/backlight: one read feeds the bar and the graph engine.
pub fn backlight_perc() -> Option<(String, f64)> {
    let dir = fs::read_dir("/sys/class/backlight").ok()?;
    for entry in dir.flatten() {
        let base = entry.path();
        if let (Some(cur), Some(max)) = (read_num(base.join("brightness")), read_num(base.join("max_brightness")))
        {
            if max > 0 {
                return Some((
                    format!("{}", cur * 100 / max),
                    cur as f64 * 100.0 / max as f64,
                ));
            }
        }
    }
    None
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

/// Get CPU temperature as (text, value in °C) (e.g. `61°C`): hwmon
/// first — Intel `coretemp`, AMD `k10temp` or `zenpower` (temp1 =
/// Tctl/package) — then the `x86_pkg_temp` thermal zone; None when
/// none of them exists. The text truncates to whole degrees, the graph
/// keeps the decimals.
pub fn cpu_temp() -> Option<(String, f64)> {
    for dev in ["coretemp", "k10temp", "zenpower"] {
        if let Some(t) = hwmon_temp(dev, "temp1_input") {
            return Some((format!("{}°C", t / 1000), t as f64 / 1000.0));
        }
    }
    thermal_zone_temp("x86_pkg_temp").map(|t| (format!("{}°C", t / 1000), t as f64 / 1000.0))
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
    let Some((rx, tx)) = net_counters(interface) else {
        // The interface is gone (removed device, config reload): its
        // sample must not outlive it.
        if let Some(map) = NET_LAST.get() {
            if let Ok(mut map) = map.lock() {
                map.remove(interface);
            }
        }
        return None;
    };
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

/// Figure space (U+2007): by Unicode definition as wide as one digit of the
/// font — padding the integer part with it keeps every formatted rate at the
/// same pixel width whatever figures the configured font uses (roadmap M5).
const FIGURE_SPACE: char = '\u{2007}';

/// Integer-part slots of a formatted rate: wide enough for the largest value
/// any tier can show (`1023.9`).
const RATE_INT_SLOTS: usize = 4;

/// Rate labels, one per order of magnitude.
const RATE_UNITS: [&str; 5] = ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"];

/// Format a byte rate with a FIXED composition (roadmap M5): a
/// [`RATE_INT_SLOTS`]-slot integer part padded with figure spaces, one
/// decimal digit, a space and the tier unit. The string's pixel width can't
/// change with the value, so the sensor box never resizes ("dances") while
/// the rate changes.
fn fmt_rate(bytes_per_sec: f64) -> String {
    let mut value = bytes_per_sec;
    let mut unit = 0;
    while value >= 1024.0 && unit < RATE_UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let shaped = format!("{value:.1}");
    let (int, dec) = shaped.split_once('.').unwrap_or((shaped.as_str(), "0"));
    let pad = std::iter::repeat_n(FIGURE_SPACE, RATE_INT_SLOTS.saturating_sub(int.len()))
        .collect::<String>();
    format!("{pad}{int}.{dec} {}", RATE_UNITS[unit])
}

/// Formatting samples covering every tier's narrow and wide shapes (roadmap
/// M5): the view pins the net sensor's width to the widest of them, so the
/// box stays stable even when the tier label changes.
pub fn net_rate_samples() -> Vec<String> {
    let mut samples = vec![fmt_rate(0.0), "N/A".to_string()];
    for tier in 0..RATE_UNITS.len() {
        let scale = 1024f64.powi(tier as i32);
        for value in [1.0, 777.7, 1023.9] {
            samples.push(fmt_rate(value * scale));
        }
    }
    samples
}

/// Get network download rate as (text, bytes/s) (fixed-width format,
/// e.g. `   1.4 MB/s`); None on the first tick — the graph plots the
/// raw B/s of the same sample.
pub fn net_down(interface: &str) -> Option<(String, f64)> {
    net_rate(interface, false).map(|rate| (fmt_rate(rate), rate))
}

/// Get network upload rate as (text, bytes/s) (fixed-width format,
/// e.g. ` 812.0 B/s`); None on the first tick.
pub fn net_up(interface: &str) -> Option<(String, f64)> {
    net_rate(interface, true).map(|rate| (fmt_rate(rate), rate))
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
        // Bare values (the bar's own display since roadmap2 #5) parse too.
        assert_eq!(parse_percent("50"), Some(50.0));
        assert_eq!(parse_percent("0"), Some(0.0));
        assert_eq!(parse_percent("N/A"), None);
    }

    /// Empty device falls back to the amixer `Master` control.
    #[test]
    fn volume_device_defaults_to_master() {
        assert_eq!(volume_device(""), "Master");
        assert_eq!(volume_device(" Speaker "), "Speaker");
    }

    /// Every formatted sample keeps the fixed composition (four-slot integer
    /// part of digits/figure spaces, dot, one decimal digit): the shape that
    /// makes the pixel width independent of the value. Spot checks included.
    #[test]
    fn net_rate_format_is_fixed_composition() {
        for sample in net_rate_samples() {
            if sample == "N/A" {
                continue;
            }
            let (int, rest) = sample
                .split_once('.')
                .unwrap_or_else(|| panic!("missing decimal dot in {sample}"));
            assert_eq!(int.chars().count(), RATE_INT_SLOTS, "{sample}");
            assert!(
                int.chars()
                    .all(|c| c.is_ascii_digit() || c == FIGURE_SPACE),
                "{sample}"
            );
            assert!(rest.starts_with(|c: char| c.is_ascii_digit()), "{sample}");
        }
        let f = FIGURE_SPACE;
        assert_eq!(fmt_rate(0.0), format!("{f}{f}{f}0.0 B/s"));
        assert_eq!(fmt_rate(1023.9), "1023.9 B/s");
        assert_eq!(fmt_rate(1024.0), format!("{f}{f}{f}1.0 KB/s"));
        assert_eq!(fmt_rate(2_000_000.0), format!("{f}{f}{f}1.9 MB/s"));
    }

    /// Titles of 80 chars stay intact; longer ones are cut at exactly 80
    /// chars and marked with "..."; multibyte titles cut at a char boundary
    /// (no panic, no split byte).
    #[test]
    fn window_title_truncation() {
        let exact: String = "a".repeat(WINDOW_TITLE_MAX);
        assert_eq!(truncate_title(&exact), exact);
        let over: String = format!("{}bb", exact);
        let cut = truncate_title(&over);
        assert_eq!(cut.len(), WINDOW_TITLE_MAX + 3);
        assert!(cut.ends_with("..."));
        assert_eq!(cut.trim_end_matches('.').len(), WINDOW_TITLE_MAX);
        // Multibyte: 81 two-byte chars -> cut stays valid UTF-8.
        let wide: String = "é".repeat(WINDOW_TITLE_MAX + 1);
        let cut = truncate_title(&wide);
        assert_eq!(cut.chars().count(), WINDOW_TITLE_MAX + 3);
        assert!(cut.ends_with("..."));
    }
}