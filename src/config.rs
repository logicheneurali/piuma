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

/// Whether the panel asks for the light variant (absent = dark).
pub fn light_mode(panel: &PanelConfig) -> bool {
    panel.theme_mode.as_deref() == Some("light")
}

/// Default panel thickness on the cross axis, in pixels: what
/// `panel.thickness` falls back to when the config does not set it
/// (shared by the bar's surface size and the setup's icon preview, so
/// both follow one number).
pub const DEFAULT_HEIGHT: u32 = 32;

// ── config structs ──────────────────────────────────────────────────

use serde::Deserialize;

/// One extra bar: where it sits and which named sections it shows.
/// Section definitions/styles stay global (in `panel.sections`), so the
/// same section can appear on several monitors without duplication.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct PanelLayout {
    /// Monitor name, `"auto"` or `"all"`; absent = `"auto"`.
    pub output: Option<String>,
    pub anchor: Option<String>,
    pub align: Option<String>,
    /// Section names to show; absent/empty = every section.
    pub sections: Option<Vec<String>>,
}

#[derive(Deserialize, serde::Serialize, Clone)]
#[allow(clippy::upper_variable_names)]
pub struct Config {
    pub panel: PanelConfig,
    /// Extra panels; the legacy `panel` is always the first one. Absent =
    /// one panel showing every section.
    #[serde(default)]
    pub panels: Option<Vec<PanelLayout>>,
}

/// The layouts to render: the first from the legacy `panel`, the rest from
/// `panels`; absent `panels` means a single full panel.
pub fn panel_layouts(cfg: &Config) -> Vec<PanelLayout> {
    // A file written by the multi-panel setup carries every layout,
    // including the first one; the legacy `panel` block still supplies
    // the global fields and the first-panel defaults.
    if let Some(panels) = cfg.panels.as_ref().filter(|panels| !panels.is_empty()) {
        let mut layouts = panels.clone();
        // Sections are shared definitions but assigned EXCLUSIVELY: the
        // first panel takes the sections not claimed by the others, so
        // adding a secondary panel does not duplicate content. An explicit
        // first-panel list wins (and may intentionally duplicate a section).
        let claimed: Vec<String> = layouts
            .iter()
            .skip(1)
            .flat_map(|panel| panel.sections.iter().flatten().cloned())
            .collect();
        if let Some(first) = layouts.first_mut() {
            if first.sections.as_ref().is_none_or(|sections| sections.is_empty()) {
                first.sections = Some(
                    cfg.panel
                        .sections
                        .iter()
                        .map(|section| section.name.clone())
                        .filter(|name| !claimed.iter().any(|claimed| claimed == name))
                        .collect(),
                );
            }
            // Legacy root placement fills gaps in the first array entry.
            first.output = first.output.clone().or_else(|| cfg.panel.output.clone());
            first.anchor = first.anchor.clone().or_else(|| cfg.panel.anchor.clone());
            first.align = first.align.clone().or_else(|| cfg.panel.align.clone());
        }
        // An extra panel with no list shows an empty bar; validation flags
        // it so the editor cannot silently create a useless panel.
        for panel in layouts.iter_mut().skip(1) {
            if panel.sections.as_ref().is_none_or(|sections| sections.is_empty()) {
                panel.sections = Some(Vec::new());
            }
        }
        return layouts;
    }
    vec![PanelLayout {
        output: cfg.panel.output.clone(),
        anchor: cfg.panel.anchor.clone(),
        align: cfg.panel.align.clone(),
        sections: None,
    }]
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
    /// How a sensor value animates when it changes: `off` (default),
    /// `underline` (fade + underline sweep) or `sheen` (fade + light
    /// sweep over the value). Only sensors with `effect: true` animate.
    pub value_effect: Option<String>,
    /// Light or dark variant of the bar and the generated app themes:
    /// `light` or `dark`; absent = dark (the historic default). The
    /// configured colours seed the light variant; explicit per-sensor
    /// overrides stay untouched.
    pub theme_mode: Option<String>,
    pub default_section_weight: Option<u32>,
    pub default_bg_color: Option<String>,
    pub default_text_color: Option<String>,
    pub anchor: Option<String>,
    pub align: Option<String>,
    /// Output the bar binds to; absent or `"auto"` = the focused output
    /// (the compositor's choice). A monitor name is passed to the layer
    /// shell as `StartMode::TargetScreen`; a missing output falls back to
    /// the focused one. Requires a piuma restart when changed.
    pub output: Option<String>,
    /// Panel thickness (fixed surface dimension) in px; `None` = 32.
    pub thickness: Option<u32>,
    /// Style applied to every sensor; per-sensor values take precedence.
    pub sensor_defaults: Option<SensorDefaults>,
    /// Style applied to every section; per-section values take precedence.
    pub section_defaults: Option<SectionDefaults>,
    /// Freedesktop Desktop Notifications server (`notifications.rs`);
    /// present ⇔ the D-Bus server runs.
    pub notifications: Option<NotificationsConfig>,
    /// Session tweaks that patch the compositor at runtime (Tweaks tab);
    /// absent = every tweak off, nothing is queried or dispatched.
    pub tweaks: Option<TweaksConfig>,
    pub sections: Vec<SectionConfig>,
}

/// `panel.tweaks`: settings that do not belong to the panel's own
/// appearance. Every block is optional, so an absent key costs nothing.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct TweaksConfig {
    /// Shrink the gap between windows as more of them open.
    #[serde(default)]
    pub dynamic_gaps: Option<DynamicGapsConfig>,
    /// The floating menu that closes/floats/pins the focused window.
    #[serde(default)]
    pub window_menu: Option<WindowMenuConfig>,
    /// Hyprland's window shadows (`decoration:shadow:*`).
    #[serde(default)]
    pub shadows: Option<ShadowsConfig>,
    /// Hyprland's window borders (`general:border_size`, `general:col`).
    #[serde(default)]
    pub borders: Option<BordersConfig>,
    /// The idle screensaver: full-screen palette rectangles after
    /// `minutes` of inactivity, dismissed by any input.
    #[serde(default)]
    pub screensaver: Option<ScreensaverConfig>,
    /// Frosted glass: blur behind the bar and its popups, with the bar's
    /// background alpha lowered so the blur shows.
    #[serde(default)]
    pub glass: Option<GlassConfig>,
    /// Hide the bar while the focused window is fullscreen.
    #[serde(default)]
    pub auto_hide: Option<AutoHideConfig>,
}

