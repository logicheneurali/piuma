//! System sensors - collects various system information.
//!
//! All functions return a formatted string.

use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

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

/// Whether `nvidia-smi` is on PATH: probed once (the hardware doesn't
/// change while piuma runs) so machines without NVIDIA don't spawn a
/// failing process on every tick.
fn have_nvidia_smi() -> bool {
    static HAVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAVE.get_or_init(|| {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join("nvidia-smi").is_file())
        })
    })
}

/// GPU temperature (millidegrees) from the DRM hwmon of the first GPU
/// card: the vendor-neutral fallback for drivers without `nvidia-smi`
/// (amdgpu, nouveau, i915/xe where the kernel exposes it).
fn gpu_temp_sysfs() -> Option<f64> {
    let cards = fs::read_dir("/sys/class/drm").ok()?;
    for card in cards.flatten() {
        let name = card.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        if let Ok(hwmons) = fs::read_dir(card.path().join("device").join("hwmon")) {
            for hwmon in hwmons.flatten() {
                if let Some(milli) = read_num(hwmon.path().join("temp1_input")) {
                    return Some(milli as f64 / 1000.0);
                }
            }
        }
    }
    None
}

/// `utilization.gpu,temperature.gpu` (`"2, 42"`) → both readings; a
/// field reporting `N/A` stays `None` (it does not hide the other one).
fn parse_gpu_csv(text: &str) -> Option<(Option<f64>, Option<f64>)> {
    let mut fields = text.lines().next()?.split(',');
    let num = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let util = fields.next().and_then(num);
    let temp = fields.next().and_then(num);
    (util.is_some() || temp.is_some()).then_some((util, temp))
}

/// NVIDIA readings through nvidia-smi (the proprietary driver and the
/// nvidia-open modules both ship it); `None` when it rejects the query,
/// as it does with nouveau, so the sysfs fallback runs.
fn nvidia_gpu() -> Option<(Option<f64>, Option<f64>)> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_gpu_csv(&String::from_utf8_lossy(&out.stdout)))
        .flatten()
}

/// One reading per tick shared by `perc_gpu` and `temp_gpu`: a bar with
/// both pays a single command instead of one each. Just under the 1 s
/// pass, so every pass refreshes (a full second would elapse as 0.99 s
/// between ticks and serve the previous reading every other pass).
const GPU_TTL: std::time::Duration = std::time::Duration::from_millis(900);
static GPU_PROBE: ProbeSlot<(Option<f64>, Option<f64>)> = OnceLock::new();

fn gpu_probe() -> Option<(Option<f64>, Option<f64>)> {
    cached_probe(&GPU_PROBE, GPU_TTL, || {
        if let Some(nvidia) = have_nvidia_smi().then(nvidia_gpu).flatten() {
            return Some(nvidia);
        }
        let (util, temp) = (amdgpu_load(), gpu_temp_sysfs());
        (util.is_some() || temp.is_some()).then_some((util, temp))
    })
}

/// Get GPU load percentage (e.g. `12`) as (text, value): NVIDIA via
/// nvidia-smi, then the amdgpu sysfs counter (AMD); None elsewhere
/// (e.g. Intel has no standard interface). No hard dependency on the
/// NVIDIA driver.
pub fn gpu_load() -> Option<(String, f64)> {
    gpu_probe()?.0.map(|perc| (format!("{perc}"), perc))
}

