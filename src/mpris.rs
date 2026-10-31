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
const MAX_TITLE: usize = 25;
const MAX_ARTIST: usize = 25;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Player {
    bus: String,
    status: String,
    title: String,
    artist: String,
    identity: String,
    can_control: bool,
    can_previous: bool,
    can_next: bool,
}

/// The text shown by the sensor: status glyph, title and artist. An
/// empty string means "no player" and hides the sensor.
pub(crate) fn display_value() -> String {
    selected().map(|player| format_player(&player)).unwrap_or_default()
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

fn format_player(player: &Player) -> String {
    let title = if player.title.is_empty() {
        &player.identity
    } else {
        &player.title
    };
    if title.is_empty() {
        return String::new();
    }
    let title = truncate(title, MAX_TITLE);
    let artist = truncate(&player.artist, MAX_ARTIST);
    let body = if artist.is_empty() {
        title
    } else {
        format!("{title} — {artist}")
    };
    let glyph = match player.status.as_str() {
        "Playing" => "▶",
        "Paused" => "⏸",
        _ => "⏹",
    };
    format!("{glyph}  {body}")
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
            can_control: true,
            can_previous: true,
            can_next: true,
        }
    }

    #[test]
    fn format_carries_status_title_and_artist() {
        assert_eq!(
            format_player(&player("a", "Playing", "Song", "Artist")),
            "▶  Song — Artist"
        );
        assert_eq!(
            format_player(&player("a", "Paused", "Song", "")),
            "⏸  Song"
        );
        assert_eq!(format_player(&player("a", "Stopped", "", "")), "");
    }

    #[test]
    fn title_and_artist_are_truncated_to_25_each() {
        let title = "abcdefghijklmnopqrstuvwxyz1234";
        let artist = "ABCDEFGHIJKLMNOPQRSTUVWXYZ5678";
        let text = format_player(&player("a", "Playing", title, artist));
        let expected = format!(
            "▶  {}... — {}...",
            &title[..25],
            &artist[..25]
        );
        assert_eq!(text, expected);
        assert!(text.starts_with("▶  "));
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
