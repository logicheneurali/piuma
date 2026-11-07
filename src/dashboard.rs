//! The dashboard: state, the probe that feeds it, and the surface's
//! open/close. The layout lives in `dashboard_view`; this half owns the
//! data the card shows and the actions it fires.
//!
//! Everything here costs nothing while the card is closed: the probe
//! (MPRIS, power profile, volume, brightness, Bluetooth, network,
//! uptime) runs on the existing tick pass, only with the dashboard up.

use std::path::PathBuf;

use iced::widget::image::Handle;

use crate::{sensors, Command, Message, Panel};

/// Everything the card shows beyond the bar's own sensors and
/// notifications. One snapshot per tick, while the dashboard is open.
#[derive(Clone, Debug)]
pub(crate) struct Probe {
    pub(crate) media: Option<crate::mpris::NowPlaying>,
    pub(crate) power_profile: String,
    pub(crate) bluetooth: String,
    pub(crate) net: String,
    pub(crate) uptime: String,
    pub(crate) battery: Option<String>,
    pub(crate) volume: Option<f32>,
    pub(crate) brightness: Option<f32>,
    /// `brightnessctl` is on PATH: the slider can write the backlight.
    pub(crate) can_set_brightness: bool,
}

/// Read the dashboard's snapshot (worker thread, open dashboard only).
pub(crate) fn probe() -> Probe {
    Probe {
        media: crate::mpris::now_playing(),
        power_profile: sensors::power_profile(),
        bluetooth: sensors::bluetooth_info(),
        net: sensors::net_info(),
        uptime: sensors::uptime(),
        battery: sensors::battery_perc().map(|(text, _)| text),
        volume: sensors::audio_volume("").map(|(_, pct)| pct as f32),
        brightness: sensors::backlight_perc().map(|(_, pct)| pct as f32),
        can_set_brightness: sensors::have_brightnessctl(),
    }
}

/// The controls that carry a state (or a one-shot action without input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    PowerProfile,
    Caffeine,
    Bluetooth,
    Lock,
    PlayPause,
    Previous,
    Next,
}

/// The open overlay: the surface that draws it.
pub(crate) struct Dashboard {
    pub(crate) id: iced::window::Id,
}

/// Open the dashboard on the active output, or close it when the sensor
/// is clicked again.
pub(crate) fn open(panel: &mut Panel) -> Command<Message> {
    if panel.dashboard.is_some() {
        return close(panel);
    }
    let (id, open) = Message::layershell_open(iced_exwlshell::reexport::NewLayerShellSettings {
        size: iced_exwlshell::settings::LayerSize::FILL,
        layer: iced_exwlshell::reexport::Layer::Overlay,
        anchor: iced_exwlshell::reexport::Anchor::Top
            | iced_exwlshell::reexport::Anchor::Bottom
            | iced_exwlshell::reexport::Anchor::Left
            | iced_exwlshell::reexport::Anchor::Right,
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: iced_exwlshell::reexport::KeyboardInteractivity::None,
        output_option: iced_exwlshell::reexport::OutputOption::Active,
        // Not transparent: the scrim catches the click that closes the
        // dashboard, wherever it lands outside the card.
        events_transparent: false,
        namespace: Some("piuma-dashboard".to_string()),
        blur_option: crate::glass_blur(&panel.config),
    });
    panel.dashboard = Some(Dashboard { id });
    // The first tick (within a second) fills the card.
    panel.dash_probe = None;
    panel.dash_profile = None;
    panel.dash_volume = None;
    panel.dash_brightness = None;
    panel.cover = None;
    panel.cover_url = None;
    open
}

pub(crate) fn close(panel: &mut Panel) -> Command<Message> {
    // Drop the track and its cover with the surface: the decode handle
    // keeps a texture in the renderer.
    panel.dash_probe = None;
    panel.dash_profile = None;
    panel.dash_volume = None;
    panel.dash_brightness = None;
    panel.cover = None;
    panel.cover_url = None;
    panel
        .dashboard
        .take()
        .map(|d| iced::window::close(d.id))
        .unwrap_or_else(Command::none)
}

