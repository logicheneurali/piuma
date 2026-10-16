//! Configuration loading from JSON file.
//!
//! Search order:
//! 1. CLI-provided path (`-c <path>` / `--config <path>`)
//! 2. `$XDG_CONFIG_HOME/piuma/piuma.json`
//! 3. `$HOME/.config/piuma/piuma.json`

use iced::Color;
use std::path::PathBuf;

// ── colour helpers ─────────────────────────────────────────────────

/// Parse a float colour string like `"0.1 0.1 0.1 0.9"` or hex
/// `"#abc"`, `"#ff0000"` / `"#ff0000ff"` (alpha last, as piuma-setup previews).
pub fn parse_color(value: &str) -> Result<Color, ()> {
    let value = value.trim();
    // Hex format: #RGB, #RRGGBB or #RRGGBBAA
    if value.starts_with('#') {
        let hex = &value[1..];
        // Slices below are byte based: reject non-ASCII instead of panicking
        // on a multi-byte boundary (same guard as piuma-setup's hex_color).
        if !hex.is_ascii() {
            return Err(());
        }
        let parse_hex = |s: &str, offset: usize| -> Option<u8> {
            u8::from_str_radix(&s[offset..offset + 2], 16).ok()
        };
        match hex.len() {
            3 => {
                // Shorthand "#rgb": each digit is duplicated, alpha opaque.
                let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).unwrap_or(0);
                let (r, g, b) = (nibble(0) * 17, nibble(1) * 17, nibble(2) * 17);
                Ok(Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0))
            }
            6 => {
                let r = parse_hex(hex, 0).unwrap_or(0);
                let g = parse_hex(hex, 2).unwrap_or(0);
                let b = parse_hex(hex, 4).unwrap_or(0);
                Ok(Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0))
            }
            8 => {
                let r = parse_hex(hex, 0).unwrap_or(0);
                let g = parse_hex(hex, 2).unwrap_or(0);
                let b = parse_hex(hex, 4).unwrap_or(0);
                let a = parse_hex(hex, 6).unwrap_or(255);
                Ok(Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0))
            }
            _ => Err(()),
        }
    } else {
        // ARGB float string: "r g b a"
        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.len() >= 4 {
            let r: f32 = parts[0].parse().unwrap_or(0.0);
            let g: f32 = parts[1].parse().unwrap_or(0.0);
            let b: f32 = parts[2].parse().unwrap_or(0.0);
            let a: f32 = parts[3].parse().unwrap_or(1.0);
            Ok(Color::from_rgba(r, g, b, a))
        } else {
            Err(())
        }
    }
}

