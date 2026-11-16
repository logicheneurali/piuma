//! MPRIS (Linux media players): the `media` sensor and its playback
//! controls. The session bus is polled once per sensor tick with the
//! same blocking connection the tray already owns — no `playerctl`, no
//! extra D-Bus connection, no UI-thread blocking.

use std::collections::HashMap;

use zbus::blocking::Proxy;
use zbus::zvariant::{OwnedValue, Value};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const MAX_TITLE: usize = 20;
const MAX_ARTIST: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Player {
    bus: String,
    status: String,
    title: String,
    artist: String,
    identity: String,
    art: Option<String>,
    /// Track length and current position in milliseconds (MPRIS reports
    /// microseconds); `None` when the player omits them.
    length_ms: Option<u64>,
    position_ms: Option<u64>,
    can_control: bool,
    can_previous: bool,
    can_next: bool,
}

/// What the dashboard shows about the selected player. Read only when
/// the dashboard is open (see `sensor_runner::read_all`): the bar's own
/// `media` sensor keeps using the compact `display_value`.
#[derive(Clone, Debug)]
pub(crate) struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub status: String,
    /// `mpris:artUrl` as reported (usually `file:///…`), never fetched.
    pub art: Option<String>,
    pub length_ms: Option<u64>,
    pub position_ms: Option<u64>,
    /// Transport capabilities: unavailable buttons read dimmed.
    pub can_control: bool,
    pub can_previous: bool,
    pub can_next: bool,
}

/// What the bar's `media` sensor renders: the transport buttons' state
/// and the truncated track text. The full text goes to the hover popup
/// (`media_details`), which is why position and length travel too.
#[derive(Clone, Debug)]
pub(crate) struct MediaState {
    pub status: String,
    pub title: String,
    pub artist: String,
    pub position_ms: Option<u64>,
    pub length_ms: Option<u64>,
    pub can_control: bool,
    pub can_previous: bool,
    pub can_next: bool,
}

/// The selected player as the bar needs it; `None` = no player (the
/// sensor hides).
pub(crate) fn media_state() -> Option<MediaState> {
    let player = selected()?;
    let title = if player.title.is_empty() {
        player.identity
    } else {
        player.title
    };
    Some(MediaState {
        status: player.status,
        title,
        artist: player.artist,
        position_ms: player.position_ms,
        length_ms: player.length_ms,
        can_control: player.can_control,
        can_previous: player.can_previous,
        can_next: player.can_next,
    })
}

/// The sensor's one-line text: title and artist, 20 chars each.
pub(crate) fn media_label(media: &MediaState) -> String {
    let title = truncate(&media.title, MAX_TITLE);
    let artist = truncate(&media.artist, MAX_ARTIST);
    match (title.is_empty(), artist.is_empty()) {
        (true, true) => String::new(),
        (true, false) => artist,
        (false, true) => title,
        (false, false) => format!("{title} — {artist}"),
    }
}

