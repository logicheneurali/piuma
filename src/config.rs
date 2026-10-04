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
    pub sections: Vec<SectionConfig>,
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

#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names, clippy::struct_field_names)]
pub struct SensorConfig {
    /// Empty/omitted name renders as an empty label (setup saves it as absent).
    #[serde(default)]
    pub name: String,
    pub description: Option<String>,
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
}

#[derive(Deserialize, serde::Serialize, Clone)]
#[serde(tag = "type")]
#[allow(clippy::upper_variable_names)]
pub enum SensorSource {
    Command { command: String },
    EnvVar { env_var: String },
    StaticText { static_text: String },
    NvidiaGpu {},
    ProcCpu {},
    ProcMem {},
    ProcDisk {},
    Uptime {},
    Network { interface: String },
    Audio { device: String },
    Clock { format: Option<String> },
    Window {},
    Workspace {},
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
            SensorConfig { name: "CPU".to_string(), description: None, source: SensorSource::ProcCpu {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None },
            SensorConfig { name: "GPU".to_string(), description: None, source: SensorSource::NvidiaGpu {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None },
            SensorConfig { name: "MEM".to_string(), description: None, source: SensorSource::ProcMem {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None },
            SensorConfig { name: "DISK".to_string(), description: None, source: SensorSource::ProcDisk {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None },
            SensorConfig { name: "UPTIME".to_string(), description: None, source: SensorSource::Uptime {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None },
        ],
    };

    Config {
        panel: PanelConfig {
            font: Some("Noto Sans".to_string()),
            font_size: Some(14),
            section_layout: Some("horizontal".to_string()),
            default_section_weight: Some(1),
            default_bg_color: None,
            default_text_color: None,
            anchor: Some("top".to_string()),
            align: Some("center".to_string()),
            thickness: None,
            sensor_defaults: None,
            section_defaults: None,
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
            if let Ok(contents) = std::fs::read_to_string(&p) {
                if let Ok(config) = serde_json::from_str::<Config>(&contents) {
                    return (config, p);
                } else {
                    eprintln!("[piuma] Failed to parse config at {}", p.display());
                }
            } else {
                eprintln!("[piuma] Cannot read config at {}", p.display());
            }
        } else {
            eprintln!("[piuma] Config file not found: {}", p.display());
        }
    }

    // Try default location
    let default_path = default_config_path();
    if default_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&default_path) {
            if let Ok(config) = serde_json::from_str::<Config>(&contents) {
                return (config, default_path);
            } else {
                eprintln!("[piuma] Failed to parse config at {}", default_path.display());
            }
        }
    }

    // Generate the default config file only when none exists: an existing
    // file that fails to parse must stay untouched (the user can fix it),
    // otherwise a broken read would silently wipe the configuration.
    if !default_path.exists() {
        let config_dir = default_path.parent();
        if let Some(dir) = config_dir {
            let _ = std::fs::create_dir_all(dir);
            if let Ok(default_cfg) = serde_json::to_string_pretty(&default_config()) {
                let _ = std::fs::write(&default_path, default_cfg);
            }
        }
    } else {
        eprintln!("[piuma] Keeping the existing config untouched; running with defaults until it parses");
    }

    (default_config(), default_path)
}
