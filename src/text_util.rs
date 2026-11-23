//! Font resolution: the configured family (or the bundled default)
//! and the two fonts the bar draws with.

use crate::config;
use crate::DEFAULT_FONT;
use iced::font::Font;

/// Configured family names kept alive for [`Font::Name`]'s `'static`
/// bound: a name is leaked the first time it is used and the same
/// pointer is reused after, so reloads — even alternating between two
/// fonts — never leak a fresh copy per change.
pub(crate) static FONT_NAMES: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());

/// The `'static` family name of `name` (roadmap M2): one leak per
/// distinct family, so `Font::with_name` can take it repeatedly.
pub(crate) fn family_name_static(name: &str) -> &'static str {
    let mut names = FONT_NAMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((leaked, _)) = names.iter().find(|(_, original)| original == name) {
        return leaked;
    }
    let leaked = Box::leak(name.to_string().into_boxed_str());
    names.push((leaked, name.to_string()));
    leaked
}

/// The panel font exactly as configured: family from `panel.font` (default
/// [`DEFAULT_FONT`]) and the `panel.bold` flag as the weight (roadmap M2).
/// Single source for every bar text and for the M5 width measurement.
pub(crate) fn panel_font(config: &config::Config) -> Font {
    let weight = if config.panel.bold.unwrap_or(false) {
        iced::font::Weight::Bold
    } else {
        iced::font::Weight::Normal
    };
    Font {
        weight,
        ..Font::with_name(family_name_static(
            config.panel.font.as_deref().unwrap_or(DEFAULT_FONT),
        ))
    }
}

/// The font of the sensors' value texts: `panel.value_font` when set
/// (a monospaced family keeps the digits one width wide, so the
/// values never jitter), the panel font otherwise — same weight.
pub(crate) fn value_font(config: &config::Config) -> Font {
    let weight = if config.panel.bold.unwrap_or(false) {
        iced::font::Weight::Bold
    } else {
        iced::font::Weight::Normal
    };
    match config.panel.value_font.as_deref() {
        Some(name) => Font {
            weight,
            ..Font::with_name(family_name_static(name))
        },
        None => panel_font(config),
    }
}

/// `text` cut to `max` characters; the cut one is replaced by an ellipsis,
/// so the result never exceeds `max` (the bar's labels, the notification
/// cards and the dashboard share this rule).
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_the_limit() {
        assert_eq!(truncate_chars("short", 10), "short");
        let out = truncate_chars("0123456789abcdef", 8);
        assert_eq!(out.chars().count(), 8);
        assert!(out.ends_with('…'));
        assert_eq!(truncate_chars("0123456789", 5), "0123…");
    }
}