/// The played fraction for the hover popup's bar: `None` when the
/// player reports no position/length (the popup then shows no bar).
pub(crate) fn media_progress(media: &MediaState) -> Option<f32> {
    match (media.position_ms, media.length_ms) {
        (Some(position), Some(length)) if length > 0 => {
            Some((position as f32 / length as f32).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

/// The hover popup: full artist - title plus position/length when the
/// player reports both.
pub(crate) fn media_details(media: &MediaState) -> String {
    let track = match (media.artist.is_empty(), media.title.is_empty()) {
        (true, true) => String::new(),
        (true, false) => media.title.clone(),
        (false, true) => media.artist.clone(),
        (false, false) => format!("{} - {}", media.artist, media.title),
    };
    match (media.position_ms, media.length_ms) {
        (Some(position), Some(length)) => format!(
            "{track} {}/{}",
            crate::dashboard::duration(position),
            crate::dashboard::duration(length)
        ),
        _ => track,
    }
}

pub(crate) fn now_playing() -> Option<NowPlaying> {
    let player = selected()?;
    Some(NowPlaying {
        title: if player.title.is_empty() {
            player.identity
        } else {
            player.title
        },
        artist: player.artist,
        status: player.status,
        art: player.art,
        length_ms: player.length_ms,
        position_ms: player.position_ms,
        can_control: player.can_control,
        can_previous: player.can_previous,
        can_next: player.can_next,
    })
}

/// Whether any player is playing right now: the idle screensaver stays
/// off while media runs, so a windowed video (not fullscreen, no idle
/// inhibitor) is never covered. MPRIS is the one cross-player signal;
/// the trade-off is that music blocks it too.
pub(crate) fn any_playing() -> bool {
    any_playing_in(&players())
}

fn any_playing_in(players: &[Player]) -> bool {
    players.iter().any(|player| player.status == "Playing")
}

pub(crate) fn play_pause() {
    call("PlayPause", |_| true);
}

/// Wheel direction: up = previous, down = next.
pub(crate) fn step(up: bool) {
    if up {
        call("Previous", |player| player.can_previous);
    } else {
        call("Next", |player| player.can_next);
    }
}

fn call(method: &'static str, allowed: impl Fn(&Player) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let Some(player) = selected() else { return };
        if !player.can_control || !allowed(&player) {
            return;
        }
        let Some(conn) = crate::tray::action_connection() else {
            return;
        };
        let Ok(proxy) = Proxy::new(conn, player.bus.as_str(), PLAYER_PATH, PLAYER_IFACE) else {
            return;
        };
        if let Err(e) = proxy.call_method(method, &()) {
            eprintln!("[piuma] mpris: {method} failed: {e}");
        }
    });
}

fn selected() -> Option<Player> {
    selected_from(players())
}

fn players() -> Vec<Player> {
    let Some(conn) = crate::tray::action_connection() else {
        return Vec::new();
    };
    let Ok(bus) = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return Vec::new();
    };
    let Ok(reply) = bus.call_method("ListNames", &()) else {
        return Vec::new();
    };
    let Ok(names) = reply.body().deserialize::<Vec<String>>() else {
        return Vec::new();
    };
    names
        .into_iter()
        .filter(|name| name.starts_with(PREFIX))
        .filter(|name| name != "org.mpris.MediaPlayer2.playerctld")
        .filter_map(|name| player(conn, &name))
        .collect()
}

fn player(conn: &zbus::blocking::Connection, bus: &str) -> Option<Player> {
    let proxy = Proxy::new(conn, bus, PLAYER_PATH, PLAYER_IFACE).ok()?;
    let status: String = proxy.get_property("PlaybackStatus").ok()?;
    let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").unwrap_or_default();
    let title = meta_string(&metadata, "xesam:title").unwrap_or_default();
    let artist = meta_artists(&metadata);
    let art = meta_string(&metadata, "mpris:artUrl");
    let length_ms = meta_i64(&metadata, "mpris:length")
        .filter(|micros| *micros > 0)
        .map(|micros| micros as u64 / 1000);
    let position_ms = proxy
        .get_property::<i64>("Position")
        .ok()
        .filter(|micros| *micros >= 0)
        .map(|micros| micros as u64 / 1000);
    let identity = Proxy::new(conn, bus, PLAYER_PATH, ROOT_IFACE)
        .ok()
        .and_then(|root| root.get_property::<String>("Identity").ok())
        .unwrap_or_default();
    Some(Player {
        bus: bus.to_string(),
        status,
        title,
        artist,
        identity,
        art,
        length_ms,
        position_ms,
        can_control: proxy.get_property("CanControl").unwrap_or(false),
        can_previous: proxy.get_property("CanGoPrevious").unwrap_or(false),
        can_next: proxy.get_property("CanGoNext").unwrap_or(false),
    })
}

/// Playing first, then paused, then stopped; on equal status prefer a
/// native browser player over the Plasma browser bridge; bus name last
/// for a deterministic choice.
fn selected_from(mut players: Vec<Player>) -> Option<Player> {
    players.sort_by_key(|player| {
        (
            status_rank(&player.status),
            u8::from(is_browser_bridge(&player.bus)),
            player.bus.clone(),
        )
    });
    players.into_iter().next()
}

fn status_rank(status: &str) -> u8 {
    match status {
        "Playing" => 0,
        "Paused" => 1,
        "Stopped" => 2,
        _ => 3,
    }
}

fn is_browser_bridge(bus: &str) -> bool {
    bus.ends_with("plasma-browser-integration")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}...")
}

fn meta_string(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match Value::from(metadata.get(key)?.clone()) {
        Value::Str(value) => Some(value.as_str().to_string()),
        _ => None,
    }
}

