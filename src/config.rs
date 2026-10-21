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
/// A malformed value is an error naming the format — it must never parse
/// into a *wrong* colour (a non-hex digit used to fall back to 0, which
/// turned `#zz0000` into silent black).
pub fn parse_color(value: &str) -> Result<Color, String> {
    let value = value.trim();
    // Hex format: #RGB, #RRGGBB or #RRGGBBAA
    if value.starts_with('#') {
        let hex = &value[1..];
        // Slices below are byte based: reject non-ASCII instead of panicking
        // on a multi-byte boundary (same guard as piuma-setup's hex_color).
        if !hex.is_ascii() {
            return Err(format!("{value:?}: non-ASCII colour"));
        }
        // Every digit must be hex: a wrong one is an error, never a zero.
        let nibble = |i: usize| -> Result<u8, String> {
            u8::from_str_radix(&hex[i..i + 1], 16)
                .map_err(|_| format!("{value:?}: expected hex digits after #"))
        };
        // `byte(i)` reads the pair of digits at byte offset `i` (two
        // nibbles: high digit × 16 + low digit).
        let byte = |i: usize| -> Result<u8, String> { Ok(nibble(2 * i)? * 16 + nibble(2 * i + 1)?) };
        let (r, g, b, a) = match hex.len() {
            // Shorthand "#rgb": each digit is duplicated, alpha opaque.
            3 => (nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17, 255),
            6 => (byte(0)?, byte(1)?, byte(2)?, 255),
            8 => (byte(0)?, byte(1)?, byte(2)?, byte(3)?),
            _ => return Err(format!("{value:?}: expected #rgb, #rrggbb or #rrggbbaa")),
        };
        Ok(Color::from_rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ))
    } else {
        // Float string: "r g b a" — every component must be a number.
        let parts: Vec<&str> = value.split_whitespace().collect();
        let component = |i: usize| -> Result<f32, String> {
            parts[i]
                .parse::<f32>()
                .map_err(|_| format!("{value:?}: expected \"r g b a\" float components"))
        };
        if parts.len() >= 4 {
            Ok(Color::from_rgba(component(0)?, component(1)?, component(2)?, component(3)?))
        } else {
            Err(format!("{value:?}: expected \"r g b a\" float components"))
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
    /// Derive the bar's base colours (panel bg, sensor default card and
    /// text) from the wallpaper at runtime (visual option); absent =
    /// false. The extracted tones are NEVER written back to this file:
    /// they live in the bar's memory only, so turning the option off
    /// restores the user's own colours on the next rebuild.
    pub wallpaper_palette: Option<bool>,
    /// Rounded screen corners (roadmap angoli-schermo); absent = off
    /// (no corner mask surface is opened).
    pub screen_corners: Option<ScreenCornersConfig>,
    pub section_layout: Option<String>,
    /// Draw each section's title above its stacked sensors (the title
    /// exists only where sensors stack); absent = true.
    pub section_title: Option<bool>,
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

/// `panel.screen_corners`: a square mask per screen corner making the
/// display read as rounded — the surface paints the square in `color`
/// except the inner quarter disk, which stays transparent (roadmap
/// angoli-schermo). Every field defaults so a partial block (or none at
/// all) keeps old configs working: off, black, 32 px.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct ScreenCornersConfig {
    /// Whether the corner masks are drawn at all.
    #[serde(default)]
    pub enable: bool,
    /// Mask colour (`#rgb` / `#rrggbb` / `#rrggbbaa`).
    #[serde(default = "default_corner_color")]
    pub color: String,
    /// Corner side in px, also the rounding radius (1..=256).
    #[serde(default = "default_corner_radius")]
    pub radius: u32,
}

fn default_corner_color() -> String {
    "#000000".to_string()
}

fn default_corner_radius() -> u32 {
    32
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
#[derive(Debug, Deserialize, serde::Serialize, Clone, PartialEq)]
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
    /// Taskbar: one icon per open window — left focuses it, middle
    /// requests its close, right opens the move-to-workspace menu
    /// (windows.rs reads `hyprctl clients -j` per sensor tick).
    #[serde(rename = "taskbar")]
    Taskbar {},
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
            wallpaper_palette: None,
            screen_corners: None,
            section_layout: Some("horizontal".to_string()),
            section_title: None,
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

// ── value validation ────────────────────────────────────────────────

/// One invalid value inside a parsed config: the field's JSON path (as
/// `panel.sections[0].sensors[1].border_color`) and why it is rejected.
/// `validate_config` collects them, so the bar (logged at load) and
/// piuma-setup (blocks the save and highlights the field) name exactly
/// the same problems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: String,
    pub message: String,
}

/// The `border_type` values the renderer understands (`styles.rs`).
pub const BORDER_TYPES: [&str; 6] = ["full", "corners", "underline", "upperline", "left", "right"];

/// The `err_*` helpers below each append one `ConfigError` (path +
/// reason) when a present value does not parse or is out of range;
/// absent fields (`None`) are always fine.

fn err_color(errors: &mut Vec<ConfigError>, path: &str, value: Option<&str>) {
    if let Some(v) = value {
        if let Err(message) = parse_color(v) {
            errors.push(ConfigError { path: path.to_string(), message });
        }
    }
}

fn err_one_of(errors: &mut Vec<ConfigError>, path: &str, value: Option<&str>, allowed: &[&str]) {
    if let Some(v) = value {
        if !allowed.contains(&v) {
            errors.push(ConfigError {
                path: path.to_string(),
                message: format!("{v:?}: expected one of {}", allowed.join(", ")),
            });
        }
    }
}

/// Signed style numbers keep the `-1 = disabled` sentinel valid (see
/// `style_num`), so the ranges start at -1 where a sentinel exists.
fn err_range(errors: &mut Vec<ConfigError>, path: &str, value: Option<i32>, min: i32, max: i32) {
    if let Some(v) = value {
        if !(min..=max).contains(&v) {
            errors.push(ConfigError {
                path: path.to_string(),
                message: format!("{v}: expected {min}..={max}"),
            });
        }
    }
}

fn err_range_u32(errors: &mut Vec<ConfigError>, path: &str, value: Option<u32>, min: u32, max: u32) {
    if let Some(v) = value {
        if !(min..=max).contains(&v) {
            errors.push(ConfigError {
                path: path.to_string(),
                message: format!("{v}: expected {min}..={max}"),
            });
        }
    }
}

/// The six-colour + border-type + style-numbers block, shared by
/// `panel.sensor_defaults` and every sensor's style overrides.
#[allow(clippy::too_many_arguments)]
fn err_sensor_style(
    errors: &mut Vec<ConfigError>,
    path: &str,
    text_color: Option<&str>,
    text_color_hover: Option<&str>,
    bg_color: Option<&str>,
    bg_color_hover: Option<&str>,
    border_color: Option<&str>,
    border_color_hover: Option<&str>,
    border_type: Option<&str>,
    border_width: Option<i32>,
    border_width_hover: Option<i32>,
    border_radius: Option<i32>,
    padding: Option<i32>,
) {
    for (name, value) in [
        ("text_color", text_color),
        ("text_color_hover", text_color_hover),
        ("bg_color", bg_color),
        ("bg_color_hover", bg_color_hover),
        ("border_color", border_color),
        ("border_color_hover", border_color_hover),
    ] {
        err_color(errors, &format!("{path}.{name}"), value);
    }
    err_one_of(errors, &format!("{path}.border_type"), border_type, &BORDER_TYPES);
    err_range(errors, &format!("{path}.border_width"), border_width, -1, 64);
    err_range(errors, &format!("{path}.border_width_hover"), border_width_hover, -1, 64);
    err_range(errors, &format!("{path}.border_radius"), border_radius, -1, 64);
    err_range(errors, &format!("{path}.padding"), padding, -1, 64);
}

/// Walks every typed field of the config — colours, the `border_type`
/// enums, the numeric ranges — and collects each value piuma would
/// silently ignore or clamp at render time. This is what turns a
/// silently-dropped edit into a named, fixable error.
pub fn validate_config(cfg: &Config) -> Vec<ConfigError> {
    let mut errors: Vec<ConfigError> = Vec::new();
    let p = &cfg.panel;

    // Panel-level typed values.
    err_color(&mut errors, "panel.default_bg_color", p.default_bg_color.as_deref());
    err_color(&mut errors, "panel.default_text_color", p.default_text_color.as_deref());
    err_one_of(
        &mut errors,
        "panel.anchor",
        p.anchor.as_deref(),
        // The setup picklist offers the four edges and four corners.
        &["top", "bottom", "left", "right", "top-left", "top-right", "bottom-left", "bottom-right"],
    );
    err_one_of(
        &mut errors,
        "panel.align",
        p.align.as_deref(),
        // "top"/"bottom" position inside a vertical panel, "fill" stretches
        // (geometry.rs reads all of them).
        &["left", "center", "right", "fill", "top", "bottom"],
    );
    err_one_of(
        &mut errors,
        "panel.section_layout",
        p.section_layout.as_deref(),
        &["horizontal", "vertical"],
    );
    err_range_u32(&mut errors, "panel.thickness", p.thickness, 4, 256);
    err_range_u32(&mut errors, "panel.font_size", p.font_size, 1, 256);
    err_range_u32(&mut errors, "panel.default_section_weight", p.default_section_weight, 1, 1000);

    if let Some(c) = &p.screen_corners {
        err_color(&mut errors, "panel.screen_corners.color", Some(&c.color));
        err_range_u32(&mut errors, "panel.screen_corners.radius", Some(c.radius), 1, 256);
    }

    if let Some(d) = &p.sensor_defaults {
        err_sensor_style(
            &mut errors,
            "panel.sensor_defaults",
            d.text_color.as_deref(),
            d.text_color_hover.as_deref(),
            d.bg_color.as_deref(),
            d.bg_color_hover.as_deref(),
            d.border_color.as_deref(),
            d.border_color_hover.as_deref(),
            d.border_type.as_deref(),
            d.border_width,
            d.border_width_hover,
            d.border_radius,
            d.padding,
        );
    }
    if let Some(sd) = &p.section_defaults {
        let path = "panel.section_defaults";
        for (name, value) in [
            ("bg_color", sd.bg_color.as_deref()),
            ("text_color", sd.text_color.as_deref()),
            ("border_color", sd.border_color.as_deref()),
        ] {
            err_color(&mut errors, &format!("{path}.{name}"), value);
        }
        err_one_of(&mut errors, &format!("{path}.border_type"), sd.border_type.as_deref(), &BORDER_TYPES);
        err_range(&mut errors, &format!("{path}.border_width"), sd.border_width, -1, 64);
        err_range(&mut errors, &format!("{path}.border_radius"), sd.border_radius, -1, 64);
    }
    if let Some(n) = &p.notifications {
        let path = "panel.notifications";
        for (name, value) in [
            ("text_color", n.text_color.as_deref()),
            ("text_color_hover", n.text_color_hover.as_deref()),
            ("bg_color", n.bg_color.as_deref()),
            ("bg_color_hover", n.bg_color_hover.as_deref()),
            ("border_color", n.border_color.as_deref()),
            ("border_color_hover", n.border_color_hover.as_deref()),
        ] {
            err_color(&mut errors, &format!("{path}.{name}"), value);
        }
        err_one_of(&mut errors, &format!("{path}.border_type"), n.border_type.as_deref(), &BORDER_TYPES);
        err_range(&mut errors, &format!("{path}.border_width"), n.border_width, -1, 64);
        err_range(&mut errors, &format!("{path}.border_radius"), n.border_radius, -1, 64);
        err_range(&mut errors, &format!("{path}.padding"), n.padding, -1, 64);
        err_range_u32(&mut errors, &format!("{path}.font_size"), n.font_size, 1, 256);
        err_range_u32(&mut errors, &format!("{path}.history_size"), n.history_size, 1, 1000);
        // A day of timeout at most; `popup_position` needs no check: any
        // unknown value keeps the historic layout by design.
        if let Some(t) = n.default_timeout_ms {
            if t > 86_400_000 {
                errors.push(ConfigError {
                    path: format!("{path}.default_timeout_ms"),
                    message: format!("{t}: expected 0..=86400000"),
                });
            }
        }
        err_range(&mut errors, &format!("{path}.popup_margin_h"), n.popup_margin_h, -1000, 1000);
        err_range(&mut errors, &format!("{path}.popup_margin_v"), n.popup_margin_v, -1000, 1000);
    }

    for (i, s) in p.sections.iter().enumerate() {
        let path = format!("panel.sections[{i}]");
        err_one_of(&mut errors, &format!("{path}.layout"), s.layout.as_deref(), &["horizontal", "vertical"]);
        err_range_u32(&mut errors, &format!("{path}.weight"), s.weight, 1, 1000);
        for (name, value) in [
            ("bg_color", s.bg_color.as_deref()),
            ("text_color", s.text_color.as_deref()),
            ("border_color", s.border_color.as_deref()),
        ] {
            err_color(&mut errors, &format!("{path}.{name}"), value);
        }
        err_one_of(&mut errors, &format!("{path}.border_type"), s.border_type.as_deref(), &BORDER_TYPES);
        err_range(&mut errors, &format!("{path}.border_width"), s.border_width, -1, 64);
        err_range(&mut errors, &format!("{path}.border_radius"), s.border_radius, -1, 64);
        for (j, sensor) in s.sensors.iter().enumerate() {
            let spath = format!("{path}.sensors[{j}]");
            err_sensor_style(
                &mut errors,
                &spath,
                sensor.text_color.as_deref(),
                sensor.text_color_hover.as_deref(),
                sensor.bg_color.as_deref(),
                sensor.bg_color_hover.as_deref(),
                sensor.border_color.as_deref(),
                sensor.border_color_hover.as_deref(),
                sensor.border_type.as_deref(),
                sensor.border_width,
                sensor.border_width_hover,
                sensor.border_radius,
                sensor.padding,
            );
            if let Some(rules) = &sensor.color_rules {
                for (k, rule) in rules.iter().enumerate() {
                    err_color(&mut errors, &format!("{spath}.color_rules[{k}].color"), Some(&rule.color));
                }
            }
        }
    }
    errors
}

/// Log every invalid value of a freshly loaded config: render time falls
/// back to the defaults silently, so the log — together with
/// piuma-setup's highlighted fields — is where a hand-edited value
/// gets named instead of vanishing.
pub fn report_invalid(cfg: &Config) {
    for error in validate_config(cfg) {
        eprintln!("[piuma] config {}: {} — using the default", error.path, error.message);
    }
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
                    Ok(config) => {
                        // Named once here: render time only falls back.
                        report_invalid(&config);
                        return (config, p);
                    }
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
                Ok(config) => {
                    report_invalid(&config);
                    return (config, default_path);
                }
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

    /// Roadmap angoli-schermo: `panel.screen_corners` parses whole,
    /// partial (per-field defaults) and absent, roundtrips — and the
    /// validator names a bad colour or radius by path, accepts the
    /// default one.
    #[test]
    fn screen_corners_parse_validate_and_roundtrip() {
        let json = r##"{"panel":{"screen_corners":{"enable":true,"color":"#101010","radius":48},"sections":[]}}"##;
        let cfg: Config = serde_json::from_str(json).expect("corners must be valid");
        let sc = cfg.panel.screen_corners.as_ref().expect("block present");
        assert!(sc.enable);
        assert_eq!(sc.color, "#101010");
        assert_eq!(sc.radius, 48);
        // A partial block falls back per field (off, black, 32 px)…
        let cfg: Config =
            serde_json::from_str(r#"{"panel":{"screen_corners":{},"sections":[]}}"#).expect("partial ok");
        let sc = cfg.panel.screen_corners.as_ref().expect("block present");
        assert!(!sc.enable);
        assert_eq!(sc.color, "#000000");
        assert_eq!(sc.radius, 32);
        // …and validates clean (the defaults are in range)…
        assert_eq!(validate_config(&cfg), Vec::new());
        // …while an absent block stays absent (old configs: off).
        let cfg: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("absent ok");
        assert!(cfg.panel.screen_corners.is_none());
        assert_eq!(validate_config(&cfg), Vec::new());
        // Roundtrip through piuma's own serializer.
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        let back: Config = serde_json::from_str(&out).expect("reparse must work");
        assert!(back.panel.screen_corners.is_none());
        // An unparsable colour and an out-of-range radius are each
        // reported at their JSON path.
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"screen_corners":{"enable":true,"color":"nope","radius":0},"sections":[]}}"#,
        )
        .expect("config must parse");
        let errors = validate_config(&cfg);
        let paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["panel.screen_corners.color", "panel.screen_corners.radius"],
        );
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

    /// `panel.section_title` parses and roundtrips; absent stays `None`
    /// — the bar draws the titles by default.
    #[test]
    fn section_title_key_parse_and_roundtrip() {
        let json = r#"{"panel":{"section_title":false,"sections":[]}}"#;
        let cfg: Config = serde_json::from_str(json).expect("section_title must parse");
        assert_eq!(cfg.panel.section_title, Some(false));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""section_title":false"#), "got: {out}");
        let bare: Config =
            serde_json::from_str(r#"{"panel":{"sections":[]}}"#).expect("bare config parses");
        assert!(bare.panel.section_title.is_none());
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

    /// The taskbar sensor type parses (parameterless, like `window`) and
    /// survives a save roundtrip.
    #[test]
    fn taskbar_sensor_parses_and_roundtrips() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"name":"T","source":{"type":"taskbar"}}]}]}}"#,
        )
        .expect("sensor must parse");
        assert!(matches!(
            cfg.panel.sections[0].sensors[0].source,
            SensorSource::Taskbar {}
        ));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""taskbar""#), "got: {out}");
    }

    // ── parse_color strictness ─────────────────────────────────────

    /// Every accepted format still parses to the same colour.
    #[test]
    fn parse_color_keeps_the_valid_formats() {
        let c = parse_color("#ff0000").expect("hex6");
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 1.0));
        let c = parse_color("#f00").expect("hex3");
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 1.0));
        let c = parse_color("#10203080").expect("hex8");
        assert!((c.a - 128.0 / 255.0).abs() < 0.001);
        let c = parse_color("0.25 0.5 0.75 1.0").expect("floats");
        assert_eq!((c.r, c.g, c.b, c.a), (0.25, 0.5, 0.75, 1.0));
    }

    /// A malformed value is an error naming the format — it must never
    /// become a *wrong* colour (a bad hex digit used to fall back to 0,
    /// turning `#zz0000` into silent black).
    #[test]
    fn parse_color_rejects_malformed_values() {
        let err = parse_color("#zz0000").expect_err("bad digit");
        assert!(err.contains("hex"), "message: {err}");
        assert!(parse_color("#ff00").is_err(), "bad length");
        assert!(parse_color("").is_err(), "empty");
        assert!(parse_color("0.5 x 0.1 1").is_err(), "bad float component");
        assert!(parse_color("0.5 0.1").is_err(), "too few floats");
    }

    // ── validate_config ────────────────────────────────────────────

    /// The built-in default config carries no invalid value.
    #[test]
    fn default_config_validates_clean() {
        assert_eq!(validate_config(&default_config()), Vec::new());
    }

    /// An unparsable colour, an unknown `border_type` and an out-of-range
    /// number are each reported at their JSON path.
    #[test]
    fn validate_config_names_bad_values_by_path() {
        let cfg: Config = serde_json::from_str(
            r##"{"panel":{
              "anchor":"middle",
              "sensor_defaults":{"border_color":"#zz0000","padding":-2},
              "sections":[{"sensors":[{"name":"T","source":{"type":"taskbar"},
                "border_type":"diagonal","color_rules":[{"min":50,"color":"nope"}]}]}]
            }}"##,
        )
        .expect("config must parse");
        let errors = validate_config(&cfg);
        let paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "panel.anchor",
                "panel.sensor_defaults.border_color",
                "panel.sensor_defaults.padding",
                "panel.sections[0].sensors[0].border_type",
                "panel.sections[0].sensors[0].color_rules[0].color",
            ],
            "every error at its path: {errors:?}"
        );
    }

    /// The -1 = disabled sentinel and the setup picklist's own values
    /// (corner anchors, "fill" align) must validate: the editor offers them.
    #[test]
    fn validate_config_accepts_sentinels_and_ui_values() {
        let cfg: Config = serde_json::from_str(
            r##"{"panel":{"anchor":"top-left","align":"fill",
              "sections":[{"border_width":-1,"padding":-1,
                "sensors":[{"name":"T","source":{"type":"taskbar"},
                  "border_width":-1,"padding":-1}]}]}}"##,
        )
        .expect("config must parse");
        assert_eq!(validate_config(&cfg), Vec::new());
    }
}