/// `panel.tweaks.glass`: compositor blur (`ext-background-effect-v1`)
/// behind piuma's surfaces. `opacity` is the bar background's alpha
/// while the tweak is on (0.05..=1.0).
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct GlassConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_glass_opacity")]
    pub opacity: f32,
}

fn default_glass_opacity() -> f32 {
    0.35
}

/// `panel.tweaks.auto_hide`: the bar slides out of the output while the
/// focused window is fullscreen, and the reserved strip goes with it.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct AutoHideConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// `panel.tweaks.screensaver`: full-screen overlay after an idle period.
/// It never starts while the active window is fullscreen, and it does
/// not lock the session (a lock needs hyprlock and its own listener).
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct ScreensaverConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Idle minutes before the overlay appears (1..=120).
    #[serde(default = "default_screensaver_minutes")]
    pub minutes: u32,
}

fn default_screensaver_minutes() -> u32 {
    10
}

impl Config {
    /// The bar background's alpha while the glass tweak is on, else
    /// `None`.
    pub fn glass(&self) -> Option<f32> {
        self.panel
            .tweaks
            .as_ref()
            .and_then(|t| t.glass.as_ref())
            .filter(|g| g.enabled)
            .map(|g| g.opacity.clamp(0.05, 1.0))
    }

    /// Whether the bar hides itself while a fullscreen window has focus.
    pub fn auto_hide(&self) -> bool {
        self.panel
            .tweaks
            .as_ref()
            .and_then(|t| t.auto_hide.as_ref())
            .is_some_and(|a| a.enabled)
    }

    /// The screensaver is on and has a usable delay.
    pub fn screensaver(&self) -> Option<u32> {
        self.panel
            .tweaks
            .as_ref()
            .and_then(|t| t.screensaver.as_ref())
            .filter(|s| s.enabled)
            .map(|s| s.minutes.clamp(1, 120))
    }
}

/// `panel.tweaks.borders`: the border every window gets. While the block
/// is enabled the bar writes these values and puts the user's own back
/// when it is disabled or piuma exits.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct BordersConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Border thickness in px.
    #[serde(default = "default_border_size")]
    pub size: i32,
    /// Focused window's border, `#rrggbbaa`.
    #[serde(default = "default_border_active")]
    pub active: String,
    /// Every other window's border.
    #[serde(default = "default_border_inactive")]
    pub inactive: String,
}

fn default_border_size() -> i32 {
    1
}

fn default_border_active() -> String {
    "#33ccffee".to_string()
}

fn default_border_inactive() -> String {
    "#595959aa".to_string()
}

/// `panel.tweaks.shadows`: every option Hyprland's `decoration:shadow`
/// takes. While the block is enabled the bar writes these values and
/// puts the user's own back when it is disabled or piuma exits. Defaults
/// mirror Hyprland's, in the config's `#rrggbbaa` colour form.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct ShadowsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Blur radius of the shadow, 0..=100.
    #[serde(default = "default_shadow_range")]
    pub range: i32,
    /// Blur passes: higher is smoother and heavier, 1..=4.
    #[serde(default = "default_shadow_render_power")]
    pub render_power: i32,
    /// Hard edge instead of a soft gradient.
    #[serde(default)]
    pub sharp: bool,
    #[serde(default = "default_shadow_color")]
    pub color: String,
    #[serde(default = "default_shadow_color_inactive")]
    pub color_inactive: String,
    /// `[x, y]` pixels.
    #[serde(default)]
    pub offset: [i32; 2],
    /// Size multiplier, 0.1..=10.
    #[serde(default = "default_shadow_scale")]
    pub scale: f32,
}

fn default_shadow_range() -> i32 {
    10
}

fn default_shadow_render_power() -> i32 {
    1
}

fn default_shadow_color() -> String {
    // Hyprland's `0x99000000` (AARRGGBB) in the config's RGBA order.
    "#00000099".to_string()
}

fn default_shadow_color_inactive() -> String {
    "#ffffffff".to_string()
}

fn default_shadow_scale() -> f32 {
    1.0
}

/// `panel.tweaks.window_menu`: a small menu following the focused
/// window, one button per action. Off by default — when off, the bar
/// never opens the socket nor the surface.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct WindowMenuConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Where the menu sits relative to the window: 4 px above or below
    /// it, its left/right edge aligned with the window's.
    #[serde(default)]
    pub position: WindowMenuPosition,
}

/// The six places a window edge can host the menu: above or below the
/// window, aligned left / centred / right on it.
#[derive(Deserialize, serde::Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMenuPosition {
    TopLeft,
    #[default]
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// `panel.tweaks.dynamic_gaps`: the bar patches Hyprland's
/// `general:gaps_in` from the open-window count. `max` with one window,
/// then a 25% of the `max - min` range less per extra window, never
/// below `min`. The count is every open window (a global Hyprland value
/// must not re-tile on every workspace switch).
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct DynamicGapsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Gap with one (or zero) windows; also the default when absent.
    #[serde(default = "default_gap_min")]
    pub min: i32,
    /// Gap floor reached after enough windows.
    #[serde(default = "default_gap_max")]
    pub max: i32,
}

fn default_list_lines() -> u32 {
    10
}

fn default_gap_min() -> i32 {
    4
}

fn default_gap_max() -> i32 {
    16
}

/// `panel.wallpaper`: the desktop background the bar draws on the
/// background layer — no external wallpaper tool is involved. Every
/// field defaults so a partial block (or none at all) keeps old
/// configs working: disabled, image mode, no path/colour, no window.
#[derive(Deserialize, serde::Serialize, Clone)]
pub struct WallpaperConfig {
    /// Whether the bar shows the wallpaper at all.
    #[serde(default)]
    pub enabled: bool,
    /// `"image"` (default) or `"color"`: which background the bar draws.
    #[serde(default = "default_wallpaper_mode")]
    pub mode: String,
    /// Absolute path of the image, drawn with a cover fit over the
    /// whole output.
    #[serde(default)]
    pub path: String,
    /// Solid colour painted over the whole output when `mode` is
    /// `"color"`. Parsed with the same rules as every other colour.
    #[serde(default)]
    pub color: String,
}