/// GPU temperature as (text, °C), from the same probe as `gpu_load`:
/// nvidia-smi on NVIDIA, the DRM hwmon on the other drivers.
pub fn gpu_temp() -> Option<(String, f64)> {
    let temp = gpu_probe()?.1?;
    Some((format!("{}°C", temp as i64), temp))
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

/// The number at the start of a displayed value (`"62°C"` → 62,
/// `"1.4 K"` → 1.4, `"N/A"` → None): threshold colour rules read
/// the value the bar already shows, whatever unit suffix follows it.
pub fn leading_number(value: &str) -> Option<f32> {
    let t = value.trim_start();
    let end = t
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
        .last()
        .map(|(i, c)| i + c.len_utf8())?;
    t[..end].parse::<f32>().ok()
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

/// Set the mixer to an absolute percentage (`0..=100`): the click branch
/// of the graphical volume meter. Uses the cached backend first, like
/// [`adjust_volume`], and ignores a failed backend instead of leaving
/// the mixer half-changed.
pub fn set_volume(device: &str, pct: f32) {
    let _guard = VOLUME_ADJUST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let device = volume_device(device);
    let pct = pct.clamp(0.0, 100.0);
    for backend in backend_order() {
        if backend.set(device, pct) {
            cache_backend(backend);
            return;
        }
    }
    eprintln!("[piuma] volume set failed: no working backend (amixer/wpctl/pactl)");
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
const WINDOW_TITLE_MAX: usize = 50;

/// Cut `title` to [`WINDOW_TITLE_MAX`] chars, appending "..." when cut.
fn truncate_title(title: &str) -> String {
    if title.chars().count() <= WINDOW_TITLE_MAX {
        return title.to_string();
    }
    let cut: String = title.chars().take(WINDOW_TITLE_MAX).collect();
    format!("{cut}...")
}

/// The focused window's title from the shared snapshot (`hypr.rs`):
/// a failed document is "No Focus" (the old fork-failure path), a
/// snapshot without a focused row keeps "No Title".
pub fn window_title(snap: Option<&crate::hypr::Snapshot>) -> String {
    let Some(rows) = snap.and_then(|s| s.clients.as_ref()) else {
        return "No Focus".to_string();
    };
    rows.iter()
        .find(|row| row.focused)
        .map(|row| truncate_title(&row.title))
        .unwrap_or_else(|| "No Title".to_string())
}

/// The active workspace's name from the shared snapshot (`hypr.rs`).
pub fn workspace_name(snap: Option<&crate::hypr::Snapshot>) -> String {
    snap.and_then(|s| s.active_workspace.clone())
        .unwrap_or_else(|| "1".to_string())
}

// ── disk (df) ───────────────────────────────────────────────────────

/// `df -h /` row fields: [device, size, used, avail, use%, mount].
/// Locale-independent: df column headers are localized, positions are not.
/// `df` is cached for half a minute: filesystem usage does not move at
/// the bar's 1 Hz, and the three disk sources share one reading.
const DF_TTL: std::time::Duration = std::time::Duration::from_secs(30);
static DF_PROBE: ProbeSlot<Vec<String>> = OnceLock::new();

fn df_root() -> Option<Vec<String>> {
    cached_probe(&DF_PROBE, DF_TTL, || {
        let out = Command::new("df").args(["-h", "/"]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let parts: Vec<String> = stdout.lines().nth(1)?.split_whitespace().map(str::to_string).collect();
        (parts.len() >= 5).then_some(parts)
    })
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

/// Whether `brightnessctl` is on PATH: the dashboard's brightness
/// slider only appears when it can actually write the backlight.
pub fn have_brightnessctl() -> bool {
    static HAVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAVE.get_or_init(|| {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join("brightnessctl").is_file())
        })
    })
}

/// Set the backlight to `pct` (0..=100) through `brightnessctl`; a
/// failure is logged, never fatal.
pub fn set_backlight(pct: f64) {
    let pct = pct.clamp(0.0, 100.0).round();
    if let Err(e) = Command::new("brightnessctl")
        .args(["set", &format!("{pct:.0}%")])
        .status()
    {
        eprintln!("[piuma] backlight set failed: {e}");
    }
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

/// Rate labels, one per order of magnitude: the bare tier letter (the
/// row is a rate at a glance — `1.9 K` — and every tier keeps the same
/// one-glyph width, so the pinned box barely moves with the unit).
const RATE_UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];

/// A byte rate scaled into its tier and that tier's index (`(9.1, 1)`
/// for 9318): the scale step both rate formattings share.
fn rate_tier(bytes_per_sec: f64) -> (f64, usize) {
    let mut value = bytes_per_sec;
    let mut unit = 0;
    while value >= 1024.0 && unit < RATE_UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    (value, unit)
}

/// A byte rate in its own tier without the sensor box's fixed-width
/// padding, for labels whose width may breathe (the graph popup's
/// min/max/average): `9.1 K/s`, `1.9 M/s` — the same step and digits
/// the sensor box shows, so the two never disagree on a unit.
pub(crate) fn rate_label(bytes_per_sec: f64) -> String {
    let (value, tier) = rate_tier(bytes_per_sec);
    format!("{value:.1} {}/s", RATE_UNITS[tier])
}

/// Format a byte rate in its tier: one decimal digit, a space and the
/// tier unit (`1.4 K`, `1.9 M`). The sensor box's width is pinned by the
/// view's fixed container, so the string itself carries no padding and
/// the value aligns with every other sensor's value.
fn fmt_rate(bytes_per_sec: f64) -> String {
    let (value, unit) = rate_tier(bytes_per_sec);
    format!("{value:.1} {}", RATE_UNITS[unit])
}

/// Formatting samples covering the narrow and wide shapes of the ONE tier
/// a displayed value belongs to (roadmap M5): the view pins the net
/// sensor's width to the widest of them, so inside a tier the box is exact
/// and constant. The pin
/// is derived from the same tick's string as the value, so when a rate
/// crosses a ×1024 boundary both move together — the box can never
/// undershoot what it holds, and it only ever resizes on that crossing.
pub fn net_rate_samples(value: &str) -> Vec<String> {
    let mut samples = vec!["N/A".to_string()];
    let scale = 1024f64.powi(tier_of(value) as i32);
    for v in [1.0, 777.7, 1023.9] {
        samples.push(fmt_rate(v * scale));
    }
    samples
}

/// The tier index of a displayed value's unit suffix (`"134.6 B"` → 0,
/// `"1.9 M"` → 2); an unrecognised string (`"N/A"`) falls back to the
/// base tier, whose samples still fit it with room to spare.
fn tier_of(value: &str) -> usize {
    value
        .rsplit_once(' ')
        .and_then(|(_, unit)| RATE_UNITS.iter().position(|u| *u == unit))
        .unwrap_or(0)
}

/// Get network download rate as (text, bytes/s) (fixed-width format,
/// e.g. `   1.4 M`); None on the first tick — the graph plots the
/// raw B/s of the same sample.
pub fn net_down(interface: &str) -> Option<(String, f64)> {
    net_rate(interface, false).map(|rate| (fmt_rate(rate), rate))
}

/// Get network upload rate as (text, bytes/s) (fixed-width format,
/// e.g. ` 812.0 B`); None on the first tick.
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

    /// Threshold rules read the leading number whatever suffix follows;
    /// negative readings (below-zero temperatures) keep their sign and
    /// unparsable values give nothing.
    #[test]
    fn leading_number_takes_the_value_head() {
        assert_eq!(leading_number("62°C"), Some(62.0));
        assert_eq!(leading_number("1.4 K"), Some(1.4));
        assert_eq!(leading_number("1.4 KB/s"), Some(1.4));
        assert_eq!(leading_number("50"), Some(50.0));
        assert_eq!(leading_number("100%"), Some(100.0));
        assert_eq!(leading_number("-3°C"), Some(-3.0));
        assert_eq!(leading_number("N/A"), None);
        assert_eq!(leading_number(""), None);
    }

    /// The shared GPU probe parses util and temperature from one line;
    /// an `N/A` field stays missing without hiding the other, and junk
    /// yields no reading (the sysfs fallback then runs).
    #[test]
    fn gpu_csv_keeps_the_fields_it_understands() {
        assert_eq!(parse_gpu_csv("2, 42\n"), Some((Some(2.0), Some(42.0))));
        assert_eq!(parse_gpu_csv("N/A, 42"), Some((None, Some(42.0))));
        assert_eq!(parse_gpu_csv("0, N/A"), Some((Some(0.0), None)));
        assert_eq!(parse_gpu_csv("N/A, N/A"), None);
        assert_eq!(parse_gpu_csv("couldn't communicate"), None);
        assert_eq!(parse_gpu_csv(""), None);
    }

    /// Empty device falls back to the amixer `Master` control.
    #[test]
    fn volume_device_defaults_to_master() {
        assert_eq!(volume_device(""), "Master");
        assert_eq!(volume_device(" Speaker "), "Speaker");
    }

    /// Every formatted sample is the tier's plain composition (digits, one
    /// decimal digit, space, unit): the value's pixel width is pinned by the
    /// view's fixed box, so the string itself carries no padding. Spot
    /// checks included.
    #[test]
    fn net_rate_format_is_the_plain_tier_shape() {
        let mut samples: Vec<String> = Vec::new();
        for tier in 0..RATE_UNITS.len() {
            let shown = fmt_rate(1023.9 * 1024f64.powi(tier as i32));
            samples.extend(net_rate_samples(&shown));
        }
        for sample in samples {
            if sample == "N/A" {
                continue;
            }
            let (int, rest) = sample
                .split_once('.')
                .unwrap_or_else(|| panic!("missing decimal dot in {sample}"));
            assert!(!int.is_empty() && int.chars().all(|c| c.is_ascii_digit()), "{sample}");
            assert!(rest.starts_with(|c: char| c.is_ascii_digit()), "{sample}");
        }
        assert_eq!(fmt_rate(0.0), "0.0 B");
        assert_eq!(fmt_rate(1023.9), "1023.9 B");
        assert_eq!(fmt_rate(1024.0), "1.0 K");
        assert_eq!(fmt_rate(2_000_000.0), "1.9 M");
    }

    /// The unpadded tier label adapts per value: bytes below a KiB, the
    /// tier the value itself reaches, and the top tier as a hard cap.
    #[test]
    fn rate_label_steps_through_the_tiers() {
        assert_eq!(rate_label(0.0), "0.0 B/s");
        assert_eq!(rate_label(1023.9), "1023.9 B/s");
        assert_eq!(rate_label(1024.0), "1.0 K/s");
        assert_eq!(rate_label(9318.4), "9.1 K/s");
        assert_eq!(rate_label(2_000_000.0), "1.9 M/s");
        assert_eq!(rate_label(1024f64.powi(6)), "1048576.0 T/s");
    }

    /// The pin is the widest same-tier shape: never below the displayed
    /// value's width (no wrap) and exactly the widest sample's (no room
    /// beyond it). `"N/A"` pins to the base tier and still fits.
    #[test]
    fn net_rate_pin_is_exact_for_its_tier() {
        let font = iced::font::Font {
            weight: iced::font::Weight::Bold,
            ..iced::font::Font::with_name("Noto Sans")
        };
        for tier in 0..RATE_UNITS.len() {
            let scale = 1024f64.powi(tier as i32);
            let widest_shape = fmt_rate(1023.9 * scale);
            let pin = crate::text_width::widest(&font, 12, &net_rate_samples(&widest_shape));
            for v in [0.0, 1.0, 12.3, 134.6, 777.7, 1023.9] {
                let shown = fmt_rate(v * scale);
                let width = crate::text_width::widest(&font, 12, std::slice::from_ref(&shown));
                assert!(pin >= width, "{shown} ({width}px) must fit its pin ({pin}px)");
            }
            let width = crate::text_width::widest(&font, 12, std::slice::from_ref(&widest_shape));
            assert_eq!(pin, width, "the widest shape pins exactly");
        }
        let shown = "N/A".to_string();
        let pin = crate::text_width::widest(&font, 12, &net_rate_samples(&shown));
        let width = crate::text_width::widest(&font, 12, std::slice::from_ref(&shown));
        assert!(pin >= width, "N/A ({width}px) must fit its pin ({pin}px)");
    }

    /// Titles of `WINDOW_TITLE_MAX` chars stay intact; longer ones are cut
    /// at exactly that many chars and marked with "..."; multibyte titles
    /// cut at a char boundary (no panic, no split byte).
    #[test]
    fn window_title_truncation() {
        let exact: String = "a".repeat(WINDOW_TITLE_MAX);
        assert_eq!(truncate_title(&exact), exact);
        let over: String = format!("{}bb", exact);
        let cut = truncate_title(&over);
        assert_eq!(cut.len(), WINDOW_TITLE_MAX + 3);
        assert!(cut.ends_with("..."));
        assert_eq!(cut.trim_end_matches('.').len(), WINDOW_TITLE_MAX);
        // Multibyte: one char over the limit -> cut stays valid UTF-8.
        let wide: String = "é".repeat(WINDOW_TITLE_MAX + 1);
        let cut = truncate_title(&wide);
        assert_eq!(cut.chars().count(), WINDOW_TITLE_MAX + 3);
        assert!(cut.ends_with("..."));
    }

    /// The keyboard sensor's short codes cover the common Hyprland
    /// keymap names; anything else becomes its first two letters.
    #[test]
    fn keymap_codes_are_short() {
        assert_eq!(keymap_code("Italian"), "IT");
        assert_eq!(keymap_code("English (US)"), "EN");
        assert_eq!(keymap_code("English (UK)"), "UK");
        assert_eq!(keymap_code("German"), "DE");
        assert_eq!(keymap_code("Workman"), "WO");
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("0123456789", 5), "0123…");
    }
}
// ── runtime state: NetworkManager, power profiles, BlueZ, keyboard ──

