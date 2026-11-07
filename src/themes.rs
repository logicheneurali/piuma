//! Colour themes: ten embedded config fragments plus the tiny
//! `theme.json` that names the active one. The user's `piuma.json` is
//! never written: the bar and the setup only read it, and the theme is
//! applied over its colour keys in memory.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config::{Config, SectionDefaults, SensorDefaults};

/// One embedded theme, parsed from `themes/*.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct Theme {
    /// File stem, the value `theme.json` carries.
    #[serde(skip)]
    pub slug: &'static str,
    pub name: String,
    pub panel: ThemePanel,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ThemePanel {
    pub theme_mode: Option<String>,
    pub wallpaper_palette: Option<bool>,
    pub default_bg_color: Option<String>,
    pub default_text_color: Option<String>,
    #[serde(default)]
    pub sensor_defaults: SensorDefaults,
    #[serde(default)]
    pub section_defaults: SectionDefaults,
}

/// The embedded files, in picker order.
const FILES: [(&str, &str); 10] = [
    ("bianco-su-nero", include_str!("../themes/01-bianco-su-nero.json")),
    ("nero-su-bianco", include_str!("../themes/02-nero-su-bianco.json")),
    ("nero-su-arancione", include_str!("../themes/03-nero-su-arancione.json")),
    ("bianco-su-verde", include_str!("../themes/04-bianco-su-verde.json")),
    ("ciano-su-blu-scuro", include_str!("../themes/05-ciano-su-blu-scuro.json")),
    ("viola-su-grafite", include_str!("../themes/06-viola-su-grafite.json")),
    ("lavanda-su-indaco", include_str!("../themes/07-lavanda-su-indaco.json")),
    ("ambra-su-carbone", include_str!("../themes/08-ambra-su-carbone.json")),
    ("ghiaccio-su-ardesia", include_str!("../themes/09-ghiaccio-su-ardesia.json")),
    ("blu-su-mezzanotte", include_str!("../themes/10-blu-su-mezzanotte.json")),
];

/// Every theme, parsed once.
pub fn all() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        FILES
            .iter()
            .map(|(slug, json)| {
                let mut theme: Theme =
                    serde_json::from_str(json).unwrap_or_else(|e| panic!("theme {slug}: {e}"));
                theme.slug = slug;
                theme
            })
            .collect()
    })
}

pub fn find(slug: &str) -> Option<&'static Theme> {
    all().iter().find(|theme| theme.slug == slug)
}

/// Apply `theme` over `config`'s colour keys, in memory. Explicit
/// per-section/sensor colours still win (the style chain checks them
/// first); the wallpaper palette is suspended so it cannot repaint the
/// theme. This is the only mutation of the user's config, and it never
/// touches the file.
pub fn apply(config: &mut Config, slug: &str) -> Option<&'static Theme> {
    let theme = find(slug)?;
    let t = &theme.panel;
    let panel = &mut config.panel;
    if let Some(value) = &t.default_bg_color {
        panel.default_bg_color = Some(value.clone());
    }
    if let Some(value) = &t.default_text_color {
        panel.default_text_color = Some(value.clone());
    }
    if let Some(value) = &t.theme_mode {
        panel.theme_mode = Some(value.clone());
    }
    panel.wallpaper_palette = Some(false);

    let sensor = panel.sensor_defaults.get_or_insert_with(SensorDefaults::default);
    sensor.text_color = t.sensor_defaults.text_color.clone();
    sensor.text_color_hover = t.sensor_defaults.text_color_hover.clone();
    sensor.bg_color = t.sensor_defaults.bg_color.clone();
    sensor.bg_color_hover = t.sensor_defaults.bg_color_hover.clone();
    sensor.border_color = t.sensor_defaults.border_color.clone();
    sensor.border_color_hover = t.sensor_defaults.border_color_hover.clone();

    let section = panel
        .section_defaults
        .get_or_insert_with(SectionDefaults::default);
    section.bg_color = t.section_defaults.bg_color.clone();
    section.text_color = t.section_defaults.text_color.clone();
    section.border_color = t.section_defaults.border_color.clone();
    Some(theme)
}