/// Numeric style field with the **-1 = disabled** rule: `None` (field missing
/// from the config) or a negative value does not activate the field (it does
/// not override `panel.sensor_defaults`); `0..=N` is a valid value that
/// activates padding / border / radius with that amount.
pub fn style_num(value: Option<i32>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

/// Default panel thickness on the cross axis, in pixels: what
/// `panel.thickness` falls back to when the config does not set it
/// (shared by the bar's surface size and the setup's icon preview, so
/// both follow one number).
pub const DEFAULT_HEIGHT: u32 = 32;

// ── config structs ──────────────────────────────────────────────────

use serde::Deserialize;

#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names)]
pub struct Config {
    pub panel: PanelConfig,
}

#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names)]
pub struct PanelConfig {
    pub font: Option<String>,
    pub font_size: Option<u32>,
    /// Bold panel text (roadmap M2); absent = false.
    pub bold: Option<bool>,
    /// Font family for the sensors' value text only; absent = the panel
    /// font. A monospaced family makes the digits one width wide, so
    /// the values never jitter as they change.
    pub value_font: Option<String>,
    /// Theme used to resolve the sensors' `icon` names (roadmap N1);
    /// absent = the tray's rule (GTK icon-theme setting, then `hicolor`).
    pub icon_theme: Option<String>,
    /// Launch the freedesktop autostart applications with the bar
    /// (roadmap N2); absent = false (nothing is scanned or spawned).
    pub autostart: Option<bool>,
    /// Freedesktop `.desktop` files the bar must NOT launch: the user's
    /// own per-app selection, managed by piuma-setup's tab. Absent or
    /// empty = no suppression beyond the spec's own switches (`Hidden`,
    /// `OnlyShowIn`, … keep deciding on their own). Names whose file is
    /// gone are pruned at the next start, before anything is launched.
    /// Piuma never writes the autostart directories: other desktop
    /// environments keep their own, unaffected view.
    pub autostart_disabled: Option<Vec<String>>,
    /// Generate the GTK theme bridge from the bar's style (roadmap N3);
    /// absent = false (no file is written or removed).
    pub app_theme: Option<bool>,
    /// Desktop background the bar renders itself on the background layer
    /// (roadmap2 #1); absent = disabled (no window is opened).
    pub wallpaper: Option<WallpaperConfig>,
    pub section_layout: Option<String>,
    pub default_section_weight: Option<u32>,
    pub default_bg_color: Option<String>,
    pub default_text_color: Option<String>,
    pub anchor: Option<String>,
    pub align: Option<String>,
    /// Panel thickness (fixed surface dimension) in px; `None` = 32.
    pub thickness: Option<u32>,
    /// Style applied to every sensor; per-sensor values take precedence.
    pub sensor_defaults: Option<SensorDefaults>,
    /// Style applied to every section; per-section values take precedence.
    pub section_defaults: Option<SectionDefaults>,
    /// Freedesktop Desktop Notifications server (`notifications.rs`);
    /// present ⇔ the D-Bus server runs.
    pub notifications: Option<NotificationsConfig>,
    pub sections: Vec<SectionConfig>,
}

/// `panel.wallpaper`: the desktop background the bar draws on the
/// background layer — no external wallpaper tool is involved. Every
/// field defaults so a partial block (or none at all) keeps old
/// configs working: disabled, no path, no window.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct WallpaperConfig {
    /// Whether the bar shows the wallpaper at all.
    #[serde(default)]
    pub enabled: bool,
    /// Absolute path of the image, drawn with a cover fit over the
    /// whole output.
    #[serde(default)]
    pub path: String,
}

/// `panel.notifications`: tunes the D-Bus server (the block's presence
/// is what enables it). All fields optional — defaults apply.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct NotificationsConfig {
    /// Auto-expiry used when an app passes `-1` (default 5000 ms).
    pub default_timeout_ms: Option<u64>,
    /// Entries kept for the sensor's scroll history (default 20).
    pub history_size: Option<u32>,
    /// Honour `replaces_id` (default true; false records every call as a
    /// new notification).
    pub replace_existing: Option<bool>,
    /// Popup card style: absent/`-1` numeric keys fall back to the popup
    /// defaults (12px, white text on black, grey border) at render time.
    pub font_size: Option<u32>,
    pub text_color: Option<String>,
    pub text_color_hover: Option<String>,
    pub bg_color: Option<String>,
    pub bg_color_hover: Option<String>,
    pub border_color: Option<String>,
    pub border_color_hover: Option<String>,
    pub border_width: Option<i32>,
    pub border_radius: Option<i32>,
    pub border_type: Option<String>,
    pub padding: Option<i32>,
    /// Popup screen position (roadmap M4): absent/`auto` (or any unknown
    /// value) keeps the historic layout — opposite corner of the bar, right
    /// side; the six explicit keys are `top-left`, `top-center`, `top-right`,
    /// `bottom-left`, `bottom-center`, `bottom-right`.
    pub popup_position: Option<String>,
    /// Horizontal screen-edge margin in px for the popup (roadmap M4),
    /// absent = 8 (the historic value).
    pub popup_margin_h: Option<i32>,
    /// Vertical screen-edge margin in px for the popup (roadmap M4),
    /// absent = 8 (the historic value).
    pub popup_margin_v: Option<i32>,
}

/// Style values applied to every sensor unless the sensor overrides them:
/// the same style fields as `SensorConfig`, minus name/description/source/
/// action (defaults are style only).
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct SensorDefaults {
    pub text_color: Option<String>,
    pub bg_color: Option<String>,
    pub text_color_hover: Option<String>,
    pub bg_color_hover: Option<String>,
    pub border_color: Option<String>,
    pub border_color_hover: Option<String>,
    pub border_width: Option<i32>,
    pub border_width_hover: Option<i32>,
    pub border_radius: Option<i32>,
    pub border_type: Option<String>,
    pub padding: Option<i32>,
}

