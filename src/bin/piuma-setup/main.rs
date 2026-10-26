//! Piuma Setup - GUI Configuration Tool
//! A visual tool to configure Piuma panel settings using iced 0.14

use iced::Font;
use serde_json::Value as JsonValue;
use std::fs;

// piuma's real config schema (same package, different bin target): the
// save path validates against it before writing, so a config piuma
// would reject is reported here instead of failing silently at load.
// config.rs serves both binaries: the bar-only helpers (colour parsing,
// default config, file loading) legitimately sit unused in the setup.
#[allow(dead_code)]
#[path = "../../config.rs"]
mod config;

// Freedesktop autostart listing and the per-app switch (roadmap2 #2):
// the bar-only runner (spawn/launch) is not this bin's job — the
// module-wide allow keeps it out of the warning baseline.
#[allow(dead_code)]
#[path = "../../autostart.rs"]
mod autostart;

// Freedesktop icon-name → file lookup shared with the bar (roadmap N1):
// the setup renders the same resolve/widget pair at the same theme and
// size, so the icon preview shows exactly what the bar will draw. The
// bar-only view helpers (font_icon_edge, icons_per_line) sit unused
// here like config's and autostart's: module-wide allow, same baseline.
#[allow(dead_code)]
#[path = "../../icons.rs"]
mod icons;

mod controls;
mod hyprland_wallpaper;
mod model;
mod modal;
mod state;
mod style;
mod update;
mod view;
mod view_layouts;
mod view_panel;
mod view_sections;
mod view_wallpaper;

// Re-exported at the root: the shared `icons.rs` (also used by the bar)
// references `crate::Message`.
pub use crate::state::Message;
use crate::state::{file_path, PiumaSetup, DEFAULT_FILE};
use crate::style::{theme, SETUP_DEFAULT_FONT};

/// The GUI font from `piuma.json` (`panel.font` + `panel.bold`, same
/// fallbacks as the bar): iced needs the default font before the state
/// boots, so the file is read here once — the family name is leaked once
/// at startup (iced wants a `'static` name, like `family_name_static` in
/// src/main.rs, but here the font never changes at runtime).
fn gui_font() -> Font {
    let panel = fs::read_to_string(file_path(DEFAULT_FILE))
        .ok()
        .and_then(|c| serde_json::from_str::<JsonValue>(&c).ok())
        .and_then(|v| v.get("panel").cloned());
    let name = panel
        .as_ref()
        .and_then(|p| p.get("font").and_then(JsonValue::as_str))
        .unwrap_or(SETUP_DEFAULT_FONT)
        .to_owned();
    let bold = panel
        .as_ref()
        .and_then(|p| p.get("bold").and_then(JsonValue::as_bool))
        .unwrap_or(false);
    Font {
        family: iced::font::Family::Name(&*Box::leak(name.into_boxed_str())),
        weight: if bold {
            iced::font::Weight::Bold
        } else {
            iced::font::Weight::Normal
        },
        ..Font::DEFAULT
    }
}

fn app() -> iced::Result {
    iced::application(PiumaSetup::new, update::update, view::view)
        .title("Piuma Setup")
        .theme(theme)
        .default_font(gui_font())
        .window_size((920, 680))
        .resizable(true)
        .run()
}

fn main() -> iced::Result {
    app()
}

#[cfg(test)]
mod tests;