/// `~/.config/piuma/theme.json`: the active theme's slug. Separate from
/// the user's config so no theme switch can ever rewrite `piuma.json`.
pub fn theme_path() -> Option<PathBuf> {
    crate::xdg::config().map(|dir| dir.join("piuma").join("theme.json"))
}

#[derive(Default, Deserialize, Serialize)]
struct ActiveTheme {
    #[serde(default)]
    theme: Option<String>,
}

/// The active slug, `None` for custom colours.
pub fn read_active() -> Option<String> {
    read_at(&theme_path()?)
}

/// The active theme resolved to its embedded entry, `None` = custom.
pub fn active() -> Option<&'static str> {
    read_active().and_then(|slug| find(&slug).map(|theme| theme.slug))
}

/// Persist the active slug (`None` = custom). Atomic, like the setup's
/// config save; no answer when there is no config directory.
pub fn write_active(slug: Option<&str>) -> Result<(), String> {
    write_at(&theme_path().ok_or("no config directory")?, slug)
}

fn read_at(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let active: ActiveTheme = serde_json::from_str(&text).ok()?;
    active
        .theme
        .filter(|slug| !slug.trim().is_empty() && find(slug).is_some())
}

fn write_at(path: &std::path::Path, slug: Option<&str>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("theme directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&ActiveTheme {
        theme: slug.map(str::to_string),
    })
    .map_err(|e| format!("theme file: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json + "\n")
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("theme write: {e}"))
}

/// Change stamp of `theme.json` (mtime + length), for the bar's watch.
pub fn state() -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(theme_path()?).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ten files parse, the slugs are unique, and applying one only
    /// changes the colour keys: sections, panels and sensors are left
    /// exactly as the user configured them.
    #[test]
    fn themes_apply_only_colours() {
        assert_eq!(all().len(), 10);
        let mut seen = std::collections::HashSet::new();
        for theme in all() {
            assert!(seen.insert(theme.slug), "duplicate slug {}", theme.slug);
        }

        let json = r##"{
            "panel": {
                "default_bg_color": "#000000",
                "default_text_color": "#ffffff",
                "wallpaper_palette": true,
                "sections": [{"name": "center", "sensors": []}]
            },
            "panels": []
        }"##;
        let mut cfg: Config = serde_json::from_str(json).unwrap();
        let purple = apply(&mut cfg, "viola-su-grafite").expect("the purple theme");
        assert_eq!(cfg.panel.default_bg_color.as_deref(), purple.panel.default_bg_color.as_deref());
        assert_eq!(cfg.panel.wallpaper_palette, Some(false), "palette suspended");
        let sensors = cfg.panel.sensor_defaults.as_ref().expect("sensor defaults");
        assert_eq!(
            sensors.bg_color.as_deref(),
            purple.panel.sensor_defaults.bg_color.as_deref()
        );
        // Structure untouched.
        assert_eq!(cfg.panel.sections.len(), 1);
        assert_eq!(cfg.panel.sections[0].name, "center");

        // An unknown slug changes nothing.
        let mut custom: Config = serde_json::from_str(json).unwrap();
        assert!(apply(&mut custom, "nope").is_none());
        assert_eq!(custom.panel.default_bg_color.as_deref(), Some("#000000"));
        assert_eq!(custom.panel.wallpaper_palette, Some(true));
    }

    /// The active-theme file round-trips and an unknown slug reads as
    /// custom (it must never reach the bar as a broken theme). Written
    /// under a unique temp path: a test never touches the user's file.
    #[test]
    fn active_theme_roundtrips() {
        let dir = std::env::temp_dir().join(format!("piuma-theme-{}", std::process::id()));
        let path = dir.join("theme.json");
        write_at(&path, Some("ghiaccio-su-ardesia")).expect("write");
        assert_eq!(read_at(&path).as_deref(), Some("ghiaccio-su-ardesia"));
        write_at(&path, Some("does-not-exist")).expect("write");
        assert_eq!(read_at(&path), None, "unknown slugs are ignored");
        write_at(&path, None).expect("write");
        assert_eq!(read_at(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