/// Style values applied to every section unless the section overrides them:
/// the style fields of `SectionConfig` (bg, text, border) — layout/weight are
/// structure and stay per-section only. Same group rule as `sensor_defaults`.
#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names)]
pub struct SectionDefaults {
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub border_color: Option<String>,
    pub border_width: Option<i32>,
    pub border_radius: Option<i32>,
    pub border_type: Option<String>,
    /// When true only the sides NOT flush with a screen edge get rounded
    /// borders (roadmap M3); absent = false.
    pub respect_screen_edges: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names, clippy::struct_field_names)]
pub struct SectionConfig {
    /// Empty/omitted name renders as an empty title (setup saves it as absent).
    #[serde(default)]
    pub name: String,
    pub layout: Option<String>,
    pub weight: Option<u32>,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    /// Section border colour; drawn when a positive width is set too.
    pub border_color: Option<String>,
    /// Border thickness; `-1` = disabled (same rule as the sensor fields).
    pub border_width: Option<i32>,
    pub border_radius: Option<i32>,
    /// Border drawing style: `full` (default), `corners`, `underline`,
    /// `upperline`, `left` or `right`.
    pub border_type: Option<String>,
    pub sensors: Vec<SensorConfig>,
}

/// A value-threshold colour rule: while the sensor's displayed value
/// parses to at least `min`, its text renders in `color`; the highest
/// matching rule wins. Rules below every reading and unparsable
/// colours/values are ignored.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct ColorRule {
    pub min: f64,
    pub color: String,
}

#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names, clippy::struct_field_names)]
pub struct SensorConfig {
    /// Empty/omitted name renders as an empty label (setup saves it as absent).
    #[serde(default)]
    pub name: String,
    pub description: Option<String>,
    /// Freedesktop icon name rendered before the sensor (roadmap N1);
    /// `None` = text only. The whole icon machinery stays dormant while
    /// no sensor of the config declares one (see `icons_theme`).
    #[serde(default)]
    pub icon: Option<String>,
    pub source: SensorSource,
    pub text_color: Option<String>,
    pub bg_color: Option<String>,
    pub text_color_hover: Option<String>,
    pub bg_color_hover: Option<String>,
    pub border_color: Option<String>,
    pub border_color_hover: Option<String>,
    pub border_width: Option<i32>,
    pub border_width_hover: Option<i32>,
    pub border_radius: Option<i32>,
    /// Border drawing style: `full` (default), `corners`, `underline`,
    /// `upperline`, `left` or `right`. `None` = `full`.
    pub border_type: Option<String>,
    pub padding: Option<i32>,
    /// Action executed when the sensor is clicked. `None` = no action.
    /// A `command:` prefix runs a shell command in the background; any other
    /// value is treated as an internal action (e.g. `close_panel`).
    #[serde(default)]
    pub action: Option<String>,
    /// Show the value's trend in the hover graph popup (roadmap2 #11).
    /// Only sources with a numeric reading can graph (`perc_*`, `used_*`,
    /// `temp_cpu`); on the others the toggle is inert.
    #[serde(default)]
    pub draw_graph: bool,
    /// Threshold colours for the displayed value (`color_rules`): the
    /// highest `min` the reading reaches wins; hover still overrides
    /// (`text_color_hover`). Absent = plain `text_color`.
    #[serde(default)]
    pub color_rules: Option<Vec<ColorRule>>,
}

