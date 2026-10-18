//! Themed system icons (roadmap N1): freedesktop icon-name → file lookup
//! for the sensors, on the same theme rule as the tray. Lookups are
//! memoized per (theme, size, name) — misses too — so the per-second
//! section rebuild only clones a cached `PathBuf` and never touches the
//! disk again; while no sensor declares an icon nothing here runs at all
//! (the gate lives in `main::icons_theme`).

use iced::Element;
use iced::Length;
use iced::widget::{image, svg};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// theme → size → icon name → resolved file (`None` = remembered miss).
type Cache = HashMap<String, HashMap<u16, HashMap<String, Option<PathBuf>>>>;

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

/// The user icon theme, resolved once per process (the fallback spawns
/// `gsettings`): the GTK setting, then `hicolor`. Tray and sensors share
/// this single rule.
pub fn icon_theme() -> &'static str {
    static THEME: OnceLock<String> = OnceLock::new();
    THEME
        .get_or_init(|| {
            // The raw setting value IS the lookup key: freedesktop-icons
            // keys themes by directory name, while its own
            // `default_theme_gtk()` returns the index's display `Name=`
            // ("breeze" → "Breeze"), which then misses every lookup and
            // silently degrades to hicolor + pixmaps. Read the setting
            // exactly as the lookup expects it.
            std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "icon-theme"])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|raw| raw.trim().trim_matches('\'').to_string())
                .filter(|name| !name.is_empty())
                // Absent setting: hicolor ships on every freedesktop system.
                .unwrap_or_else(|| "hicolor".to_string())
        })
        .as_str()
}

/// The cache map, recovered from poisoning (a panic while holding it can
/// only make the cache stale, never wrong).
fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Resolve `name` under `theme` at `size` to a file (svg preferred by the
/// theme, otherwise the nearest sized bitmap). The first call per key pays
/// the theme walk, every later one is a map hit — including misses, so an
/// unknown icon is not re-searched every tick. Only the size actually in
/// use is kept: the sensor icon size follows the config, so other
/// buckets mean the config changed and the next tick re-resolves
/// everything anyway.
pub fn resolve(theme: &str, name: &str, size: u16) -> Option<PathBuf> {
    {
        let cache = cache();
        if let Some(hit) = cache
            .get(theme)
            .and_then(|by_size| by_size.get(&size))
            .and_then(|by_name| by_name.get(name))
        {
            return hit.clone();
        }
    }
    let found = freedesktop_icons::lookup(name)
        .with_size(size)
        .with_theme(theme)
        .find();
    let mut cache = cache();
    let by_size = cache.entry(theme.to_string()).or_default();
    by_size.retain(|s, _| *s == size);
    by_size
        .entry(size)
        .or_default()
        .insert(name.to_string(), found.clone());
    found
}

/// The widget for a resolved file: SVGs go to the svg widget, bitmaps to
/// the image widget (same rule as the tray); a square box with the icon's
/// aspect kept inside it.
pub fn widget<'a>(path: &Path, size: f32) -> Element<'a, crate::Message> {
    let (width, height) = (Length::Fixed(size), Length::Fixed(size));
    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("svg")) {
        svg(svg::Handle::from_path(path.to_path_buf()))
            .width(width)
            .height(height)
            .into()
    } else {
        image(image::Handle::from_path(path.to_path_buf()))
            .width(width)
            .height(height)
            .into()
    }
}

/// Number of remembered resolutions (tests assert the miss path does not
/// walk the disk again).
#[cfg(test)]
pub(crate) fn cache_len() -> usize {
    cache()
        .values()
        .flat_map(|by_size| by_size.values())
        .map(HashMap::len)
        .sum()
}

// ── icon sizing: one formula for tray, sensor rows and the setup preview ──

/// Padding the section content Row/Column adds on every side: the layout
/// draws it around a section's title and sensors, so it is chrome an icon
/// has to fit inside (single source — the formula must match what the
/// view really draws).
pub const SECTION_CONTENT_PAD: u16 = 4;

/// Thickness (px) a section title eats when it shares the thickness axis:
/// a vertically laid out section draws the title above its sensors, as
/// text (font + 2) in a box padded `SECTION_CONTENT_PAD` on both sides.
/// A horizontal layout or an empty title draws nothing on that axis.
pub fn stacked_title_extra(layout: &str, name: &str, font: u32) -> u32 {
    if layout == "vertical" && !name.is_empty() {
        font + 2 + 2 * u32::from(SECTION_CONTENT_PAD)
    } else {
        0
    }
}

