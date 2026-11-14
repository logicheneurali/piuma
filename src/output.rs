//! Monitor, output and layer-shell placement helpers.
//!
//! Hyprland reports physical pixels; layer-shell sizes are logical, so a
//! scaled output is divided by its scale. These helpers are pure where it
//! matters (`*_in` parse a `hyprctl monitors -j` document) so the tests
//! can feed real answers.

use std::time::Instant;

use iced_exwlshell::reexport::OutputOption;
use iced_exwlshell::settings::StartMode;

use crate::config;
use crate::{SCREEN_WIDTH, SCREEN_HEIGHT};

/// Last monitor list with its timestamp: outputs change rarely (hotplug),
/// so the wallpaper sync — which asks every tick — gets a cached answer
/// for `MONITORS_TTL` instead of spawning and parsing `hyprctl` at 1 Hz.
pub(crate) static MONITORS: std::sync::Mutex<Option<(Instant, Vec<String>)>> = std::sync::Mutex::new(None);

/// Freshness window of [`MONITORS`]: long enough to make the per-tick
/// spawn rare, short enough that a hotplugged output gets its wallpaper
/// surface within seconds.
pub(crate) const MONITORS_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// Names of the currently active outputs, from `hyprctl monitors -j` —
/// best-effort, the same source the bar's geometry helpers use (cached,
/// see [`MONITORS`]); no answer = no surface for now (retried on the
/// next tick).
pub(crate) fn monitor_names() -> Vec<String> {
    if let Ok(cached) = MONITORS.lock() {
        if let Some((at, names)) = cached.as_ref() {
            if at.elapsed() < MONITORS_TTL && !names.is_empty() {
                return names.clone();
            }
        }
    }
    let Ok(output) = std::process::Command::new("hyprctl").args(["monitors", "-j"]).output() else {
        return Vec::new();
    };
    let names = monitor_names_json(&String::from_utf8_lossy(&output.stdout));
    // An empty answer is never cached: a transient hyprctl failure
    // retries on the next tick instead of freezing the list.
    if !names.is_empty() {
        if let Ok(mut cached) = MONITORS.lock() {
            *cached = Some((Instant::now(), names.clone()));
        }
    }
    names
}

/// `name` of every entry of the `hyprctl monitors -j` array; anything
/// unparseable yields no outputs instead of breaking the bar.
pub(crate) fn monitor_names_json(json: &str) -> Vec<String> {
    let Ok(monitors) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    monitors
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect()
}

/// The focused output — the bar's own, since Hyprland focuses the
/// monitor under the cursor and everything cursor-driven (graph popup,
/// tray menu, notification cards) belongs there — as `(name, x, y)`
/// from a fresh `hyprctl monitors -j`. `OutputOption::LastOutput`
/// picked another output once a second monitor joined (the popup
/// opened over on it); no answer falls back to the old behaviour.
pub(crate) fn focused_output() -> Option<(String, i32, i32)> {
    let output = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    focused_output_in(&String::from_utf8_lossy(&output.stdout))
}

/// The (`name`, `x`, `y`) of the `hyprctl monitors -j` entry with
/// `"focused": true`; unparseable input yields no output.
pub(crate) fn focused_output_in(json: &str) -> Option<(String, i32, i32)> {
    let monitors = serde_json::from_str::<Vec<serde_json::Value>>(json).ok()?;
    let m = monitors
        .iter()
        .find(|m| m.get("focused").and_then(|f| f.as_bool()) == Some(true))?;
    Some((
        m.get("name")?.as_str()?.to_string(),
        m.get("x")?.as_i64()? as i32,
        m.get("y")?.as_i64()? as i32,
    ))
}

/// The output the cursor-driven popup surfaces pin themselves to (the
/// bar's own, see [`focused_output`]); `LastOutput` as the fallback.
pub(crate) fn active_output_name() -> Option<String> {
    focused_output().map(|(name, _, _)| name)
}

/// The output for the graph popup: the bar's own
/// ([`active_output_name`]), falling back to the old `LastOutput`
/// behaviour when hyprctl can't answer.
pub(crate) fn popup_output_option() -> OutputOption {
    active_output_name()
        .map(OutputOption::OutputName)
        .unwrap_or(OutputOption::LastOutput)
}










/// Best-effort primary monitor width, via `hyprctl`.
/// The layer-shell start mode selected by `panel.output`: a named output,
/// or the compositor's focused one for `auto`/absent.
#[cfg(test)]
pub(crate) fn start_mode_for(panel: &config::PanelConfig) -> StartMode {
    start_mode_from(panel.output.as_deref())
}

pub(crate) fn start_mode_for_layout(layout: &config::PanelLayout) -> StartMode {
    start_mode_from(layout.output.as_deref())
}

fn start_mode_from(output: Option<&str>) -> StartMode {
    match output.map(str::trim).filter(|name| !name.is_empty() && *name != "auto") {
        Some("all") => StartMode::AllScreens,
        Some(name) => StartMode::TargetScreen(name.to_string()),
        None => StartMode::Active,
    }
}

/// Output for an additional panel window: an explicit monitor, otherwise
/// the focused one (`all` is a property of the first panel only).
pub(crate) fn layout_output_option(layout: &config::PanelLayout) -> OutputOption {
    match layout
        .output
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "auto" && *name != "all")
    {
        Some(name) => OutputOption::OutputName(name.to_string()),
        None => OutputOption::Active,
    }
}

/// `(width, height)` of the requested output, else the focused one, else
/// the first monitor. Pure: the caller passes the `hyprctl monitors -j`
/// document.
pub(crate) fn screen_size_in(json: &str, output: Option<&str>) -> Option<(u32, u32)> {
    let monitors = serde_json::from_str::<Vec<serde_json::Value>>(json).ok()?;
    let size = |monitor: &serde_json::Value| {
        // Hyprland reports PHYSICAL pixels; layer-shell sizes are logical,
        // so a scaled output must be divided by its scale (eDP 2560@2 →
        // 1280 wide).
        let scale = monitor
            .get("scale")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .max(0.01);
        let width = monitor.get("width")?.as_u64()? as f64 / scale;
        let height = monitor.get("height")?.as_u64()? as f64 / scale;
        Some((width.round() as u32, height.round() as u32))
    };
    let named = output
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "auto")
        .and_then(|name| {
            monitors
                .iter()
                .find(|m| m.get("name").and_then(|v| v.as_str()) == Some(name))
        })
        .and_then(size);
    named
        .or_else(|| {
            monitors
                .iter()
                .find(|m| m.get("focused").and_then(|v| v.as_bool()) == Some(true))
                .and_then(size)
        })
        .or_else(|| monitors.first().and_then(size))
}

/// Logical size of the focused output: popups/menus opened under the
/// cursor must clamp to the monitor they actually open on, not to the
/// last panel that happened to render.
pub(crate) fn focused_screen_size() -> (i32, i32) {
    focused_output()
        .map(|(name, _, _)| screen_size_for(Some(&name)))
        .map(|(width, height)| (width as i32, height as i32))
        .unwrap_or(unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) })
}

/// [`screen_size_in`] against the live compositor; 1920×1080 when
/// `hyprctl` cannot answer.
pub(crate) fn screen_size_for(output: Option<&str>) -> (u32, u32) {
    std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()
        .and_then(|out| screen_size_in(&String::from_utf8_lossy(&out.stdout), output))
        .unwrap_or((1920, 1080))
}