/// Sensor sources. JSON `type` values follow the naming convention:
/// `perc_` percent, `used_` numeric consumption, `total_` total value,
/// `detail_` used/total ratio, `temp_` temperature; everything else is
/// plain lowercase (it has no numeric category). The `alias` on each
/// renamed variant keeps configs written with the old PascalCase names
/// loadable forever.
#[derive(Deserialize, serde::Serialize, Clone)]
#[serde(tag = "type")]
#[allow(clippy::upper_variable_names)]
pub enum SensorSource {
    #[serde(rename = "command", alias = "Command")]
    Command { command: String },
    #[serde(rename = "env_var", alias = "EnvVar")]
    EnvVar { env_var: String },
    #[serde(rename = "static_text", alias = "StaticText")]
    StaticText { static_text: String },
    #[serde(rename = "perc_cpu", alias = "ProcCpu")]
    PercCpu {},
    #[serde(rename = "perc_gpu", alias = "NvidiaGpu")]
    PercGpu {},
    #[serde(rename = "perc_mem")]
    PercMem {},
    #[serde(rename = "perc_swap")]
    PercSwap {},
    #[serde(rename = "perc_disk", alias = "ProcDisk")]
    PercDisk {},
    #[serde(rename = "perc_battery")]
    PercBattery {},
    #[serde(rename = "perc_backlight")]
    PercBacklight {},
    #[serde(rename = "perc_volume", alias = "Audio")]
    PercVolume { device: String },
    #[serde(rename = "used_mem")]
    UsedMem {},
    #[serde(rename = "used_swap")]
    UsedSwap {},
    #[serde(rename = "used_disk")]
    UsedDisk {},
    #[serde(rename = "used_battery")]
    UsedBattery {},
    #[serde(rename = "used_netdown")]
    UsedNetdown { interface: String },
    #[serde(rename = "used_netup")]
    UsedNetup { interface: String },
    #[serde(rename = "total_mem")]
    TotalMem {},
    #[serde(rename = "total_swap")]
    TotalSwap {},
    #[serde(rename = "total_disk")]
    TotalDisk {},
    #[serde(rename = "total_battery")]
    TotalBattery {},
    #[serde(rename = "detail_mem", alias = "ProcMem")]
    DetailMem {},
    #[serde(rename = "detail_swap")]
    DetailSwap {},
    #[serde(rename = "detail_disk")]
    DetailDisk {},
    #[serde(rename = "temp_cpu")]
    TempCpu {},
    #[serde(rename = "uptime", alias = "Uptime")]
    Uptime {},
    #[serde(rename = "net_state", alias = "Network")]
    NetState { interface: String },
    #[serde(rename = "clock", alias = "Clock")]
    Clock { format: Option<String> },
    /// Date/time on the bar (arbitrary `date` strftime, like `clock`)
    /// with the month calendar popup on hover.
    #[serde(rename = "calendar", alias = "Calendar")]
    Calendar { format: Option<String> },
    #[serde(rename = "window", alias = "Window")]
    Window {},
    #[serde(rename = "workspace", alias = "Workspace")]
    Workspace {},
    /// System tray (StatusNotifierItem): renders the tray icons; the
    /// rendering and the whole D-Bus side live in `tray.rs`.
    #[serde(rename = "tray")]
    Tray {},
    /// Desktop Notifications: shows the newest active notification; the
    /// D-Bus server itself lives in `notifications.rs` (enabled by the
    /// `panel.notifications` block).
    #[serde(rename = "notifications")]
    Notifications {},
}

// ── defaults ────────────────────────────────────────────────────────