/// Icon edge (px) that fits inside a sensor row: panel thickness minus
/// everything the layout actually spends on the thickness axis inside it —
/// the section content padding on both sides, the sensor's own padding, and
/// the section title when sensors are stacked vertically (`title_extra`) —
/// clamped to a usable range so a hairline bar and a very thick bar both
/// stay sane. Borders are not subtracted: iced draws them inside the box (a
/// pill measures the same with or without one), so they never eat thickness —
/// the sensor row really is this tall, and this is the height the row's text
/// already fills. Tray icons, sensor icons and the setup's live preview all
/// size to this one number.
pub fn row_icon_target(thickness: u32, title_extra: u32, padding: u32) -> u16 {
    let chrome = 2 * (u32::from(SECTION_CONTENT_PAD) + padding) + title_extra;
    thickness.saturating_sub(chrome).clamp(4, 48) as u16
}

/// How many icon slots of `slot` px fit in `available` px: the cap the
/// taskbar and tray rows use, so their content can never paint over the
/// neighbouring sensors (iced does not clip a Row that outgrows its
/// box). At least one slot, so even a hairline bar shows the first icon.
pub fn icon_capacity(available: u32, slot: u32) -> u16 {
    if slot == 0 {
        return 1;
    }
    (available / slot).clamp(1, u32::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A miss is remembered: the first call walks the theme once, the
    /// second one is a pure map hit (same `None`, same cache size), and a
    /// hit agrees with itself across calls. One test only, so the global
    /// cache is never asserted from two threads at once.
    #[test]
    fn resolve_is_stable_and_remembers_misses() {
        let name = "piuma-definitely-missing-icon";
        assert!(resolve("hicolor", name, 16).is_none());
        let after_first = cache_len();
        assert!(resolve("hicolor", name, 16).is_none());
        assert_eq!(cache_len(), after_first, "the second call must not re-walk");

        // `folder` ships with hicolor on every freedesktop system; if even
        // that is missing the lookup returns `None` twice — what must hold
        // is that the cache gives the same answer every time.
        let first = resolve("hicolor", "folder", 48);
        assert_eq!(first, resolve("hicolor", "folder", 48));
    }

    /// A bare 26px bar: only the section content padding (4px a side).
    #[test]
    fn row_icon_target_subtracts_section_padding() {
        assert_eq!(row_icon_target(26, 0, 0), 18);
    }

    /// The sensor's own padding (6px a side) subtracts on top of it.
    #[test]
    fn row_icon_target_subtracts_sensor_padding() {
        assert_eq!(row_icon_target(44, 0, 6), 24);
    }

    /// A stacked section title eats thickness as well (font 14 → 14+2+8).
    #[test]
    fn row_icon_target_subtracts_stacked_title() {
        assert_eq!(row_icon_target(44, 24, 0), 12);
    }

    /// Clamped: never smaller than 4px, never larger than 48px.
    #[test]
    fn row_icon_target_clamps() {
        assert_eq!(row_icon_target(10, 0, 0), 4);
        assert_eq!(row_icon_target(400, 0, 0), 48);
    }

    /// The title term applies only to a vertical layout with a name to
    /// draw (font 14 → 14 + 2 + 2×4 of box).
    #[test]
    fn stacked_title_extra_only_for_named_vertical_sections() {
        assert_eq!(stacked_title_extra("horizontal", "Side", 14), 0);
        assert_eq!(stacked_title_extra("vertical", "", 14), 0);
        assert_eq!(stacked_title_extra("vertical", "Side", 14), 24);
    }

    /// Whole slots only, never zero, saturated at `u16` — a bar too
    /// narrow still shows the first icon.
    #[test]
    fn icon_capacity_fits_whole_slots() {
        assert_eq!(icon_capacity(100, 52), 1);
        assert_eq!(icon_capacity(104, 52), 2);
        assert_eq!(icon_capacity(0, 52), 1);
        assert_eq!(icon_capacity(100, 0), 1);
        assert_eq!(icon_capacity(u32::MAX, 1), u16::MAX);
    }
}