/// How often the CLI probes behind these sensors run: the values change
/// on human timescales and the bar polls at 1 Hz, so one fork every
/// [`PROBE_TTL`] keeps them fresh without spawning per tick. Failures
/// are cached too (a missing daemon does not spawn a process a second).
const PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

type ProbeSlot<T> = OnceLock<Mutex<Option<(Instant, Option<T>)>>>;

static NET_INFO_PROBE: ProbeSlot<String> = OnceLock::new();
static POWER_PROFILE_PROBE: ProbeSlot<String> = OnceLock::new();
static BLUETOOTH_PROBE: ProbeSlot<String> = OnceLock::new();
static WIFI_PROBE: ProbeSlot<WifiStatus> = OnceLock::new();
static INTERNET_PROBE: ProbeSlot<bool> = OnceLock::new();

/// A cached CLI probe: the command runs at most every `ttl`; `None`
/// means "no reading" and the sensor shows N/A.
fn cached_probe<T: Clone>(
    slot: &'static ProbeSlot<T>,
    ttl: std::time::Duration,
    read: impl FnOnce() -> Option<T>,
) -> Option<T> {
    let mut cached = slot.get_or_init(Default::default).lock().ok()?;
    if let Some((at, value)) = cached.as_ref() {
        if at.elapsed() < ttl {
            return value.clone();
        }
    }
    let value = read();
    *cached = Some((Instant::now(), value.clone()));
    value
}