pub fn default_config() -> Config {
    let section = SectionConfig {
        name: "System".to_string(),
        layout: Some("horizontal".to_string()),
        weight: None,
        bg_color: None,
        text_color: None,
        border_color: None,
        border_width: None,
        border_radius: None,
        border_type: None,
        sensors: vec![
            SensorConfig { name: "CPU".to_string(), description: None, icon: None, source: SensorSource::PercCpu {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, color_rules: None },
            SensorConfig { name: "GPU".to_string(), description: None, icon: None, source: SensorSource::PercGpu {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, color_rules: None },
            SensorConfig { name: "MEM".to_string(), description: None, icon: None, source: SensorSource::DetailMem {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, color_rules: None },
            SensorConfig { name: "DISK".to_string(), description: None, icon: None, source: SensorSource::PercDisk {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, color_rules: None },
            SensorConfig { name: "UPTIME".to_string(), description: None, icon: None, source: SensorSource::Uptime {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, color_rules: None },
        ],
    };

    Config {
        panel: PanelConfig {
            value_font: None,
            font: Some("Noto Sans".to_string()),
            font_size: Some(14),
            bold: None,
            icon_theme: None,
            autostart: None,
            autostart_disabled: None,
            app_theme: None,
            wallpaper: None,
            section_layout: Some("horizontal".to_string()),
            default_section_weight: Some(1),
            default_bg_color: None,
            default_text_color: None,
            anchor: Some("top".to_string()),
            align: Some("center".to_string()),
            thickness: None,
            sensor_defaults: None,
            section_defaults: None,
            notifications: None,
            sections: vec![section],
        },
    }
}

// ── loading ─────────────────────────────────────────────────────────

fn default_config_path() -> PathBuf {
    let default_path = if let Some(config_home) = dirs::config_dir() {
        config_home.join("piuma").join("piuma.json")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".config").join("piuma").join("piuma.json")
    } else {
        PathBuf::from("piuma.json")
    };
    default_path
}

/// Load configuration from an explicit path or the default location.
///
/// Returns the config together with the path it was actually loaded from,
/// so the caller can watch that file for hot reload.
pub fn load_config(path: Option<String>) -> (Config, PathBuf) {
    if let Some(path) = path {
        let p = PathBuf::from(path);
        if p.exists() {
            match std::fs::read_to_string(&p) {
                // The serde reason (e.g. "missing field `static_text`") is
                // part of the message: a broken config must be fixable, not
                // mysterious (the hot reload prints it the same way).
                Ok(contents) => match serde_json::from_str::<Config>(&contents) {
                    Ok(config) => return (config, p),
                    Err(e) => eprintln!("[piuma] Failed to parse config at {}: {}", p.display(), e),
                },
                Err(e) => eprintln!("[piuma] Cannot read config at {}: {}", p.display(), e),
            }
        } else {
            eprintln!("[piuma] Config file not found: {}", p.display());
        }
    }

    // Try default location
    let default_path = default_config_path();
    if default_path.exists() {
        match std::fs::read_to_string(&default_path) {
            Ok(contents) => match serde_json::from_str::<Config>(&contents) {
                Ok(config) => return (config, default_path),
                Err(e) => eprintln!("[piuma] Failed to parse config at {}: {}", default_path.display(), e),
            },
            Err(e) => eprintln!("[piuma] Cannot read config at {}: {}", default_path.display(), e),
        }
    }

    // Generate the default config file only when none exists: an existing
    // file that fails to parse must stay untouched (the user can fix it),
    // otherwise a broken read would silently wipe the configuration.
    if !default_path.exists() {
        let config_dir = default_path.parent();
        if let Some(dir) = config_dir {
            // Every failure below is reported: discarding it would leave the
            // user without a config file and without a clue why.
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("[piuma] Cannot create config dir {}: {}", dir.display(), e);
            }
            match serde_json::to_string_pretty(&default_config()) {
                Ok(default_cfg) => {
                    if let Err(e) = std::fs::write(&default_path, default_cfg) {
                        eprintln!("[piuma] Cannot write default config {}: {}", default_path.display(), e);
                    }
                }
                Err(e) => eprintln!("[piuma] Cannot serialize default config: {}", e),
            }
        }
    } else {
        eprintln!("[piuma] Keeping the existing config untouched; running with defaults until it parses");
    }

    (default_config(), default_path)
}

/// Write the config back to `path` atomically (temp file + rename): the
/// bar's autostart pruning rewrites the file it loaded whenever the
/// disabled list lost stale names, and a running piuma must never read
/// a half-written config. Serialization is piuma's own schema, exactly
/// what [`load_config`] parses.
pub fn save_config(path: &std::path::Path, config: &Config) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `tray` sensor must be part of a config piuma accepts whole: this
    /// parse is exactly what piuma-setup's pre-save validation runs.
    #[test]
    fn tray_sensor_parses_and_roundtrips() {
        let json = r#"{"panel":{"sections":[{"name":"sys","sensors":[{"name":"Tray","source":{"type":"tray"}}]}]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("a tray sensor must be valid");
        assert!(matches!(
            cfg.panel.sections[0].sensors[0].source,
            SensorSource::Tray {}
        ));
        // The type name must come back unchanged (setup writes it verbatim).
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""type":"tray""#), "got: {out}");
    }

    /// Roadmap2 #1: `panel.wallpaper` parses whole, partial and absent —
    /// exactly the validation piuma-setup runs before saving, and the
    /// per-field defaults every older config relies on.
    #[test]
    fn wallpaper_keys_parse_and_roundtrip() {
        let json = r#"{"panel":{"wallpaper":{"enabled":true,"path":"/tmp/a.png"},"sections":[]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("wallpaper must be valid");
        let wp = cfg.panel.wallpaper.as_ref().expect("block present");
        assert!(wp.enabled);
        assert_eq!(wp.path, "/tmp/a.png");
        // A partial block falls back per field…
        let cfg: Config =
            serde_json::from_str(r#"{"panel":{"wallpaper":{},"sections":[]}}"#).expect("partial ok");
        let wp = cfg.panel.wallpaper.as_ref().expect("block present");
        assert!(!wp.enabled);
        assert!(wp.path.is_empty());
        // …and an absent block stays absent (old configs: disabled).
        let cfg: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("absent ok");
        assert!(cfg.panel.wallpaper.is_none());
        // Roundtrip through piuma's own serializer (setup writes the same
        // shape by hand).
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        let back: Config = serde_json::from_str(&out).expect("reparse must work");
        assert!(back.panel.wallpaper.is_none());
    }

    /// Roadmap N1: `panel.icon_theme` and the per-sensor `icon` parse and
    /// roundtrip — exactly the validation piuma-setup runs before saving.
    #[test]
    fn icon_keys_parse_and_roundtrip() {
        let json = r#"{"panel":{"icon_theme":"Papirus","sections":[{"name":"sys","sensors":[{"name":"CPU","icon":"cpu","source":{"type":"perc_cpu"}}]}]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("icon keys must parse");
        assert_eq!(cfg.panel.icon_theme.as_deref(), Some("Papirus"));
        assert_eq!(cfg.panel.sections[0].sensors[0].icon.as_deref(), Some("cpu"));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""icon":"cpu""#), "got: {out}");
        assert!(out.contains(r#""icon_theme":"Papirus""#), "got: {out}");
        // Absent keys deserialize as `None` (setup never writes them).
        let bare: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("bare config parses");
        assert!(bare.panel.icon_theme.is_none());
        assert!(bare.panel.sections.is_empty());
    }

    /// The `calendar` sensor's source parses with and without a format
    /// and roundtrips the type name — exactly the validation
    /// piuma-setup runs before saving.
    #[test]
    fn calendar_sensor_parses_and_roundtrips() {
        let json = r#"{"panel":{"sections":[{"name":"sys","sensors":[
            {"name":"Ora","source":{"type":"calendar","format":"%a %d %b %Y, %H:%M:%S"}},
            {"name":"Ora base","source":{"type":"calendar"}}
        ]}]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("a calendar sensor must be valid");
        let SensorSource::Calendar { format } = &cfg.panel.sections[0].sensors[0].source else {
            panic!("first sensor must be a calendar source");
        };
        assert_eq!(format.as_deref(), Some("%a %d %b %Y, %H:%M:%S"));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""type":"calendar""#), "got: {out}");
        assert!(
            matches!(
                cfg.panel.sections[0].sensors[1].source,
                SensorSource::Calendar { format: None }
            ),
            "the bare form keeps format absent"
        );
    }

    /// Roadmap N2: `panel.autostart` parses and roundtrips; absent stays
    /// `None` — the bar never autostarts anything by default.
    #[test]
    fn autostart_key_parse_and_roundtrip() {
        let json = r#"{"panel":{"autostart":true,"sections":[]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("autostart must parse");
        assert_eq!(cfg.panel.autostart, Some(true));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""autostart":true"#), "got: {out}");
        let bare: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("bare config parses");
        assert!(bare.panel.autostart.is_none());
    }

    /// `panel.autostart_disabled` parses, keeps its order and roundtrips
    /// through [`save_config`]: the bar's prune rewrites the file it
    /// loaded, so a load→save cycle must come back intact.
    #[test]
    fn autostart_disabled_roundtrips_through_save() {
        let json = r#"{"panel":{"autostart":true,"autostart_disabled":["b.desktop","a.desktop"],"sections":[]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("selection must parse");
        assert_eq!(
            cfg.panel.autostart_disabled,
            Some(vec!["b.desktop".to_string(), "a.desktop".to_string()])
        );
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(
            out.contains(r#""autostart_disabled":["b.desktop","a.desktop"]"#),
            "got: {out}"
        );

        let dir = std::env::temp_dir().join(format!("piuma-cfg-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("piuma.json");
        save_config(&path, &cfg).expect("save must succeed");
        let back = std::fs::read_to_string(&path).expect("saved file");
        let round: Config = serde_json::from_str(&back).expect("saved config must parse");
        assert_eq!(round.panel.autostart_disabled, cfg.panel.autostart_disabled);

        let bare: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("bare config parses");
        assert!(bare.panel.autostart_disabled.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Roadmap N3: `panel.app_theme` parses and roundtrips; absent stays
    /// `None` — the GTK theme bridge stays off by default.
    #[test]
    fn app_theme_key_parse_and_roundtrip() {
        let json = r#"{"panel":{"app_theme":true,"sections":[]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("app_theme must parse");
        assert_eq!(cfg.panel.app_theme, Some(true));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""app_theme":true"#), "got: {out}");
        let bare: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("bare config parses");
        assert!(bare.panel.app_theme.is_none());
    }

    /// `panel.notifications`: presence gates the D-Bus server, missing
    /// keys fall back to the runtime defaults, and the sensor type must
    /// parse — this is exactly piuma-setup's pre-save validation. The
    /// popup style keys parse too (`-1` = "use the popup default").
    #[test]
    fn notifications_block_and_sensor_parse() {
        let cfg: Config = serde_json::from_str(
            r##"{"panel":{"notifications":{"default_timeout_ms":4000,"history_size":7,"replace_existing":false,"font_size":16,"text_color":"#ffe066","bg_color":"#101010","border_color":"#ff5555","border_width":2,"border_radius":16,"border_type":"corners","padding":8},"sections":[]}}"##,
        )
        .expect("block must parse");
        let n = cfg.panel.notifications.as_ref().expect("present");
        assert_eq!(
            (n.default_timeout_ms, n.history_size, n.replace_existing),
            (Some(4000), Some(7), Some(false))
        );
        assert_eq!(
            (
                n.font_size,
                n.text_color.as_deref(),
                n.bg_color.as_deref(),
                n.border_color.as_deref(),
                n.border_width,
                n.border_radius,
                n.border_type.as_deref(),
                n.padding
            ),
            (
                Some(16),
                Some("#ffe066"),
                Some("#101010"),
                Some("#ff5555"),
                Some(2),
                Some(16),
                Some("corners"),
                Some(8)
            )
        );
        // Empty block: present (server on) with absent keys → defaults.
        let cfg2: Config =
            serde_json::from_str(r#"{"panel":{"notifications":{},"sections":[]}}"#).unwrap();
        let n2 = cfg2.panel.notifications.as_ref().expect("presence");
        assert_eq!(
            (n2.default_timeout_ms, n2.history_size, n2.replace_existing),
            (None, None, None)
        );
        assert!(
            n2.font_size.is_none() && n2.text_color.is_none() && n2.border_width.is_none(),
            "absent style keys keep the popup defaults"
        );
        // Absent: no server.
        let cfg3: Config = serde_json::from_str(r#"{"panel":{"sections":[]}}"#).unwrap();
        assert!(cfg3.panel.notifications.is_none());
        // The sensor type parses and the block survives serialization.
        let cfg4: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"name":"N","source":{"type":"notifications"}}]}]}}"#,
        )
        .expect("sensor must parse");
        assert!(matches!(
            cfg4.panel.sections[0].sensors[0].source,
            SensorSource::Notifications {}
        ));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""notifications""#), "got: {out}");
    }
}