fn default_wallpaper_mode() -> String {
    "image".to_string()
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
#[derive(Debug, Default, Deserialize, serde::Serialize, Clone)]
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
#[derive(Debug, Default, Deserialize, serde::Serialize, Clone)]
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
    /// The section can be collapsed to a clickable dot: the dot toggles
    /// it, and the bar lays out only that dot while collapsed.
    #[serde(default)]
    pub collapsible: bool,
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
    /// Opt this sensor into the panel's value effect (`panel.value_effect`):
    /// absent = off, so a global mode alone never animates everything.
    #[serde(default)]
    pub effect: bool,
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
    Command {
        command: String,
        /// Minimum seconds between two runs; absent/0 = every tick.
        /// The bar polls at 1 Hz, so expensive commands (network
        /// probes, `checkupdates`, …) opt into a slower cadence.
        #[serde(default)]
        interval: Option<u64>,
    },
    /// A command's stdout as a list: the bar shows the sensor's name (or
    /// the first line when the name is empty) and hovering lists up to
    /// `max_lines` lines.
    #[serde(rename = "list", alias = "List")]
    List {
        command: String,
        #[serde(default)]
        interval: Option<u64>,
        #[serde(default = "default_list_lines")]
        max_lines: u32,
    },
    #[serde(rename = "env_var", alias = "EnvVar")]
    EnvVar { env_var: String },
    #[serde(rename = "static_text", alias = "StaticText")]
    StaticText {
        /// Fixed text. Empty is valid when the sensor has an icon: the
        /// source is the "text/icon" sensor, either content fits.
        #[serde(default)]
        static_text: String,
    },
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
    /// Graphical volume meter: the same mixer reading as `perc_volume`,
    /// rendered as a bar with click/wheel control instead of text.
    #[serde(rename = "volume_meter")]
    VolumeMeter {
        #[serde(default)]
        device: String,
    },
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
    /// Active NetworkManager connection: Wi-Fi SSID, VPN or wired name
    /// (`offline` when nothing is up).
    #[serde(rename = "net_info")]
    NetInfo {},
    /// Wi-Fi state: connected + internet access (NetworkManager, with an
    /// iwd fallback), signal strength carried by the icon. No params.
    #[serde(rename = "wifi")]
    Wifi {},
    /// power-profiles-daemon's active profile (balanced, performance,
    /// power-saver); click cycles it.
    #[serde(rename = "power_profile")]
    PowerProfile {},
    /// BlueZ adapter state and connected devices; click toggles the
    /// adapter.
    #[serde(rename = "bluetooth")]
    Bluetooth {},
    /// Hyprland keyboard layout and Caps/Num lock of the main keyboard;
    /// click switches to the next configured layout.
    #[serde(rename = "keyboard")]
    Keyboard {},
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
    /// Purely graphical workspace indicator: `count` numbered circles,
    /// the active one filled and the others outlined.
    #[serde(rename = "workspace_dots")]
    WorkspaceDots {
        #[serde(default = "default_workspace_count")]
        count: u32,
    },
    /// System tray (StatusNotifierItem): renders the tray icons; the
    /// rendering and the whole D-Bus side live in `tray.rs`.
    /// MPRIS media player: title/artist of the selected player, click
    /// play/pause, wheel previous/next. Empty = no player (sensor hides).
    #[serde(rename = "media")]
    Media {},
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
    /// Window layout (roadmap layout-automatici): the click saves the
    /// current workspace's window arrangement in `piuma-layouts.json`
    /// under an automatic name (layouts.rs holds the engine and the
    /// file), and the bar re-applies it whenever every saved window
    /// (class + title) is open again with the exact saved count. A
    /// legacy `"name"` key from old configs is ignored: layouts are
    /// named by their window set.
    #[serde(rename = "layout")]
    Layout {},
    /// Lock screen trigger: the sensor shows its name or icon, and a
    /// click locks every output with the password overlay (PAM). No
    /// value, no configured action: the click *is* the action.
    #[serde(rename = "lock")]
    Lock {},
    /// Idle-inhibit toggle: "on" while piuma's lock screen must stay
    /// away; clicking flips it. No compositor inhibitor is created —
    /// only piuma's own idle lock consults it.
    #[serde(rename = "caffeine")]
    Caffeine {},
    /// Power menu trigger: clicking opens lock/logout/suspend/reboot/
    /// shutdown next to the bar.
    #[serde(rename = "power")]
    Power {},
    /// Theme picker: shows the active theme's name (or "Custom"), and a
    /// click opens the list; the choice is saved in `theme.json`.
    #[serde(rename = "theme")]
    Theme {},
    /// Dashboard trigger: clicking opens the full-screen command center.
    #[serde(rename = "dashboard")]
    Dashboard {},
}

// ── defaults ────────────────────────────────────────────────────────