/// Drop a probe's cached answer after an action changed the state, so
/// the next tick re-reads it instead of showing the old value for `ttl`.
fn invalidate_probe<T>(slot: &'static ProbeSlot<T>) {
    if let Some(lock) = slot.get() {
        if let Ok(mut cached) = lock.lock() {
            *cached = None;
        }
    }
}

/// Cut `text` to `max` chars (multibyte-safe) with a trailing ellipsis.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// The active NetworkManager connection: VPN, then Wi-Fi (its SSID),
/// then wired, then anything else; `offline` when nothing is up.
pub fn net_info() -> String {
    cached_probe(&NET_INFO_PROBE, PROBE_TTL, || {
        let out = Command::new("nmcli")
            .args([
                "-t", "--escape", "no", "-f", "NAME,TYPE,DEVICE", "connection", "show",
                "--active",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut best: Option<(u8, String)> = None;
        for line in text.lines() {
            let mut parts = line.splitn(3, ':');
            let (Some(name), Some(kind)) = (parts.next(), parts.next()) else {
                continue;
            };
            if kind == "loopback" {
                continue;
            }
            let rank = match kind {
                "vpn" | "wireguard" => 0,
                "802-11-wireless" => 1,
                "802-3-ethernet" => 2,
                _ => 3,
            };
            if best.as_ref().is_none_or(|(current, _)| rank < *current) {
                best = Some((rank, name.to_string()));
            }
        }
        Some(best.map_or_else(|| "offline".to_string(), |(_, name)| truncate_chars(&name, 24)))
    })
    .unwrap_or_else(|| "N/A".to_string())
}

/// Wi-Fi state for the `wifi` sensor. NetworkManager is the primary
/// source (its own connectivity check answers "internet"); iwd is the
/// fallback, queried over D-Bus (iwd's bus policy allows `wheel` and
/// `network` members, the groups its setups put the desktop user in).
/// The probe is cached for [`PROBE_TTL`]; the internet TCP check has its
/// own, longer TTL.
#[derive(Clone, Debug, PartialEq)]
pub struct WifiStatus {
    pub connected: bool,
    /// The link has internet access (NM connectivity `full`, or the
    /// fallback TCP probe succeeding when the manager cannot tell).
    pub internet: bool,
    pub ssid: String,
    /// Signal 0..=100, `None` while disconnected or unknown.
    pub signal: Option<u8>,
}

impl WifiStatus {
    /// The sensor text: the SSID when online, `no internet` when the
    /// link is up without access, `off` when down.
    pub fn label(&self) -> String {
        if !self.connected {
            "off".to_string()
        } else if !self.internet {
            "no internet".to_string()
        } else if self.ssid.is_empty() {
            "on".to_string()
        } else {
            truncate_chars(&self.ssid, 24)
        }
    }
}

/// The current Wi-Fi state, or `None` when neither NetworkManager nor
/// iwd answers (the sensor then shows `N/A`).
pub fn wifi_status() -> Option<WifiStatus> {
    cached_probe(&WIFI_PROBE, PROBE_TTL, || {
        network_manager_wifi().or_else(iwd_wifi)
    })
}

/// NetworkManager: the active access point (signal + SSID) and its
/// connectivity state. `None` only when nmcli itself is unavailable or
/// NetworkManager is not running, so the caller can try iwd.
fn network_manager_wifi() -> Option<WifiStatus> {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "IN-USE,SIGNAL,SSID", "device", "wifi"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let active = parse_active_wifi(&String::from_utf8_lossy(&out.stdout));
    let Some((ssid, signal)) = active else {
        return Some(WifiStatus {
            connected: false,
            internet: false,
            ssid: String::new(),
            signal: None,
        });
    };
    let internet = match nm_connectivity().as_deref() {
        Some("full") => true,
        Some("limited" | "portal" | "none") => false,
        // Connectivity checking disabled or not answered yet: ask the
        // network directly instead of guessing.
        _ => internet_reachable(),
    };
    Some(WifiStatus {
        connected: true,
        internet,
        ssid,
        signal,
    })
}

/// The `*` line of `nmcli -t -f IN-USE,SIGNAL,SSID device wifi`:
/// `(SSID, signal)`. `None` = no active access point.
fn parse_active_wifi(text: &str) -> Option<(String, Option<u8>)> {
    text.lines().find_map(|line| {
        let mut parts = line.splitn(3, ':');
        if parts.next()?.trim() != "*" {
            return None;
        }
        let signal = parts.next()?.trim().parse::<u8>().ok();
        let ssid = parts.next().map(unescape_terse).unwrap_or_default();
        Some((ssid, signal))
    })
}

/// nmcli's terse mode escapes `:`, `\` and `;` with a backslash.
fn unescape_terse(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn nm_connectivity() -> Option<String> {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "CONNECTIVITY", "general"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// iwd fallback, over its D-Bus API: the station's state, connected
/// network name and the signal from `GetOrderedNetworks` (int16 dBm).
fn iwd_wifi() -> Option<WifiStatus> {
    use zbus::zvariant::OwnedObjectPath;

    let conn = zbus::blocking::Connection::system().ok()?;
    let manager = zbus::blocking::Proxy::new(
        &conn,
        "net.connman.iwd",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .ok()?;
    let reply = manager.call_method("GetManagedObjects", &()).ok()?;
    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>> =
        reply.body().deserialize().ok()?;
    let station_path = objects.iter().find_map(|(path, interfaces)| {
        interfaces
            .contains_key("net.connman.iwd.Station")
            .then(|| path.clone())
    })?;
    let station = zbus::blocking::Proxy::new(
        &conn,
        "net.connman.iwd",
        station_path.as_str(),
        "net.connman.iwd.Station",
    )
    .ok()?;
    let state: String = station.get_property("State").ok()?;
    if state != "connected" {
        return Some(WifiStatus {
            connected: false,
            internet: false,
            ssid: String::new(),
            signal: None,
        });
    }
    let network_path: OwnedObjectPath = station.get_property("ConnectedNetwork").ok()?;
    let network = zbus::blocking::Proxy::new(
        &conn,
        "net.connman.iwd",
        network_path.as_str(),
        "net.connman.iwd.Network",
    )
    .ok()?;
    let ssid: String = network.get_property("Name").unwrap_or_default();
    let ordered: Vec<(OwnedObjectPath, i16)> = station
        .call_method("GetOrderedNetworks", &())
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let signal = ordered
        .iter()
        .find(|(path, _)| path == &network_path)
        .map(|(_, dbm)| dbm_to_percent(*dbm));
    Some(WifiStatus {
        connected: true,
        internet: internet_reachable(),
        ssid,
        signal,
    })
}

/// RSSI (dBm) → 0..=100: -50 dBm or better is full scale, -100 dBm is
/// zero (the usual 2 dB per percent slope).
fn dbm_to_percent(dbm: i16) -> u8 {
    ((i32::from(dbm) + 100) * 2).clamp(0, 100) as u8
}

/// Whether a TCP connection to a public address succeeds. iwd has no
/// connectivity API, and NetworkManager may report `unknown` when its
/// own check is disabled; one SYN per [`INTERNET_TTL`] with a 1.5 s
/// timeout is the cheap, manager-independent answer. The address is
/// only a reachability target, not fetched or trusted.
const INTERNET_CHECK: &str = "1.1.1.1:443";
const INTERNET_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn internet_reachable() -> bool {
    cached_probe(&INTERNET_PROBE, INTERNET_TTL, || {
        let address = INTERNET_CHECK.parse::<std::net::SocketAddr>().ok()?;
        Some(
            std::net::TcpStream::connect_timeout(
                &address,
                std::time::Duration::from_millis(1500),
            )
            .is_ok(),
        )
    })
    .unwrap_or(false)
}

/// power-profiles-daemon's active profile, as a readable label.
pub fn power_profile() -> String {
    cached_probe(&POWER_PROFILE_PROBE, PROBE_TTL, || {
        let out = Command::new("powerprofilesctl").arg("get").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!raw.is_empty()).then_some(raw)
    })
    .map(|raw| match raw.as_str() {
        "balanced" => "Balanced".to_string(),
        "performance" => "Performance".to_string(),
        "power-saver" => "Power saver".to_string(),
        other => other.to_string(),
    })
    .unwrap_or_else(|| "N/A".to_string())
}

/// Click on the power-profile sensor: balanced → performance →
/// power-saver → balanced. Spawned off the UI thread.
pub fn cycle_power_profile() {
    let current = Command::new("powerprofilesctl")
        .arg("get")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
    let next = match current.as_deref() {
        Some("performance") => "power-saver",
        Some("power-saver") => "balanced",
        _ => "performance",
    };
    let _ = Command::new("powerprofilesctl").args(["set", next]).status();
    invalidate_probe(&POWER_PROFILE_PROBE);
}

/// BlueZ adapter state: `Off`, `On`, `On (N)` or the single connected
/// device's name.
pub fn bluetooth_info() -> String {
    cached_probe(&BLUETOOTH_PROBE, PROBE_TTL, || {
        let show = Command::new("bluetoothctl").arg("show").output().ok()?;
        if !show.status.success() {
            return None;
        }
        let powered = String::from_utf8_lossy(&show.stdout)
            .lines()
            .any(|line| line.trim() == "Powered: yes");
        if !powered {
            return Some("Off".to_string());
        }
        let devices = Command::new("bluetoothctl")
            .args(["devices", "Connected"])
            .output()
            .ok()?;
        let names: Vec<String> = String::from_utf8_lossy(&devices.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("Device "))
            .filter_map(|rest| rest.split_once(' ').map(|(_, name)| name.trim().to_string()))
            .filter(|name| !name.is_empty())
            .collect();
        Some(match names.len() {
            0 => "On".to_string(),
            1 => truncate_chars(&names[0], 20),
            count => format!("On ({count})"),
        })
    })
    .unwrap_or_else(|| "N/A".to_string())
}

/// Click on the Bluetooth sensor: toggle the adapter (off disconnects
/// every device — that is the user's click).
pub fn toggle_bluetooth() {
    let on = Command::new("bluetoothctl")
        .arg("show")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .is_some_and(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line.trim() == "Powered: yes")
        });
    let _ = Command::new("bluetoothctl")
        .args(["power", if on { "off" } else { "on" }])
        .status();
    invalidate_probe(&BLUETOOTH_PROBE);
}

/// The `main` keyboard (falling back to the first), from the `devices`
/// document.
fn main_keyboard(json: &serde_json::Value) -> Option<&serde_json::Value> {
    let keyboards = json["keyboards"].as_array()?;
    keyboards
        .iter()
        .find(|keyboard| keyboard["main"].as_bool() == Some(true))
        .or_else(|| keyboards.first())
}

/// Short code for a Hyprland keymap name (`Italian` → `IT`).
fn keymap_code(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "italian" => "IT",
        "english (us)" | "english (usa)" | "us" => "EN",
        "english (uk)" | "gb" => "UK",
        "german" | "de" => "DE",
        "french" | "fr" => "FR",
        "spanish" | "es" => "ES",
        "portuguese" | "pt" => "PT",
        "russian" | "ru" => "RU",
        _ => {
            let code: String = name
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .take(2)
                .collect();
            return code.to_uppercase();
        }
    }
    .to_string()
}

/// Hyprland's main keyboard layout and lock state: `IT`, `IT CAPS`,
/// `IT NUM`, both when active (plain words: the layout font is the
/// user's, and the arrow glyphs are not guaranteed to exist).
pub fn keyboard_info() -> String {
    let Ok(out) = Command::new("hyprctl").args(["-j", "devices"]).output() else {
        return "N/A".to_string();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return "N/A".to_string();
    };
    let Some(keyboard) = main_keyboard(&json) else {
        return "N/A".to_string();
    };
    let mut text = keymap_code(keyboard["active_keymap"].as_str().unwrap_or(""));
    if keyboard["capsLock"].as_bool() == Some(true) {
        text.push_str(" CAPS");
    }
    if keyboard["numLock"].as_bool() == Some(true) {
        text.push_str(" NUM");
    }
    text
}

/// Click on the keyboard sensor: next configured layout on the main
/// keyboard (a single-layout setup stays put).
pub fn keyboard_next_layout() {
    let Ok(out) = Command::new("hyprctl").args(["-j", "devices"]).output() else {
        return;
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return;
    };
    let Some(name) = main_keyboard(&json).and_then(|keyboard| keyboard["name"].as_str()) else {
        return;
    };
    let _ = Command::new("hyprctl")
        .args(["switchxkblayout", name, "next"])
        .status();
}

#[cfg(test)]
mod wifi_tests {
    use super::*;

    /// The active access point is the `*` line; escaped colons in the
    /// SSID survive the terse parser and the others are ignored.
    #[test]
    fn active_access_point_parses_from_terse_output() {
        assert_eq!(
            parse_active_wifi(" :80:OTHER\n*:57:FASTWEB-3N2E64\n :77:\n"),
            Some(("FASTWEB-3N2E64".to_string(), Some(57)))
        );
        assert_eq!(
            parse_active_wifi("*:65:My\\:Network"),
            Some(("My:Network".to_string(), Some(65)))
        );
        assert_eq!(parse_active_wifi(" :80:OTHER\n"), None);
        assert_eq!(parse_active_wifi(""), None);
    }

    /// dBm mapping: -50 or better is 100, -100 is 0, 2 dB per percent.
    #[test]
    fn dbm_maps_to_five_signal_levels() {
        assert_eq!(dbm_to_percent(-40), 100);
        assert_eq!(dbm_to_percent(-50), 100);
        assert_eq!(dbm_to_percent(-60), 80);
        assert_eq!(dbm_to_percent(-70), 60);
        assert_eq!(dbm_to_percent(-80), 40);
        assert_eq!(dbm_to_percent(-90), 20);
        assert_eq!(dbm_to_percent(-100), 0);
        assert_eq!(dbm_to_percent(-120), 0);
    }

    /// The text names the state, and a hidden SSID still reads "on".
    #[test]
    fn wifi_labels_name_the_state() {
        let label = |connected, internet, ssid: &str| WifiStatus {
            connected,
            internet,
            ssid: ssid.to_string(),
            signal: None,
        }
        .label();
        assert_eq!(label(false, false, "home"), "off");
        assert_eq!(label(true, false, "home"), "no internet");
        assert_eq!(label(true, true, ""), "on");
        assert_eq!(label(true, true, "home"), "home");
    }

    /// The internet probe talks to a reachable public endpoint on this
    /// machine (the cache makes it one attempt); it must not panic and
    /// must answer whether the network is up.
    #[test]
    fn internet_probe_answers() {
        let _ = internet_reachable();
    }
}