pub(crate) fn is_window(panel: &Panel, window: iced::window::Id) -> bool {
    panel.dashboard.as_ref().is_some_and(|d| d.id == window)
}

/// The power profile the next click selects, in the labels
/// `sensors::power_profile` returns. Unknown readings behave like the
/// cycle's fallback (performance).
pub(crate) fn next_profile(current: &str) -> &'static str {
    match current {
        "Performance" => "Power saver",
        "Power saver" => "Balanced",
        _ => "Performance",
    }
}

/// `m:ss` under an hour, `h:mm:ss` above.
pub(crate) fn duration(ms: u64) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Decode a player's `mpris:artUrl` into a small square handle. Local
/// files only: the card never downloads anything.
pub(crate) fn cover(url: &str) -> Option<Handle> {
    let path = file_path(url)?;
    let decoded = ::image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let thumb = decoded.thumbnail(COVER as u32, COVER as u32).to_rgba8();
    let (width, height) = thumb.dimensions();
    Some(Handle::from_rgba(width, height, thumb.into_raw()))
}

/// The cover edge in the card.
pub(crate) const COVER: f32 = 96.0;

/// `file:///a%20b/c.png` → `/a b/c.png`; any other scheme is refused.
fn file_path(url: &str) -> Option<PathBuf> {
    let raw = url.strip_prefix("file://")?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8(out).ok()?;
    // `file://host/path` is not a local file: only an empty authority
    // (`file:///path`) is ours.
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    Some(PathBuf::from(path))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Local time without forking `date`: the card refreshes with the tick,
/// so it re-reads the clock on every render.
pub(crate) fn clock_and_date() -> (String, String) {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut now = std::mem::MaybeUninit::<libc::tm>::uninit();
    let mut secs = 0 as libc::time_t;
    // SAFETY: `now` is a valid `tm` buffer and `secs` a valid time_t.
    unsafe {
        libc::time(&mut secs);
        if libc::localtime_r(&secs, now.as_mut_ptr()).is_null() {
            return (String::new(), String::new());
        }
    }
    // SAFETY: `localtime_r` initialised the buffer above.
    let tm = unsafe { now.assume_init() };
    let weekday = WEEKDAYS.get(tm.tm_wday.rem_euclid(7) as usize).unwrap_or(&"");
    let month = MONTHS
        .get(tm.tm_mon.rem_euclid(12) as usize)
        .unwrap_or(&"");
    (
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
        format!("{weekday} {} {month} {}", tm.tm_mday, tm.tm_year + 1900),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_profile_cycle_matches_the_sensor() {
        assert_eq!(next_profile("Balanced"), "Performance");
        assert_eq!(next_profile("Performance"), "Power saver");
        assert_eq!(next_profile("Power saver"), "Balanced");
        assert_eq!(next_profile("N/A"), "Performance");
    }

    #[test]
    fn durations_use_the_shortest_form() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(61_000), "1:01");
        assert_eq!(duration(3_600_000), "1:00:00");
        assert_eq!(duration(3_661_000), "1:01:01");
    }

    #[test]
    fn file_urls_decode_to_local_paths_only() {
        assert_eq!(
            file_path("file:///home/me/a%20b/cover.png"),
            Some(PathBuf::from("/home/me/a b/cover.png"))
        );
        assert_eq!(
            file_path("file:///tmp/x.png"),
            Some(PathBuf::from("/tmp/x.png"))
        );
        assert_eq!(file_path("file://host/tmp/x.png"), None);
        assert_eq!(file_path("https://example.com/x.png"), None);
        assert_eq!(file_path("file:///bad%zz"), Some(PathBuf::from("/bad%zz")));
    }
}
