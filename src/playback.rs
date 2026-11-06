//! Whether a video (or other media) is playing, for the screensaver's
//! gate. Two signals, layered so the check stays portable:
//!
//! 1. The sound server's streams (`pactl list sink-inputs`): PulseAudio
//!    and PipeWire's pulse layer both report `media.role`, so a browser
//!    playing a video says `Movie` while music says `Music`. When streams
//!    are reported, only a video role blocks — music does not.
//! 2. MPRIS: when no stream is visible (a muted video, a player writing
//!    straight to ALSA, no sound server at all) any player in `Playing`
//!    blocks. This is the backend-independent fallback.
//!
//! Known limits: a bare-ALSA system has no stream metadata, so a silent
//! windowed video from a player without MPRIS is invisible to both.

use crate::mpris;

/// Roles that mean "a video is running". Chromium-based browsers send
/// `Movie` for `<video>` and Firefox `video`; `Game` covers full-screen
/// games feeding the mixer.
fn is_video(role: &str) -> bool {
    matches!(role.to_ascii_lowercase().as_str(), "movie" | "video" | "game")
}

/// The `media.role` of every sink input currently streaming, lowercased
/// (`Corked: yes` streams are skipped: paused media does not block).
/// Only the `Properties` block carries the role.
fn active_roles(output: &str) -> Vec<String> {
    let mut roles = Vec::new();
    let mut corked = false;
    let mut in_properties = false;
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Sink Input #") {
            corked = false;
            in_properties = false;
            continue;
        }
        if line == "Properties:" {
            in_properties = true;
            continue;
        }
        if !in_properties {
            if let Some(state) = line.strip_prefix("Corked:") {
                corked = state.trim() == "yes";
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("media.role") {
            let role = value.trim_start_matches([' ', '=']).trim().trim_matches('"');
            if !corked && !role.is_empty() {
                roles.push(role.to_ascii_lowercase());
            }
        }
    }
    roles
}

/// The streaming roles from the sound server, or `None` when there is
/// no server or `pactl` is missing (bare ALSA).
fn pactl_roles() -> Option<Vec<String>> {
    let out = std::process::Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(active_roles(&String::from_utf8_lossy(&out.stdout)))
}

/// The decision, without I/O: a visible video role blocks; visible
/// streams without one (music) do not; with no stream information the
/// MPRIS fallback decides.
fn blocks_from(roles: Option<&[String]>, mpris_playing: bool) -> bool {
    match roles {
        Some(roles) if !roles.is_empty() => roles.iter().any(|role| is_video(role)),
        _ => mpris_playing,
    }
}

/// Whether the idle screensaver must stay off.
pub(crate) fn blocks_screensaver() -> bool {
    let roles = pactl_roles();
    blocks_from(roles.as_deref(), mpris::any_playing())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Sink Input #12
	Driver: PipeWire
	Corked: no
	Properties:
		media.name = "YouTube"
		media.role = "Movie"
		application.name = "Vivaldi"

Sink Input #13
	Driver: PipeWire
	Corked: yes
	Properties:
		media.role = "Movie"

Sink Input #14
	Driver: PipeWire
	Corked: no
	Properties:
		media.role = "Music"
		application.name = "Spotify"
"#;

    /// Only streaming (uncorked) inputs count, and the role is read from
    /// the `Properties` block: a paused video is not "playing".
    #[test]
    fn only_uncorked_streams_report_their_role() {
        assert_eq!(active_roles(SAMPLE), vec!["movie", "music"]);
        assert!(active_roles("").is_empty());
    }

    /// A video role blocks even with music next to it; music alone does
    /// not; no stream information falls back to MPRIS.
    #[test]
    fn video_blocks_music_does_not() {
        let video = vec!["movie".to_string()];
        let music = vec!["music".to_string()];
        assert!(blocks_from(Some(&video), false));
        assert!(blocks_from(Some(&["music".into(), "movie".into()]), false));
        assert!(!blocks_from(Some(&music), true), "music is allowed");
        assert!(blocks_from(Some(&[]), true), "no stream: MPRIS decides");
        assert!(!blocks_from(Some(&[]), false));
        assert!(blocks_from(None, true), "no sound server: MPRIS decides");
        assert!(!blocks_from(None, false));
        assert!(is_video("VIDEO"));
        assert!(is_video("game"));
        assert!(!is_video("music"));
    }
}