/// Workspace circles shown by a `workspace_dots` sensor when the config
/// omits the count.
fn default_workspace_count() -> u32 {
    10
}

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
        collapsible: false,
        sensors: vec![
            SensorConfig { name: "CPU".to_string(), description: None, icon: None, source: SensorSource::PercCpu {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, effect: false, color_rules: None },
            SensorConfig { name: "GPU".to_string(), description: None, icon: None, source: SensorSource::PercGpu {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, effect: false, color_rules: None },
            SensorConfig { name: "MEM".to_string(), description: None, icon: None, source: SensorSource::DetailMem {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, effect: false, color_rules: None },
            SensorConfig { name: "DISK".to_string(), description: None, icon: None, source: SensorSource::PercDisk {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, effect: false, color_rules: None },
            SensorConfig { name: "UPTIME".to_string(), description: None, icon: None, source: SensorSource::Uptime {}, text_color: None, bg_color: None, text_color_hover: None, bg_color_hover: None, border_color: None, border_color_hover: None, border_width: None, border_width_hover: None, border_radius: None, border_type: None, padding: None, action: None, draw_graph: false, effect: false, color_rules: None },
        ],
    };

    Config {
        panels: None,
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
            value_effect: None,
            theme_mode: None,
            default_section_weight: Some(1),
            default_bg_color: None,
            default_text_color: None,
            anchor: Some("top".to_string()),
            align: Some("center".to_string()),
            output: None,
            thickness: None,
            sensor_defaults: None,
            section_defaults: None,
            notifications: None,
            tweaks: None,
            sections: vec![section],
        },
    }
}

/// The config a first start writes out and runs with: the shipped
/// `default_config.json`, compiled in (the built-in above is only the
/// last resort should it ever fail to parse).
fn shipped_config() -> Config {
    serde_json::from_str(include_str!("default_config.json"))
        .unwrap_or_else(|_| default_config())
}

// ── loading ─────────────────────────────────────────────────────────

fn default_config_path() -> PathBuf {
    let default_path = if let Some(config_home) = crate::xdg::config() {
        config_home.join("piuma").join("piuma.json")
    } else if let Some(home) = crate::xdg::home() {
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
pub const BORDER_TYPES: [&str; 8] = [
    "full",
    "corners",
    "underline",
    "upperline",
    "left",
    "right",
    "leaf-right",
    "leaf-left",
];

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
    if let Some(w) = &p.wallpaper {
        err_one_of(&mut errors, "panel.wallpaper.mode", Some(&w.mode), &["image", "color"]);
        if w.mode == "color" {
            if w.color.trim().is_empty() {
                if w.enabled {
                    errors.push(ConfigError {
                        path: "panel.wallpaper.color".to_string(),
                        message: "solid colour mode needs a colour".to_string(),
                    });
                }
            } else {
                err_color(&mut errors, "panel.wallpaper.color", Some(&w.color));
            }
        } else if !w.color.trim().is_empty() {
            err_color(&mut errors, "panel.wallpaper.color", Some(&w.color));
        }
    }
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
    err_one_of(
        &mut errors,
        "panel.value_effect",
        p.value_effect.as_deref(),
        &["off", "underline", "sheen"],
    );
    err_one_of(
        &mut errors,
        "panel.theme_mode",
        p.theme_mode.as_deref(),
        &["dark", "light"],
    );
    err_range_u32(&mut errors, "panel.thickness", p.thickness, 4, 256);
    err_range_u32(&mut errors, "panel.font_size", p.font_size, 1, 256);
    err_range_u32(&mut errors, "panel.default_section_weight", p.default_section_weight, 1, 1000);

    if let Some(c) = &p.screen_corners {
        err_color(&mut errors, "panel.screen_corners.color", Some(&c.color));
        err_range_u32(&mut errors, "panel.screen_corners.radius", Some(c.radius), 1, 256);
    }

    if let Some(g) = p.tweaks.as_ref().and_then(|t| t.dynamic_gaps.as_ref()) {
        let path = "panel.tweaks.dynamic_gaps";
        err_range(&mut errors, &format!("{path}.min"), Some(g.min), 1, 200);
        err_range(&mut errors, &format!("{path}.max"), Some(g.max), 1, 200);
        if g.min > g.max {
            errors.push(ConfigError {
                path: path.to_string(),
                message: "the minimum gap cannot exceed the maximum".to_string(),
            });
        }
    }

    if let Some(b) = p.tweaks.as_ref().and_then(|t| t.borders.as_ref()) {
        let path = "panel.tweaks.borders";
        err_range(&mut errors, &format!("{path}.size"), Some(b.size), 0, 20);
        err_color(&mut errors, &format!("{path}.active"), Some(&b.active));
        err_color(&mut errors, &format!("{path}.inactive"), Some(&b.inactive));
    }

    if let Some(s) = p.tweaks.as_ref().and_then(|t| t.shadows.as_ref()) {
        let path = "panel.tweaks.shadows";
        err_range(&mut errors, &format!("{path}.range"), Some(s.range), 0, 100);
        err_range(&mut errors, &format!("{path}.render_power"), Some(s.render_power), 1, 4);
        err_range(&mut errors, &format!("{path}.offset"), Some(s.offset[0]), -500, 500);
        err_range(&mut errors, &format!("{path}.offset"), Some(s.offset[1]), -500, 500);
        if !(0.1..=10.0).contains(&s.scale) {
            errors.push(ConfigError {
                path: format!("{path}.scale"),
                message: format!("{}: expected 0.1..=10", s.scale),
            });
        }
        err_color(&mut errors, &format!("{path}.color"), Some(&s.color));
        err_color(&mut errors, &format!("{path}.color_inactive"), Some(&s.color_inactive));
    }

    if let Some(g) = p.tweaks.as_ref().and_then(|t| t.glass.as_ref()) {
        if !(0.05..=1.0).contains(&g.opacity) {
            errors.push(ConfigError {
                path: "panel.tweaks.glass.opacity".to_string(),
                message: format!("{}: expected 0.05..=1", g.opacity),
            });
        }
    }

    if let Some(s) = p.tweaks.as_ref().and_then(|t| t.screensaver.as_ref()) {
        err_range_u32(
            &mut errors,
            "panel.tweaks.screensaver.minutes",
            Some(s.minutes),
            1,
            120,
        );
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
            // Text/icon sensor: either half may carry it. An icon name is
            // enough (the bar renders it even with an empty value); no
            // text and no icon leaves a sensor with nothing to draw.
            if let SensorSource::StaticText { static_text } = &sensor.source {
                let has_text = !static_text.trim().is_empty();
                let has_icon = sensor
                    .icon
                    .as_deref()
                    .is_some_and(|icon| !icon.trim().is_empty());
                if !has_text && !has_icon {
                    errors.push(ConfigError {
                        path: format!("{spath}.source"),
                        message: "text/icon sensor needs text or an icon".to_string(),
                    });
                }
            }
            if let SensorSource::WorkspaceDots { count } = &sensor.source {
                if !(1..=50).contains(count) {
                    errors.push(ConfigError {
                        path: format!("{spath}.source.count"),
                        message: format!("{count}: expected 1..=50"),
                    });
                }
            }
            if let SensorSource::Command { interval: Some(interval), .. }
            | SensorSource::List { interval: Some(interval), .. } = &sensor.source
            {
                if *interval > 86_400 {
                    errors.push(ConfigError {
                        path: format!("{spath}.source.interval"),
                        message: format!("{interval}: expected 0..=86400"),
                    });
                }
            }
            if let SensorSource::List { max_lines, .. } = &sensor.source {
                if !(1..=200).contains(max_lines) {
                    errors.push(ConfigError {
                        path: format!("{spath}.source.max_lines"),
                        message: format!("{max_lines}: expected 1..=200"),
                    });
                }
            }
        }
    }

    // Extra layouts: placement values and section names must exist. The
    // first array entry mirrors the root panel; validating it again is
    // harmless and keeps the paths unambiguous for the editor.
    if let Some(panels) = &cfg.panels {
        let names: Vec<&str> = cfg.panel.sections.iter().map(|s| s.name.as_str()).collect();
        for (index, layout) in panels.iter().enumerate() {
            err_one_of(
                &mut errors,
                &format!("panels[{index}].anchor"),
                layout.anchor.as_deref(),
                &[
                    "top", "bottom", "left", "right", "top-left", "top-right",
                    "bottom-left", "bottom-right",
                ],
            );
            err_one_of(
                &mut errors,
                &format!("panels[{index}].align"),
                layout.align.as_deref(),
                &["left", "center", "right", "fill"],
            );
            if layout.output.as_deref() == Some("all") && index > 0 {
                errors.push(ConfigError {
                    path: format!("panels[{index}].output"),
                    message: "only the first panel can mirror all monitors".to_string(),
                });
            }
            if index > 0 && layout.sections.as_ref().is_none_or(|sections| sections.is_empty()) {
                errors.push(ConfigError {
                    path: format!("panels[{index}].sections"),
                    message: "a secondary panel needs at least one section".to_string(),
                });
            }
            if let Some(sections) = &layout.sections {
                for name in sections {
                    if !names.iter().any(|known| known == name) {
                        errors.push(ConfigError {
                            path: format!("panels[{index}].sections"),
                            message: format!("{name:?}: no such section"),
                        });
                    }
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
            match serde_json::to_string_pretty(&shipped_config()) {
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

    // No file could be read: run with the same values the shipped
    // default carries (a broken/absent config must not start the bare
    // built-in bar while the file on disk holds a real layout).
    (shipped_config(), default_path)
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

    /// The shipped theme fragments: a `name` plus the colour keys the
    /// theme picker will merge into `panel`. Every colour parses and the
    /// text stays readable on the bar and on the sensor pills (WCAG
    /// 4.5:1), so the picker never offers an unreadable combination.
    #[test]
    fn shipped_themes_are_valid_and_readable() {
        #[derive(serde::Deserialize)]
        struct Theme {
            name: String,
            panel: ThemePanel,
        }
        #[derive(serde::Deserialize)]
        struct ThemePanel {
            theme_mode: Option<String>,
            wallpaper_palette: Option<bool>,
            default_bg_color: Option<String>,
            default_text_color: Option<String>,
            sensor_defaults: SensorDefaults,
            section_defaults: SectionDefaults,
        }

        let files: [&str; 10] = [
            include_str!("../themes/01-white-on-black.json"),
            include_str!("../themes/02-black-on-white.json"),
            include_str!("../themes/03-black-on-orange.json"),
            include_str!("../themes/04-white-on-green.json"),
            include_str!("../themes/05-cyan-on-navy.json"),
            include_str!("../themes/06-purple-on-graphite.json"),
            include_str!("../themes/07-lavender-on-indigo.json"),
            include_str!("../themes/08-amber-on-charcoal.json"),
            include_str!("../themes/09-ice-on-slate.json"),
            include_str!("../themes/10-blue-on-midnight.json"),
        ];
        for json in files {
            let theme: Theme =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("theme parse: {e}"));
            let name = &theme.name;
            let p = &theme.panel;
            assert!(p.wallpaper_palette != Some(true), "{name}: palette must stay off");
            assert!(
                matches!(p.theme_mode.as_deref(), Some("dark" | "light")),
                "{name}: theme_mode"
            );
            let colour = |value: &Option<String>, what: &str| {
                parse_color(value.as_deref().unwrap_or_else(|| panic!("{name}: missing {what}")))
                    .unwrap_or_else(|e| panic!("{name}: {what}: {e}"))
            };
            let bg = colour(&p.default_bg_color, "bar bg");
            let fg = colour(&p.default_text_color, "bar text");
            assert!(
                contrast(bg, fg) >= 4.5,
                "{name}: bar contrast {}",
                contrast(bg, fg)
            );
            let s = &p.sensor_defaults;
            let (sbg, stx) = (
                colour(&s.bg_color, "pill bg"),
                colour(&s.text_color, "pill text"),
            );
            assert!(
                contrast(sbg, stx) >= 4.5,
                "{name}: pill contrast {}",
                contrast(sbg, stx)
            );
            // Every other colour the theme sets must parse too.
            for (value, what) in [
                (&s.text_color_hover, "pill hover text"),
                (&s.bg_color_hover, "pill hover bg"),
                (&s.border_color, "pill border"),
                (&s.border_color_hover, "pill hover border"),
            ] {
                let _ = colour(value, what);
            }
            let sd = &p.section_defaults;
            for (value, what) in [
                (&sd.bg_color, "section bg"),
                (&sd.text_color, "section text"),
                (&sd.border_color, "section border"),
            ] {
                let _ = colour(value, what);
            }
        }
    }

    /// WCAG relative-luminance contrast between two opaque colours.
    fn contrast(a: Color, b: Color) -> f32 {
        let lum = |c: Color| {
            let channel = |v: f32| {
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
        };
        let (l1, l2) = (lum(a), lum(b));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The shipped default is the embedded file, not the built-in
    /// fallback: a broken embed must fail here instead of silently
    /// shipping the bare bar.
    #[test]
    fn shipped_default_is_the_embedded_file() {
        let shipped = shipped_config();
        assert_eq!(shipped.panel.sections.len(), 3, "embedded file parsed");
        assert_eq!(shipped.panel.sections[0].name, "left");
        assert_eq!(shipped.panel.font.as_deref(), Some("Noto Sans"));
    }

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
        let json = r##"{"panel":{"wallpaper":{"enabled":true,"mode":"color","path":"/tmp/a.png","color":"#112233"},"sections":[]}}"##;
        let cfg: Config = serde_json::from_str(json).expect("wallpaper must be valid");
        let wp = cfg.panel.wallpaper.as_ref().expect("block present");
        assert!(wp.enabled);
        assert_eq!(wp.mode, "color");
        assert_eq!(wp.path, "/tmp/a.png");
        assert_eq!(wp.color, "#112233");
        // A partial block falls back per field…
        let cfg: Config =
            serde_json::from_str(r#"{"panel":{"wallpaper":{},"sections":[]}}"#).expect("partial ok");
        let wp = cfg.panel.wallpaper.as_ref().expect("block present");
        assert!(!wp.enabled);
        assert_eq!(wp.mode, "image");
        assert!(wp.path.is_empty());
        assert!(wp.color.is_empty());
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

    #[test]
    fn wallpaper_validation_names_bad_mode_and_colour() {
        let cfg: Config = serde_json::from_str(
            r##"{"panel":{"wallpaper":{"enabled":true,"mode":"solid","color":"nope"},"sections":[]}}"##,
        )
        .expect("config must parse");
        let errors = validate_config(&cfg);
        let paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["panel.wallpaper.mode", "panel.wallpaper.color"]);

        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"wallpaper":{"enabled":true,"mode":"color","color":""},"sections":[]}}"#,
        )
        .expect("config must parse");
        let errors = validate_config(&cfg);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, "panel.wallpaper.color");

        let cfg: Config = serde_json::from_str(
            r##"{"panel":{"wallpaper":{"enabled":true,"mode":"color","color":"#112233"},"sections":[]}}"##,
        )
        .expect("config must parse");
        assert!(validate_config(&cfg).is_empty());
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

    /// The layout sensor parses with and without the legacy `"name"`
    /// key (layouts are named automatically now — the old key is
    /// ignored) and survives a save roundtrip without it.
    #[test]
    fn layout_sensor_parses_and_roundtrips() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[
                {"name":"L","source":{"type":"layout","name":"work"}},
                {"name":"D","source":{"type":"layout"}}
            ]}]}}"#,
        )
        .expect("sensor must parse");
        let sensors = &cfg.panel.sections[0].sensors;
        assert!(matches!(&sensors[0].source, SensorSource::Layout {}));
        assert!(matches!(&sensors[1].source, SensorSource::Layout {}));
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""type":"layout""#), "got: {out}");
        assert!(!out.contains(r#""name":"work""#), "the legacy name is dropped: {out}");
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

    /// The graphical volume meter shares the volume backend and keeps
    /// its device through a save/load roundtrip.
    #[test]
    fn volume_meter_parses_roundtrips_and_is_percent() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"source":{"type":"volume_meter"}}]}]}}"#,
        )
        .expect("volume meter must parse");
        assert!(matches!(
            &cfg.panel.sections[0].sensors[0].source,
            SensorSource::VolumeMeter { device } if device.is_empty()
        ));
        assert!(validate_config(&cfg).is_empty());
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"source":{"type":"volume_meter","device":"Speaker"}}]}]}}"#,
        )
        .expect("volume meter must parse");
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""volume_meter""#), "got: {out}");
        assert!(out.contains(r#""device":"Speaker""#), "got: {out}");
    }

    /// Multiple layouts: the array is authoritative when present; a
    /// missing array falls back to the single legacy panel.
    #[test]
    fn panel_layouts_use_the_array_or_the_legacy_panel() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"output":"HDMI-A-2","sections":[]},
                "panels":[
                  {"output":"HDMI-A-2","anchor":"top","sections":["A"]},
                  {"output":"eDP-1","anchor":"bottom","align":"fill","sections":["B"]}
                ]}"#,
        )
        .expect("panels must parse");
        let layouts = panel_layouts(&cfg);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[1].output.as_deref(), Some("eDP-1"));
        assert_eq!(layouts[1].align.as_deref(), Some("fill"));

        let legacy: Config = serde_json::from_str(
            r#"{"panel":{"output":"eDP-1","sections":[]}}"#,
        )
        .unwrap();
        let layouts = panel_layouts(&legacy);
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].output.as_deref(), Some("eDP-1"));

        let broken: Config = serde_json::from_str(
            r#"{"panel":{"sections":[]},"panels":[{"anchor":"nope"},{"output":"eDP-1","align":"diagonal"}]}"#,
        )
        .unwrap();
        let errors = validate_config(&broken);
        assert!(errors.iter().any(|e| e.path == "panels[0].anchor"));
        assert!(errors.iter().any(|e| e.path == "panels[1].align"));
    }

    /// Sections are assigned exclusively: the main panel defaults to the
    /// ones no secondary panel claimed, so nothing is duplicated.
    #[test]
    fn panel_sections_are_assigned_exclusively() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"name":"A","sensors":[]},{"name":"B","sensors":[]},{"name":"C","sensors":[]}]},
                "panels":[
                  {"output":"HDMI-A-2"},
                  {"output":"eDP-1","sections":["B"]}
                ]}"#,
        )
        .expect("panels must parse");
        let layouts = panel_layouts(&cfg);
        assert_eq!(
            layouts[0].sections,
            Some(vec!["A".to_string(), "C".to_string()]),
            "main keeps only unclaimed sections"
        );
        assert_eq!(layouts[1].sections, Some(vec!["B".to_string()]));

        // An explicit main list wins (an intentional duplicate is allowed).
        let explicit: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"name":"A","sensors":[]},{"name":"B","sensors":[]}]},
                "panels":[
                  {"sections":["A"]},
                  {"sections":["B"]}
                ]}"#,
        )
        .unwrap();
        assert_eq!(panel_layouts(&explicit)[0].sections, Some(vec!["A".to_string()]));

        // A secondary panel with no sections is invalid.
        let empty: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"name":"A","sensors":[]}]},
                "panels":[{"sections":["A"]},{}]}"#,
        )
        .unwrap();
        assert!(validate_config(&empty)
            .iter()
            .any(|e| e.path == "panels[1].sections"));
    }

    /// The Tweaks block is optional (absent = every tweak off) and the
    /// The list sensor: name/first line on the bar, the lines for the
    /// popup, and the values Hyprland-side validation refuses.
    #[test]
    fn list_sensor_parse_and_validate() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[
                {"name":"Downloads","source":{"type":"list","command":"ls ~/Downloads"}}
            ]}]}}"#,
        )
        .expect("list source must parse");
        let source = &cfg.panel.sections[0].sensors[0].source;
        assert!(matches!(
            source,
            SensorSource::List {
                max_lines: 10,
                interval: None,
                ..
            }
        ));
        assert!(validate_config(&cfg).is_empty());

        let bad: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[
                {"source":{"type":"list","command":"ls","max_lines":0,"interval":90000}}
            ]}]}}"#,
        )
        .unwrap();
        let errors = validate_config(&bad);
        for path in [
            "panel.sections[0].sensors[0].source.max_lines",
            "panel.sections[0].sensors[0].source.interval",
        ] {
            assert!(errors.iter().any(|e| e.path == path), "missing {path}");
        }
    }

    /// The border block: absent fields take Hyprland's own defaults, and
    /// the values Hyprland would reject are flagged.
    #[test]
    fn border_tweaks_parse_and_validate() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"borders":{"enabled":true}}}}"#,
        )
        .expect("borders must parse");
        let b = cfg
            .panel
            .tweaks
            .as_ref()
            .and_then(|t| t.borders.as_ref())
            .expect("borders");
        assert!(b.enabled);
        assert_eq!(b.size, 1);
        assert_eq!(b.active, "#33ccffee");
        assert_eq!(b.inactive, "#595959aa");
        assert!(validate_config(&cfg).is_empty());

        let bad: Config = serde_json::from_str(
            r##"{"panel":{"sections":[],"tweaks":{"borders":{"size":40,"active":"nope"}}}}"##,
        )
        .unwrap();
        let errors = validate_config(&bad);
        for path in ["panel.tweaks.borders.size", "panel.tweaks.borders.active"] {
            assert!(errors.iter().any(|e| e.path == path), "missing {path}");
        }
    }

    /// The screensaver block: absent fields take the bar's defaults (off,
    /// 10 minutes), the delay validates 1..=120, and the runtime helper
    /// clamps a hand-edited value instead of trusting it.
    #[test]
    fn screensaver_tweaks_parse_validate_and_clamp() {
        let mut cfg = default_config();
        assert_eq!(cfg.screensaver(), None, "absent block: no screen saver");

        let parsed: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"screensaver":{"enabled":true}}}}"#,
        )
        .expect("screensaver must parse");
        let saver = parsed
            .panel
            .tweaks
            .as_ref()
            .and_then(|t| t.screensaver.as_ref())
            .expect("screensaver");
        assert!(saver.enabled);
        assert_eq!(saver.minutes, 10, "the bar's default delay");
        assert_eq!(parsed.screensaver(), Some(10));
        assert!(validate_config(&parsed).is_empty());

        cfg.panel.tweaks = Some(TweaksConfig {
            dynamic_gaps: None,
            window_menu: None,
            shadows: None,
            borders: None,
            screensaver: Some(ScreensaverConfig {
                enabled: true,
                minutes: 0,
            }),
            glass: None,
            auto_hide: None,
        });
        assert!(
            validate_config(&cfg)
                .iter()
                .any(|e| e.path == "panel.tweaks.screensaver.minutes"),
            "0 minutes is a config error"
        );
        assert_eq!(cfg.screensaver(), Some(1), "the runtime clamps it");

        cfg.panel.tweaks.as_mut().unwrap().screensaver = Some(ScreensaverConfig {
            enabled: false,
            minutes: 30,
        });
        assert_eq!(cfg.screensaver(), None, "disabled means off");
    }

    /// The shadow block: absent fields take Hyprland's own defaults, and
    /// the values Hyprland would reject are flagged.
    #[test]
    fn shadows_tweaks_parse_and_validate() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"shadows":{"enabled":true}}}}"#,
        )
        .expect("shadows must parse");
        let sh = cfg
            .panel
            .tweaks
            .as_ref()
            .and_then(|t| t.shadows.as_ref())
            .expect("shadows");
        assert!(sh.enabled);
        assert_eq!((sh.range, sh.render_power), (10, 1));
        assert!(!sh.sharp);
        assert_eq!(sh.color, "#00000099");
        assert_eq!(sh.offset, [0, 0]);
        assert!((sh.scale - 1.0).abs() < 1e-6);
        assert!(validate_config(&cfg).is_empty());

        let bad: Config = serde_json::from_str(
            r##"{"panel":{"sections":[],"tweaks":{"shadows":{"range":200,"render_power":0,"scale":0.0,"color":"nope"}}}}"##,
        )
        .unwrap();
        let errors = validate_config(&bad);
        for path in [
            "panel.tweaks.shadows.range",
            "panel.tweaks.shadows.render_power",
            "panel.tweaks.shadows.scale",
            "panel.tweaks.shadows.color",
        ] {
            assert!(errors.iter().any(|e| e.path == path), "missing {path}");
        }
    }

    /// The floating menu's block: absent values default to off and
    /// top-center; an unknown position is a parse error (never a silent
    /// wrong placement).
    #[test]
    fn window_menu_tweaks_parse_and_validate() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"window_menu":{"enabled":true}}}}"#,
        )
        .expect("window menu must parse");
        let menu = cfg
            .panel
            .tweaks
            .as_ref()
            .and_then(|t| t.window_menu.as_ref())
            .expect("window menu");
        assert!(menu.enabled);
        assert_eq!(menu.position, WindowMenuPosition::TopCenter, "default");
        assert!(validate_config(&cfg).is_empty());

        let corners: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"window_menu":{"enabled":true,"position":"bottom-right"}}}}"#,
        )
        .unwrap();
        assert_eq!(
            corners.panel.tweaks.unwrap().window_menu.unwrap().position,
            WindowMenuPosition::BottomRight
        );

        assert!(
            serde_json::from_str::<Config>(
                r#"{"panel":{"sections":[],"tweaks":{"window_menu":{"position":"middle"}}}}"#
            )
            .is_err(),
            "an unknown position must not load"
        );
    }

    /// A wrapped dynamic-gaps range must not be inverted.
    #[test]
    fn dynamic_gaps_tweaks_parse_and_validate() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"dynamic_gaps":{"enabled":true}}}}"#,
        )
        .expect("tweaks must parse");
        let gaps = cfg
            .panel
            .tweaks
            .as_ref()
            .and_then(|t| t.dynamic_gaps.as_ref())
            .expect("dynamic gaps");
        assert!(gaps.enabled);
        assert_eq!((gaps.min, gaps.max), (4, 16), "serde defaults");
        assert!(validate_config(&cfg).is_empty());

        let bad: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"dynamic_gaps":{"enabled":true,"min":20,"max":10}}}}"#,
        )
        .unwrap();
        assert!(validate_config(&bad)
            .iter()
            .any(|e| e.path == "panel.tweaks.dynamic_gaps"));

        // The editor's sliders start at 1: a hand-edited 0 is refused.
        let zero: Config = serde_json::from_str(
            r#"{"panel":{"sections":[],"tweaks":{"dynamic_gaps":{"enabled":true,"min":0,"max":16}}}}"#,
        )
        .unwrap();
        assert!(validate_config(&zero)
            .iter()
            .any(|e| e.path == "panel.tweaks.dynamic_gaps.min"));
    }

    /// The leaf border shapes are part of the accepted set and validate.
    #[test]
    fn leaf_border_types_validate() {
        assert!(BORDER_TYPES.contains(&"leaf-right"));
        assert!(BORDER_TYPES.contains(&"leaf-left"));
        let cfg: Config = serde_json::from_str(
            r##"{"panel":{"sections":[{"sensors":[{"name":"L","source":{"type":"static_text","static_text":"L"},"border_color":"#ffffffff","border_width":2,"border_radius":8,"border_type":"leaf-right"}]}]}}"##,
        )
        .expect("leaf border parses");
        assert!(validate_config(&cfg).is_empty());
    }

    /// The runtime-state sources parse with no parameters, and a
    /// command's interval is bounded like every other numeric field.
    #[test]
    fn runtime_sensors_parse_and_command_interval_is_bounded() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[
                {"name":"N","source":{"type":"net_info"}},
                {"name":"P","source":{"type":"power_profile"}},
                {"name":"B","source":{"type":"bluetooth"}},
                {"name":"K","source":{"type":"keyboard"}},
                {"name":"W","source":{"type":"wifi"}},
                {"name":"C","source":{"type":"command","command":"date","interval":30}}
            ]}]}}"#,
        )
        .expect("sources must parse");
        let sources: Vec<&SensorSource> = cfg.panel.sections[0]
            .sensors
            .iter()
            .map(|sensor| &sensor.source)
            .collect();
        assert!(matches!(sources[0], SensorSource::NetInfo {}));
        assert!(matches!(sources[1], SensorSource::PowerProfile {}));
        assert!(matches!(sources[2], SensorSource::Bluetooth {}));
        assert!(matches!(sources[3], SensorSource::Keyboard {}));
        assert!(matches!(sources[4], SensorSource::Wifi {}));
        assert!(matches!(
            sources[5],
            SensorSource::Command { interval: Some(30), .. }
        ));
        assert!(validate_config(&cfg).is_empty());

        let bad: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[
                {"name":"C","source":{"type":"command","command":"date","interval":90000}}
            ]}]}}"#,
        )
        .unwrap();
        assert!(validate_config(&bad)
            .iter()
            .any(|error| error.path == "panel.sections[0].sensors[0].source.interval"));
    }

    /// The output selection roundtrips; absent means "focused output".
    #[test]
    fn output_key_roundtrips() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"output":"HDMI-A-2","sections":[]}}"#,
        )
        .expect("output must parse");
        assert_eq!(cfg.panel.output.as_deref(), Some("HDMI-A-2"));
        assert!(validate_config(&cfg).is_empty());
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(out.contains(r#""output":"HDMI-A-2""#), "got: {out}");
        let bare: Config = serde_json::from_str(r#"{"panel":{"sections":[]}}"#).unwrap();
        assert!(bare.panel.output.is_none());
    }

    /// The light/dark variant roundtrips and an unknown value is named.
    #[test]
    fn theme_mode_roundtrips_and_validates() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"theme_mode":"light","sections":[]}}"#,
        )
        .expect("theme_mode must parse");
        assert_eq!(cfg.panel.theme_mode.as_deref(), Some("light"));
        assert!(validate_config(&cfg).is_empty());
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""theme_mode":"light""#), "got: {out}");
        let broken: Config = serde_json::from_str(
            r#"{"panel":{"theme_mode":"sepia","sections":[]}}"#,
        )
        .unwrap();
        assert_eq!(validate_config(&broken)[0].path, "panel.theme_mode");
    }

    /// Value effects: global mode and per-sensor opt-in roundtrip, and
    /// an unknown mode is named at its path.
    #[test]
    fn value_effect_and_sensor_optin_roundtrip() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"value_effect":"underline","sections":[{"sensors":[{"name":"CPU","effect":true,"source":{"type":"perc_cpu"}}]}]}}"#,
        )
        .expect("effect config must parse");
        assert_eq!(cfg.panel.value_effect.as_deref(), Some("underline"));
        assert!(cfg.panel.sections[0].sensors[0].effect);
        assert!(validate_config(&cfg).is_empty());
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""value_effect":"underline""#), "got: {out}");
        assert!(out.contains(r#""effect":true"#), "got: {out}");

        let bad: Config = serde_json::from_str(
            r#"{"panel":{"value_effect":"flash","sections":[]}}"#,
        )
        .expect("unknown mode still parses");
        let errors = validate_config(&bad);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].path, "panel.value_effect");
    }

    /// The MPRIS media sensor has no parameters and roundtrips.
    #[test]
    fn media_sensor_parses_and_roundtrips() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"source":{"type":"media"}}]}]}}"#,
        )
        .expect("media sensor must parse");
        assert!(matches!(
            cfg.panel.sections[0].sensors[0].source,
            SensorSource::Media {}
        ));
        assert!(validate_config(&cfg).is_empty());
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""media""#), "got: {out}");
    }

    /// The graphical workspace sensor carries its circle count and
    /// rejects a count no workspace can fill.
    #[test]
    fn workspace_dots_parse_roundtrip_and_validate_count() {
        let cfg: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"source":{"type":"workspace_dots","count":12}}]}]}}"#,
        )
        .expect("workspace dots must parse");
        assert!(matches!(
            &cfg.panel.sections[0].sensors[0].source,
            SensorSource::WorkspaceDots { count: 12 }
        ));
        assert!(validate_config(&cfg).is_empty());
        let out = serde_json::to_string(&cfg).expect("config must serialize");
        assert!(out.contains(r#""workspace_dots""#), "got: {out}");
        assert!(out.contains(r#""count":12"#), "got: {out}");

        let bad: Config = serde_json::from_str(
            r#"{"panel":{"sections":[{"sensors":[{"source":{"type":"workspace_dots","count":0}}]}]}}"#,
        )
        .expect("count 0 still parses");
        let errors = validate_config(&bad);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].path, "panel.sections[0].sensors[0].source.count");
    }

    /// Text/Icon sensors: the text is optional when the sensor carries a
    /// (non-empty) icon; neither leaves nothing to render.
    #[test]
    fn static_text_accepts_an_icon_without_text() {
        let parse = |sensor: &str| {
            serde_json::from_str::<Config>(&format!(
                r#"{{"panel":{{"sections":[{{"sensors":[{sensor}]}}]}}}}"#
            ))
            .expect("sensor must parse")
        };
        let icon_only = parse(r#"{"icon":"folder","source":{"type":"static_text"}}"#);
        assert!(matches!(
            &icon_only.panel.sections[0].sensors[0].source,
            SensorSource::StaticText { static_text } if static_text.is_empty()
        ));
        assert_eq!(validate_config(&icon_only), Vec::new());
        assert_eq!(validate_config(&parse(
            r#"{"source":{"type":"static_text","static_text":"Menu"}}"#
        )), Vec::new());
        let empty = validate_config(&parse(r#"{"source":{"type":"static_text"}}"#));
        assert_eq!(empty.len(), 1, "{empty:?}");
        assert_eq!(empty[0].path, "panel.sections[0].sensors[0].source");
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


