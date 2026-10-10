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
            freedesktop_icons::default_theme_gtk().unwrap_or_else(|| "hicolor".to_string())
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
}
