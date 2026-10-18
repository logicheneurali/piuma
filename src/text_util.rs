//! Font resolution: the configured family (or the bundled default)
//! and the two fonts the bar draws with.

use crate::config;
use crate::{DEFAULT_FONT, FONT_NAMES};
use iced::font::Font;

/// The `'static` family name of `name` (roadmap M2): one leak per
/// distinct family, so `Font::with_name` can take it repeatedly.
pub(crate) fn family_name_static(name: &str) -> &'static str {
    let mut names = FONT_NAMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((leaked, _)) = names.iter().find(|(_, original)| original == name) {
        return *leaked;
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