/// An integer metadata value (`mpris:length` is a signed microsecond
/// count; some players send it unsigned).
fn meta_i64(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<i64> {
    match Value::from(metadata.get(key)?.clone()) {
        Value::I64(value) => Some(value),
        Value::U64(value) => i64::try_from(value).ok(),
        _ => None,
    }
}

fn meta_artists(metadata: &HashMap<String, OwnedValue>) -> String {
    let Some(value) = metadata.get("xesam:artist") else {
        return String::new();
    };
    match Value::from(value.clone()) {
        Value::Str(value) => value.as_str().to_string(),
        Value::Array(values) => values
            .iter()
            .filter_map(|value| match value {
                Value::Str(value) => Some(value.as_str().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(bus: &str, status: &str, title: &str, artist: &str) -> Player {
        Player {
            bus: bus.to_string(),
            status: status.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            identity: String::new(),
            art: None,
            length_ms: None,
            position_ms: None,
            can_control: true,
            can_previous: true,
            can_next: true,
        }
    }

    fn media(title: &str, artist: &str, position: Option<u64>, length: Option<u64>) -> MediaState {
        MediaState {
            status: "Playing".to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            position_ms: position,
            length_ms: length,
            can_control: true,
            can_previous: true,
            can_next: true,
        }
    }

    #[test]
    fn the_label_carries_title_and_artist_truncated_to_20_each() {
        let title = "abcdefghijklmnopqrstuvwxyz1234";
        let artist = "ABCDEFGHIJKLMNOPQRSTUVWXYZ5678";
        let text = media_label(&media(title, artist, None, None));
        let expected = format!("{}... — {}...", &title[..20], &artist[..20]);
        assert_eq!(text, expected);
        assert_eq!(media_label(&media("Song", "", None, None)), "Song");
        assert_eq!(media_label(&media("", "", None, None)), "");
    }

    /// The hover popup: full artist - title, then position/length when
    /// the player reports both.
    #[test]
    fn the_progress_fraction_is_clamped_and_needs_both_values() {
        assert_eq!(media_progress(&media("S", "A", Some(50_000), Some(100_000))), Some(0.5));
        assert_eq!(media_progress(&media("S", "A", Some(0), Some(100_000))), Some(0.0));
        assert_eq!(media_progress(&media("S", "A", Some(200_000), Some(100_000))), Some(1.0));
        assert_eq!(media_progress(&media("S", "A", Some(1), None)), None);
        assert_eq!(media_progress(&media("S", "A", None, Some(0))), None);
    }

    #[test]
    fn the_details_line_adds_position_and_length() {
        assert_eq!(
            media_details(&media("Song", "Artist", Some(61_000), Some(200_000))),
            "Artist - Song 1:01/3:20"
        );
        assert_eq!(
            media_details(&media("Song", "Artist", None, None)),
            "Artist - Song"
        );
        assert_eq!(media_details(&media("Song", "", None, None)), "Song");
    }

    /// The screensaver gate: any player mid-playback blocks it, paused
    /// or stopped players do not (the browser keeps them listed).
    #[test]
    fn any_playing_catches_a_playing_player() {
        assert!(!any_playing_in(&[]));
        assert!(!any_playing_in(&[player("a", "Paused", "A", "")]));
        assert!(!any_playing_in(&[player("a", "Stopped", "A", "")]));
        assert!(any_playing_in(&[
            player("a", "Paused", "A", ""),
            player("b", "Playing", "B", ""),
        ]));
    }

    #[test]
    fn selection_prefers_playing_then_native_over_bridge() {
        let selected = selected_from(vec![
            player("org.mpris.MediaPlayer2.vivaldi.instance1", "Paused", "A", ""),
            player("org.mpris.MediaPlayer2.plasma-browser-integration", "Playing", "A", ""),
        ])
        .expect("one player");
        assert_eq!(selected.bus, "org.mpris.MediaPlayer2.plasma-browser-integration");

        let selected = selected_from(vec![
            player("org.mpris.MediaPlayer2.plasma-browser-integration", "Paused", "A", ""),
            player("org.mpris.MediaPlayer2.vivaldi.instance1", "Paused", "A", ""),
        ])
        .expect("one player");
        assert_eq!(selected.bus, "org.mpris.MediaPlayer2.vivaldi.instance1");
    }
}
