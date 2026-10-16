//! Piuma Setup - GUI Configuration Tool
//! A visual tool to configure Piuma panel settings using iced 0.14

use iced::theme::Palette as IcedPalette;
use iced::widget::operation;
use iced::widget::text::Style as TextStyle;
use iced::widget::{
    button, column, container, scrollable, stack, text, toggler, Button, Column,
    Container, PickList, Row, Slider, TextInput,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding, Task, Theme};
use serde_json::{Map, Value as JsonValue};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// piuma's real config schema (same package, different bin target): the
// save path validates against it before writing, so a config piuma
// would reject is reported here instead of failing silently at load.
// config.rs serves both binaries: the bar-only helpers (colour parsing,
// default config, file loading) legitimately sit unused in the setup.
#[allow(dead_code)]
#[path = "../config.rs"]
mod config;

// Freedesktop autostart listing and the per-app switch (roadmap2 #2):
// the bar-only runner (spawn/launch) is not this bin's job — the
// module-wide allow keeps it out of the warning baseline.
#[allow(dead_code)]
#[path = "../autostart.rs"]
mod autostart;

// Freedesktop icon-name → file lookup shared with the bar (roadmap N1):
// the setup renders the same resolve/widget pair at the same theme and
// size, so the icon preview shows exactly what the bar will draw.
#[path = "../icons.rs"]
mod icons;

/// The `panel.wallpaper` block (roadmap2 #1): whether the bar draws the
/// desktop background itself and which image covers the output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WallpaperCfg {
    pub enabled: bool,
    pub path: String,
}

// Configuration structure for GUI - mirrors src/config.rs
#[derive(Debug, Clone)]
pub struct PiumaConfig {
    pub font: Option<String>,
    pub font_size: u32,
    /// Bold panel text (roadmap M2): `None` = key absent (the bar reads
    /// false) — written only when set, so a save never injects the key.
    pub bold: Option<bool>,
    /// Theme resolving the sensors' `icon` names (roadmap N1): `None` =
    /// key absent (the bar falls back to the system icon-theme setting).
    pub icon_theme: Option<String>,
    /// Freedesktop autostart launch with the bar (roadmap N2): `None` =
    /// key absent (the bar never scans or spawns anything).
    pub autostart: Option<bool>,
    /// The user's per-app autostart selection (`panel.autostart_disabled`):
    /// file names piuma must not launch. Empty = key absent; the bar
    /// prunes stale names before launching, and no autostart file is
    /// ever written — other desktop environments keep their own view.
    pub autostart_disabled: Vec<String>,
    /// GTK theme bridge generation (roadmap N3): `None` = key absent (no
    /// file written or removed).
    pub app_theme: Option<bool>,
    /// Desktop background the bar draws itself (roadmap2 #1): `None` =
    /// key absent (the bar opens no background window).
    pub wallpaper: Option<WallpaperCfg>,
    pub section_layout: String,
    pub default_section_weight: u32,
    pub default_bg_color: Option<String>,
    pub default_text_color: Option<String>,
    pub anchor: Option<String>,
    pub align: Option<String>,
    pub thickness: u32,
    pub sensor_defaults: SensorDefaults,
    pub section_defaults: SectionDefaults,
    /// `panel.notifications`: presence gates piuma's D-Bus server, so
    /// `None` and `Some` are meaningful states (the GUI toggle).
    pub notifications: Option<NotificationsCfg>,
    pub sections: Vec<Section>,
}

/// The `panel.notifications` block with resolved values: the editor
/// shows concrete values, so enabling writes every key (identical
/// behaviour to omitting them — these are piuma's runtime defaults:
/// server 5000/20/true, popup white text on black, grey border).
#[derive(Debug, Clone)]
pub struct NotificationsCfg {
    pub default_timeout_ms: u32,
    pub history_size: u32,
    pub replace_existing: bool,
    // Popup card style (mirrors `PopupStyle` in src/main.rs).
    pub font_size: u32,
    pub text_color: String,
    pub text_color_hover: String,
    pub bg_color: String,
    pub bg_color_hover: String,
    pub border_color: String,
    pub border_color_hover: String,
    pub border_width: i32,
    pub border_radius: i32,
    pub border_type: String,
    pub padding: i32,
    /// Popup screen position (roadmap M4): `auto` reproduces the historic
    /// opposite-corner layout; the six explicit keys place the cards.
    pub popup_position: String,
    /// Horizontal screen-edge margin in px (roadmap M4).
    pub popup_margin_h: i32,
    /// Vertical screen-edge margin in px (roadmap M4).
    pub popup_margin_v: i32,
}

/// Server defaults from `src/notifications.rs` (Settings::DEFAULTS) and
/// popup defaults from `src/main.rs` (PopupStyle::default).
impl Default for NotificationsCfg {
    fn default() -> Self {
        Self {
            default_timeout_ms: 5_000,
            history_size: 20,
            replace_existing: true,
            font_size: 12,
            text_color: "#ffffff".to_string(),
            text_color_hover: "#ffffff".to_string(),
            bg_color: "#000000".to_string(),
            bg_color_hover: "#000000".to_string(),
            border_color: "#808080".to_string(),
            border_color_hover: "#808080".to_string(),
            border_width: 1,
            border_radius: 10,
            border_type: "full".to_string(),
            padding: 12,
            popup_position: "auto".to_string(),
            popup_margin_h: 8,
            popup_margin_v: 8,
        }
    }
}

/// Style defaults for every sensor (style fields only, no action).
#[derive(Debug, Clone, Default)]
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

/// Style defaults for every section (style fields only, no layout/weight);
/// mirrors `panel.section_defaults` in src/config.rs.
#[derive(Debug, Clone, Default)]
pub struct SectionDefaults {
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub border_color: Option<String>,
    pub border_width: Option<i32>,
    pub border_radius: Option<i32>,
    pub border_type: Option<String>,
    /// Only sides NOT flush with a screen edge get rounded borders
    /// (roadmap M3); `None` (absent) = false.
    pub respect_screen_edges: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: Option<String>,
    /// Section layout override; `None` = inherit the panel `section_layout`.
    pub layout: Option<String>,
    pub weight: Option<u32>,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub border_color: Option<String>,
    pub border_width: Option<i32>,
    pub border_radius: Option<i32>,
    pub border_type: Option<String>,
    pub sensors: Vec<Sensor>,
}

#[derive(Debug, Clone)]
pub struct Sensor {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Freedesktop icon name rendered before the sensor (roadmap N1):
    /// `None` = key absent (text only).
    pub icon: Option<String>,
    pub source_type: String,
    pub source_config: SourceConfig,
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
    pub action: Option<String>,
    pub draw_graph: bool,
}

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub command: Option<String>,
    pub env_var: Option<String>,
    pub static_text: Option<String>,
    pub interface: Option<String>,
    pub device: Option<String>,
    pub format: Option<String>,
}

impl Default for PiumaConfig {
    fn default() -> Self {
        PiumaConfig {
            font: Some("Noto Sans".to_string()),
            font_size: 12,
            bold: None,
            icon_theme: None,
            autostart: None,
            autostart_disabled: Vec::new(),
            app_theme: None,
            wallpaper: None,
            section_layout: "horizontal".to_string(),
            default_section_weight: 1,
            default_bg_color: Some("#1a1b26".to_string()),
            default_text_color: Some("#e8e8e8".to_string()),
            anchor: Some("top".to_string()),
            align: Some("center".to_string()),
            thickness: 32,
            sensor_defaults: SensorDefaults::default(),
            section_defaults: SectionDefaults::default(),
            notifications: None,
            sections: vec![Section::default()],
        }
    }
}

impl Default for Section {
    fn default() -> Self {
        Section {
            name: Some("System".to_string()),
            layout: Some("horizontal".to_string()),
            weight: None,
            bg_color: Some("#2d2d3a".to_string()),
            text_color: Some("#ff6b6b".to_string()),
            border_color: None,
            border_width: None,
            border_radius: None,
            border_type: None,
            sensors: vec![Sensor::default()],
        }
    }
}

impl Default for Sensor {
    fn default() -> Self {
        Sensor {
            name: Some("CPU".to_string()),
            description: None,
            icon: None,
            source_type: "perc_cpu".to_string(),
            source_config: SourceConfig::default(),
            text_color: None,
            bg_color: None,
            text_color_hover: None,
            bg_color_hover: None,
            border_color: None,
            border_color_hover: None,
            border_width: None,
            border_width_hover: None,
            border_radius: None,
            border_type: None,
            padding: None,
            action: None,
            draw_graph: false,
        }
    }
}

impl Default for SourceConfig {
    fn default() -> Self {
        SourceConfig {
            command: None,
            env_var: None,
            static_text: None,
            interface: None,
            device: None,
            format: None,
        }
    }
}

// Message enum for handling GUI events
#[derive(Debug, Clone)]
pub enum Message {
    FontChanged(String),
    FontSizeChanged(u32),
    /// Bold panel text toggle (roadmap M2).
    BoldChanged(bool),
    /// Icon theme of the panel (roadmap N1); empty = absent.
    IconThemeChanged(String),
    /// Freedesktop autostart toggle (roadmap N2).
    AutostartChanged(bool),
    /// One autostart app switched in its tab: (file name, enable) — the
    /// write lands in the user's override file (roadmap2 #2).
    AutostartAppToggled(String, bool),
    /// GTK theme bridge toggle (roadmap N3).
    AppThemeChanged(bool),
    /// Wallpaper enable toggle (roadmap2 #1).
    WallpaperEnabled(bool),
    /// Wallpaper image path, from the scanned list or a hand-typed
    /// custom path (roadmap2 #1); one destination for both sources.
    WallpaperPathChanged(String),
    /// Sidebar category switch (roadmap N4).
    TabSelected(Tab),
    ThicknessChanged(u32),
    /// Sensor style defaults: (field name, value) and (field name, number)
    /// for the fields of `panel.sensor_defaults`.
    DefaultsStyleChanged(String, String),
    DefaultsNumberChanged(String, i32),
    /// Section border style of the card at `usize`: (field name, string
    /// value) for `border_color` / `border_type` (bg/text have own messages).
    SectionStyleChanged(usize, String, String),
    /// Section border style number of the card at `usize`: (field name, number)
    /// for `border_width` / `border_radius`.
    SectionNumberChanged(usize, String, i32),
    /// Section style defaults: same keys as the section style fields,
    /// for the fields of `panel.section_defaults`.
    SecDefaultsStyleChanged(String, String),
    SecDefaultsNumberChanged(String, i32),
    /// "Borders respect screen edges" toggle of Section Defaults (M3).
    SecDefaultsRespectEdges(bool),
    SectionLayoutChanged(String),
    DefaultSectionWeightChanged(u32),
    DefaultBgColorChanged(String),
    DefaultTextColorChanged(String),
    AnchorChanged(String),
    AlignChanged(String),
    /// `panel.notifications` editor: presence gates piuma's D-Bus server.
    NotificationsEnabled(bool),
    NotificationsTimeoutChanged(u32),
    NotificationsHistoryChanged(u32),
    NotificationsReplaceChanged(bool),
    /// Popup style of the notifications card, by JSON key: colors and
    /// `border_type` (strings), font/border/padding (numbers).
    NotifStyleChanged(String, String),
    NotifNumberChanged(String, i32),
    /// Popup screen position picklist (roadmap M4): one of the 6 corners/
    /// edges or `auto`.
    NotifPositionChanged(String),
    /// Opens the modal creating a section (name optional).
    AddSectionPressed,
    /// Opens the removal confirmation for one section.
    RemoveSectionPressed(usize),
    /// Moves a section one position up/down in the panel order.
    SectionMoveUp(usize),
    SectionMoveDown(usize),
    SectionNameChanged(usize, String),
    SectionLayoutChangedIndex(usize, String),
    SectionWeightChanged(usize, u32),
    SectionBgColorChanged(usize, String),
    SectionTextColorChanged(usize, String),
    /// Opens the modal creating a sensor (type + name) in the section.
    AddSensorPressed(usize),
    /// Opens the removal confirmation for one sensor.
    RemoveSensorPressed(usize, usize),
    /// Clones a sensor right after itself (same section).
    SensorDuplicated(usize, usize),
    /// Moves a sensor to the end of another section (or of its own).
    SensorMoveTo(usize, usize, usize),
    /// Moves a sensor one position up/down inside its section.
    SensorMoveUp(usize, usize),
    SensorMoveDown(usize, usize),
    SensorNameChanged(usize, usize, String),
    SensorDescriptionChanged(usize, usize, String),
    /// Icon name of the sensor (roadmap N1); empty = absent.
    SensorIconChanged(usize, usize, String),
    /// Opens the icon browser for one sensor (searchable theme names).
    BrowseIconsPressed(usize, usize),
    /// Icon chosen in the browser: (section, sensor, name); closes it.
    IconPicked(usize, usize, String),
    SensorSourceTypeChanged(usize, usize, String),
    SensorSourceTypeConfigChanged(usize, usize, String, String),
    SensorTextColorChanged(usize, usize, String),
    SensorBgColorChanged(usize, usize, String),
    SensorTextColorHoverChanged(usize, usize, String),
    SensorBgColorHoverChanged(usize, usize, String),
    SensorBorderColorChanged(usize, usize, String),
    SensorBorderColorHoverChanged(usize, usize, String),
    SensorBorderWidthChanged(usize, usize, i32),
    SensorBorderWidthHoverChanged(usize, usize, i32),
    SensorBorderRadiusChanged(usize, usize, i32),
    SensorBorderTypeChanged(usize, usize, String),
    SensorPaddingChanged(usize, usize, i32),
    SensorActionChanged(usize, usize, String),
    /// Graph toggle of one sensor (hover popup trend).
    SensorDrawGraphChanged(usize, usize, bool),
    /// Toggles the collapsible "Sensor Defaults" card.
    ToggleDefaults,
    /// Toggles the collapsible "Section Defaults" card.
    ToggleSecDefaults,
    /// Toggles the collapsible style block of one sensor: (section, sensor).
    ToggleSensorStyle(usize, usize),
    /// Toggles the accordion body of one section.
    ToggleSection(usize),
    /// Toggles the accordion body of one sensor: (section, sensor).
    ToggleSensor(usize, usize),
    /// Save: asks confirmation when the config was modified, then writes.
    SaveConfig,
    /// Opens the "save as" modal (write the config under a new name).
    SaveAs,
    /// Opens the modal to pick and load any *.json config of the directory.
    LoadConfig,
    /// Text typed in the active modal (file name / picked file).
    ModalTextChanged(String),
    /// Sensor type picked in the "new sensor" modal.
    ModalTypeChanged(String),
    /// Confirms the primary action of the active modal.
    ModalConfirm,
    /// Closes the active modal discarding its action.
    ModalCancel,
    ResetConfig,
    Status(String),
}

/// Modal dialogs stacked over the main view (see `view`).
#[derive(Debug, Clone)]
pub enum Modal {
    /// Overwrite `current_file` with the modified configuration.
    ConfirmSave,
    /// Save-as form: typed file name and whether it already exists.
    SaveAs { name: String, exists: bool },
    /// Extra confirmation: the save-as target file already exists.
    ConfirmSaveAs(String),
    /// Load picker: selected file among the *.json of the config directory.
    LoadFile { selected: String, files: Vec<String> },
    /// New-section form: optional name (an empty one keeps the default).
    AddSection { name: String },
    /// New-sensor form: target section, picked type, optional name.
    AddSensor { section_index: usize, kind: String, name: String },
    /// Removal confirmation for one section (and its sensors).
    ConfirmRemoveSection(usize),
    /// Removal confirmation for one sensor.
    ConfirmRemoveSensor(usize, usize),
    /// Icon browser: target sensor and the query filtering theme names.
    BrowseIcons { section_index: usize, sensor_index: usize, query: String },
}

/// Default config file: the one the running piuma reads.
const DEFAULT_FILE: &str = "piuma.json";

/// Directory holding the piuma configs (~/.config/piuma).
fn config_dir() -> PathBuf {
    dirs::config_dir().map(|d| d.join("piuma")).unwrap_or_else(|| PathBuf::from("."))
}

/// Full path of a config file name inside the piuma config directory.
fn file_path(name: &str) -> PathBuf {
    config_dir().join(name)
}

/// Sorted *.json file names of the piuma config directory (load picker).
fn list_config_files() -> Vec<String> {
    let mut files: Vec<String> = match fs::read_dir(config_dir()) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) == Some("json") {
                    path.file_name().and_then(|n| n.to_str()).map(|n| n.to_string())
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
}

/// Validates a typed config name: plain file name, ".json" appended when missing.
fn sanitize_file_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("File name is empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("File name must not contain a path".to_string());
    }
    if name.ends_with(".json") { Ok(name.to_string()) } else { Ok(format!("{}.json", name)) }
}

// Application state
/// The setup categories, shown one at a time behind the sidebar tabs
/// (roadmap N4). `Panel` is the startup tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Panel,
    Notifications,
    SensorDefaults,
    SectionDefaults,
    Sections,
    Wallpaper,
    AutostartApps,
}

impl Tab {
    /// Sidebar order.
    const ALL: [Tab; 7] = [
        Tab::Panel,
        Tab::Notifications,
        Tab::SectionDefaults,
        Tab::SensorDefaults,
        Tab::Sections,
        Tab::Wallpaper,
        Tab::AutostartApps,
    ];

    /// Label shown on the sidebar button.
    fn label(self) -> &'static str {
        match self {
            Tab::Panel => "Panel",
            Tab::Notifications => "Notifications",
            Tab::SensorDefaults => "Sensor Defaults",
            Tab::SectionDefaults => "Section Defaults",
            Tab::Sections => "Sections",
            Tab::Wallpaper => "Wallpaper",
        Tab::AutostartApps => "Autostart apps",
        }
    }
}

pub struct PiumaSetup {
    config: PiumaConfig,
    /// Sidebar tab currently shown (roadmap N4): only its card renders.
    active_tab: Tab,
    status_message: Option<String>,
    status_is_error: bool,
    /// Whether the collapsible "Sensor Defaults" card is open.
    defaults_expanded: bool,
    /// Whether the collapsible "Section Defaults" card is open.
    sec_defaults_expanded: bool,
    /// Indices of the sensors whose "Style overrides" block is open.
    sensor_style_open: HashSet<(usize, usize)>,
    /// Sections collapsed in the Sections accordion (absent = open).
    sections_collapsed: HashSet<usize>,
    /// Sensors whose accordion body is expanded (absent = collapsed).
    sensors_open: HashSet<(usize, usize)>,
    /// Config file currently edited (name inside the piuma config directory).
    current_file: String,
    /// Serialized config at the last load/save: differs = unsaved changes.
    snapshot: String,
    /// Modal dialog stacked over the view, when one is open.
    modal: Option<Modal>,
    /// Images found under the freedesktop wallpaper folders (roadmap2 #1),
    /// scanned once at startup for the Wallpaper tab's list.
    wallpaper_files: Vec<PathBuf>,
    /// Autostart `.desktop` files found under the freedesktop dirs
    /// (roadmap2 #2), scanned once at startup for the "Autostart apps"
    /// tab and re-read after every switch — the switches show the spec
    /// state merged with the config's own selection.
    autostart_apps: Vec<autostart::editor::App>,
}

impl PiumaSetup {
    /// Builds the initial state, loading the config in use (piuma.json) when
    /// present so the user starts from the real configuration, not from defaults.
    fn new() -> Self {
        let mut state = PiumaSetup {
            config: PiumaConfig::default(),
            active_tab: Tab::Panel,
            status_message: None,
            status_is_error: false,
            defaults_expanded: true,
            sec_defaults_expanded: true,
            sensor_style_open: HashSet::new(),
            sections_collapsed: HashSet::new(),
            sensors_open: HashSet::new(),
            current_file: DEFAULT_FILE.to_string(),
            snapshot: String::new(),
            modal: None,
            wallpaper_files: scan_wallpaper_dirs(&wallpaper_dirs()),
            autostart_apps: Vec::new(),
        };
        match fs::read_to_string(file_path(&state.current_file)) {
            Ok(content) => match state.load_from_json(&content) {
                Ok(()) => {
                    // Snapshot is only the dirty-check: if this ever failed
                    // (it cannot for a parsed config) the first save would
                    // just ask for confirmation and report any real error.
                    state.snapshot = state.serialize().unwrap_or_default();
                    state.set_status(format!("Loaded {}", state.current_file), false);
                }
                // Parse errors leave `snapshot` empty, so the first save always
                // asks for confirmation instead of silently overwriting.
                Err(e) => state.set_status(format!("Error parsing {}: {}", state.current_file, e), true),
            },
            Err(_) => {
                state.snapshot = state.serialize().unwrap_or_default();
                state.set_status("No config file found; starting from defaults".to_string(), false);
            }
        }
        // One scan for the tab, with the loaded selection applied (the
        // config was parsed just above on every path).
        state.autostart_apps = autostart::editor::apps(&state.config.autostart_disabled);
        state
    }

    fn set_status(&mut self, msg: String, is_error: bool) {
        self.status_message = Some(msg);
        self.status_is_error = is_error;
    }

    fn config_to_json(&self) -> JsonValue {
        let mut panel = Map::new();

        if let Some(font) = &self.config.font {
            panel.insert("font".to_string(), JsonValue::String(font.clone()));
        }
        panel.insert("font_size".to_string(), JsonValue::Number(self.config.font_size.into()));
        if let Some(b) = self.config.bold {
            panel.insert("bold".to_string(), JsonValue::Bool(b));
        }
        if let Some(theme) = &self.config.icon_theme {
            panel.insert("icon_theme".to_string(), JsonValue::String(theme.clone()));
        }
        if let Some(a) = self.config.autostart {
            panel.insert("autostart".to_string(), JsonValue::Bool(a));
        }
        if !self.config.autostart_disabled.is_empty() {
            panel.insert(
                "autostart_disabled".to_string(),
                JsonValue::Array(
                    self.config
                        .autostart_disabled
                        .iter()
                        .cloned()
                        .map(JsonValue::String)
                        .collect(),
                ),
            );
        }
        if let Some(t) = self.config.app_theme {
            panel.insert("app_theme".to_string(), JsonValue::Bool(t));
        }
        // The wallpaper block is written whole (present = configured,
        // absent = untouched, so a save never injects the key).
        if let Some(w) = &self.config.wallpaper {
            let mut wp = Map::new();
            wp.insert("enabled".to_string(), JsonValue::Bool(w.enabled));
            wp.insert("path".to_string(), JsonValue::String(w.path.clone()));
            panel.insert("wallpaper".to_string(), JsonValue::Object(wp));
        }
        panel.insert("section_layout".to_string(), JsonValue::String(self.config.section_layout.clone()));
        panel.insert("default_section_weight".to_string(), JsonValue::Number(self.config.default_section_weight.into()));

        if let Some(bg) = &self.config.default_bg_color {
            panel.insert("default_bg_color".to_string(), JsonValue::String(bg.clone()));
        }
        if let Some(tc) = &self.config.default_text_color {
            panel.insert("default_text_color".to_string(), JsonValue::String(tc.clone()));
        }
        if let Some(anchor) = &self.config.anchor {
            panel.insert("anchor".to_string(), JsonValue::String(anchor.clone()));
        }
        if let Some(align) = &self.config.align {
            panel.insert("align".to_string(), JsonValue::String(align.clone()));
        }
        panel.insert("thickness".to_string(), JsonValue::Number(self.config.thickness.into()));

        // `panel.notifications`: whole block in or out (it gates the D-Bus
        // server); the editor writes every key explicitly, style included.
        if let Some(n) = &self.config.notifications {
            let mut block = Map::new();
            block.insert(
                "default_timeout_ms".to_string(),
                JsonValue::Number(n.default_timeout_ms.into()),
            );
            block.insert("history_size".to_string(), JsonValue::Number(n.history_size.into()));
            block.insert("replace_existing".to_string(), JsonValue::Bool(n.replace_existing));
            block.insert(
                "font_size".to_string(),
                JsonValue::Number(n.font_size.into()),
            );
            block.insert(
                "popup_position".to_string(),
                JsonValue::String(n.popup_position.clone()),
            );
            block.insert(
                "popup_margin_h".to_string(),
                JsonValue::Number(n.popup_margin_h.into()),
            );
            block.insert(
                "popup_margin_v".to_string(),
                JsonValue::Number(n.popup_margin_v.into()),
            );
            for (k, v) in [
                ("text_color", &n.text_color),
                ("text_color_hover", &n.text_color_hover),
                ("bg_color", &n.bg_color),
                ("bg_color_hover", &n.bg_color_hover),
                ("border_color", &n.border_color),
                ("border_color_hover", &n.border_color_hover),
                ("border_type", &n.border_type),
            ] {
                block.insert(k.to_string(), JsonValue::String(v.clone()));
            }
            for (k, v) in [
                ("border_width", n.border_width),
                ("border_radius", n.border_radius),
                ("padding", n.padding),
            ] {
                block.insert(k.to_string(), JsonValue::Number(v.into()));
            }
            panel.insert("notifications".to_string(), JsonValue::Object(block));
        }

        // Sensor style defaults: written only when at least one field is set.
        let d = &self.config.sensor_defaults;
        let mut defaults_map = Map::new();
        for (key, val) in [
            ("text_color", &d.text_color),
            ("bg_color", &d.bg_color),
            ("text_color_hover", &d.text_color_hover),
            ("bg_color_hover", &d.bg_color_hover),
            ("border_color", &d.border_color),
            ("border_color_hover", &d.border_color_hover),
            ("border_type", &d.border_type),
        ] {
            if let Some(v) = val {
                defaults_map.insert(key.to_string(), JsonValue::String(v.clone()));
            }
        }
        for (key, val) in [
            ("border_width", &d.border_width),
            ("border_width_hover", &d.border_width_hover),
            ("border_radius", &d.border_radius),
            ("padding", &d.padding),
        ] {
            if let Some(v) = val {
                defaults_map.insert(key.to_string(), JsonValue::Number((*v).into()));
            }
        }
        if !defaults_map.is_empty() {
            panel.insert("sensor_defaults".to_string(), JsonValue::Object(defaults_map));
        }

        // Section style defaults: written only when at least one field is set.
        let sd = &self.config.section_defaults;
        let mut sec_map = Map::new();
        for (key, val) in [
            ("bg_color", &sd.bg_color),
            ("text_color", &sd.text_color),
            ("border_color", &sd.border_color),
            ("border_type", &sd.border_type),
        ] {
            if let Some(v) = val {
                sec_map.insert(key.to_string(), JsonValue::String(v.clone()));
            }
        }
        for (key, val) in [("border_width", &sd.border_width), ("border_radius", &sd.border_radius)] {
            if let Some(v) = val {
                sec_map.insert(key.to_string(), JsonValue::Number((*v).into()));
            }
        }
        if let Some(v) = sd.respect_screen_edges {
            sec_map.insert("respect_screen_edges".to_string(), JsonValue::Bool(v));
        }
        if !sec_map.is_empty() {
            panel.insert("section_defaults".to_string(), JsonValue::Object(sec_map));
        }

        let sections_vec: Vec<JsonValue> = self.config.sections.iter().map(|s| self.section_to_json(s)).collect();
        panel.insert("sections".to_string(), JsonValue::Array(sections_vec));

        let mut root = Map::new();
        root.insert("panel".to_string(), JsonValue::Object(panel));
        JsonValue::Object(root)
    }

    fn source_to_json(&self, sensor: &Sensor) -> JsonValue {
        let mut map = Map::new();
        map.insert("type".to_string(), JsonValue::String(sensor.source_type.clone()));

        match sensor.source_type.as_str() {
            // Parameterized types; load_sensor guarantees canonical names,
            // so the keys written here always match the piuma.json schema.
            "net_state" | "used_netdown" | "used_netup" => {
                if let Some(iface) = &sensor.source_config.interface {
                    map.insert("interface".to_string(), JsonValue::String(iface.clone()));
                }
            }
            "perc_volume" => {
                if let Some(device) = &sensor.source_config.device {
                    map.insert("device".to_string(), JsonValue::String(device.clone()));
                }
            }
            "clock" | "calendar" => {
                if let Some(format) = &sensor.source_config.format {
                    map.insert("format".to_string(), JsonValue::String(format.clone()));
                }
            }
            "command" => {
                if let Some(cmd) = &sensor.source_config.command {
                    map.insert("command".to_string(), JsonValue::String(cmd.clone()));
                }
            }
            "env_var" => {
                if let Some(ev) = &sensor.source_config.env_var {
                    map.insert("env_var".to_string(), JsonValue::String(ev.clone()));
                }
            }
            "static_text" => {
                if let Some(st) = &sensor.source_config.static_text {
                    map.insert("static_text".to_string(), JsonValue::String(st.clone()));
                }
            }
            _ => {}
        }

        JsonValue::Object(map)
    }

    fn sensor_to_json(&self, sensor: &Sensor) -> JsonValue {
        let mut map = Map::new();
        if let Some(name) = &sensor.name {
            map.insert("name".to_string(), JsonValue::String(name.clone()));
        }
        if let Some(desc) = &sensor.description {
            map.insert("description".to_string(), JsonValue::String(desc.clone()));
        }
        if let Some(icon) = &sensor.icon {
            map.insert("icon".to_string(), JsonValue::String(icon.clone()));
        }
        map.insert("source".to_string(), self.source_to_json(sensor));

        for (key, opt) in [
            ("text_color", &sensor.text_color),
            ("bg_color", &sensor.bg_color),
            ("text_color_hover", &sensor.text_color_hover),
            ("bg_color_hover", &sensor.bg_color_hover),
            ("border_color", &sensor.border_color),
            ("border_color_hover", &sensor.border_color_hover),
            ("border_type", &sensor.border_type),
        ] {
            if let Some(val) = opt {
                map.insert(key.to_string(), JsonValue::String(val.clone()));
            }
        }

        if let Some(action) = &sensor.action {
            map.insert("action".to_string(), JsonValue::String(action.clone()));
        }
        // Saved only when on: an absent key keeps the file minimal and
        // means "no graph" forever (serde default).
        if sensor.draw_graph {
            map.insert("draw_graph".to_string(), JsonValue::Bool(true));
        }

        for (key, opt) in [
            ("border_width", &sensor.border_width),
            ("border_width_hover", &sensor.border_width_hover),
            ("border_radius", &sensor.border_radius),
            ("padding", &sensor.padding),
        ] {
            if let Some(val) = opt {
                map.insert(key.to_string(), JsonValue::Number((*val).into()));
            }
        }

        JsonValue::Object(map)
    }

    fn section_to_json(&self, section: &Section) -> JsonValue {
        let mut map = Map::new();
        if let Some(name) = &section.name {
            map.insert("name".to_string(), JsonValue::String(name.clone()));
        }
        // Layout written only when set: an absent key means "inherit the
        // panel section_layout", writing it would freeze the inheritance.
        if let Some(layout) = &section.layout {
            map.insert("layout".to_string(), JsonValue::String(layout.clone()));
        }
        if let Some(w) = section.weight {
            map.insert("weight".to_string(), JsonValue::Number(w.into()));
        }
        if let Some(bg) = &section.bg_color {
            map.insert("bg_color".to_string(), JsonValue::String(bg.clone()));
        }
        if let Some(tc) = &section.text_color {
            map.insert("text_color".to_string(), JsonValue::String(tc.clone()));
        }
        if let Some(bc) = &section.border_color {
            map.insert("border_color".to_string(), JsonValue::String(bc.clone()));
        }
        for (key, val) in [("border_width", section.border_width), ("border_radius", section.border_radius)] {
            if let Some(v) = val {
                map.insert(key.to_string(), JsonValue::Number(v.into()));
            }
        }
        if let Some(bt) = &section.border_type {
            map.insert("border_type".to_string(), JsonValue::String(bt.clone()));
        }
        let sensors_vec: Vec<JsonValue> = section.sensors.iter().map(|s| self.sensor_to_json(s)).collect();
        map.insert("sensors".to_string(), JsonValue::Array(sensors_vec));
        JsonValue::Object(map)
    }

    /// Pretty-printed JSON of the current config: exactly what Save would
    /// write. A serialization error is returned so the save path can report
    /// it instead of writing an empty file (`unwrap_or_default` did that).
    fn serialize(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.config_to_json())
            .map_err(|e| format!("Error serializing config: {}", e))
    }

    /// Writes the config to `name` (atomic tmp+rename) and refreshes
    /// `current_file`/`snapshot` so the modification check restarts clean.
    fn save_config(&mut self, name: &str) -> Result<(), String> {
        // A removed .desktop must not leave a stale selection behind:
        // prune `autostart_disabled` against a fresh scan before writing
        // (the bar does the same before it launches anything).
        let existing = autostart::existing_files();
        self.config.autostart_disabled =
            autostart::prune_disabled(&existing, &self.config.autostart_disabled);
        self.autostart_apps = autostart::editor::apps(&self.config.autostart_disabled);
        let path = file_path(name);
        let json = self.serialize()?;
        // Validate against piuma's real schema before touching the file: a
        // config piuma would reject (e.g. a static_text sensor without its
        // text) must fail loudly here, not load as broken later.
        serde_json::from_str::<config::Config>(&json)
            .map_err(|e| format!("Save rejected, piuma would refuse this config: {}", e))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Error creating directory: {}", e))?;
        }
        // Atomic save: write to a temp file, then rename over the target so a
        // running piuma never reads a half-written config.
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, &json)
            .and_then(|_| fs::rename(&tmp_path, &path))
            .map_err(|e| format!("Error saving: {}", e))?;
        self.snapshot = json;
        self.current_file = name.to_string();
        Ok(())
    }

    /// Reads and applies the config file `name`, refreshing current_file/snapshot.
    fn load_config(&mut self, name: &str) -> Result<(), String> {
        let content = fs::read_to_string(file_path(name))
            .map_err(|_| format!("No config file found: {}", name))?;
        self.load_from_json(&content)?;
        self.current_file = name.to_string();
        self.snapshot = self.serialize().unwrap_or_default();
        // The newly loaded file carries its own selection: refresh the
        // tab so its switches show it (scanned fresh, like at startup).
        self.autostart_apps = autostart::editor::apps(&self.config.autostart_disabled);
        Ok(())
    }

    /// Saves to `name` and reports the outcome in the status bar, returning
    /// it so callers can react (e.g. keep a confirmation modal open on error).
    fn save_reporting(&mut self, name: &str) -> Result<(), String> {
        let outcome = self.save_config(name);
        match &outcome {
            Ok(()) => self.set_status(format!("Configuration saved to {:?}", file_path(name)), false),
            Err(e) => self.set_status(e.clone(), true),
        }
        outcome
    }

    /// Loads `name` and reports the outcome in the status bar (see save_reporting).
    fn load_reporting(&mut self, name: &str) -> Result<(), String> {
        let outcome = self.load_config(name);
        match &outcome {
            Ok(()) => self.set_status(format!("Loaded {}", name), false),
            Err(e) => self.set_status(e.clone(), true),
        }
        outcome
    }

    /// Swaps sensors `i` and `i+1` of a section, carrying their
    /// style-block-open flags so an expanded block follows its sensor.
    /// Rewrites both per-sensor open sets (style block + accordion body)
    /// after a structural change: `f` maps every (section, sensor) key to
    /// its new home, or `None` when it disappeared.
    fn remap_sensor_keys(&mut self, f: impl Fn(usize, usize) -> Option<(usize, usize)>) {
        self.sensor_style_open = self.sensor_style_open.drain().filter_map(|(s, i)| f(s, i)).collect();
        self.sensors_open = self.sensors_open.drain().filter_map(|(s, i)| f(s, i)).collect();
    }

    fn swap_sensors(&mut self, section_index: usize, sensor_index: usize) {
        let (a, b) = (sensor_index, sensor_index + 1);
        self.remap_sensor_keys(|s, i| {
            Some((
                s,
                if s == section_index && i == a {
                    b
                } else if s == section_index && i == b {
                    a
                } else {
                    i
                },
            ))
        });
        self.config.sections[section_index].sensors.swap(a, b);
    }

    /// Swaps two sections, remapping the sensors' style flags of both so
    /// the expanded blocks stay attached to their sensors.
    fn swap_sections(&mut self, a: usize, b: usize) {
        self.config.sections.swap(a, b);
        self.remap_sensor_keys(|s, i| Some((if s == a { b } else if s == b { a } else { s }, i)));
    }

    fn load_from_json(&mut self, content: &str) -> Result<(), String> {
        let json: JsonValue = serde_json::from_str(content).map_err(|e| e.to_string())?;
        let panel = json.get("panel").ok_or("Missing panel")?;
        // Reset to an all-absent state before applying the file: the
        // *_default() values are templates for items created in the UI, not
        // file content — seeding from them wrote defaults back on save
        // whenever a key was absent (and the old "keep previous state" merge
        // leaked leftovers of the previously loaded file). Errors above leave
        // the current config untouched. Non-Option fields always get written,
        // so they hold piuma's own fallbacks to stay behaviour-neutral:
        // font_size 14 = DEFAULT_FONT_SIZE, section_layout "horizontal" and
        // weight 1 = main.rs fallbacks, thickness 32 = config.rs default.
        self.config = PiumaConfig {
            font: None,
            font_size: 14,
            bold: None,
            icon_theme: None,
            autostart: None,
            autostart_disabled: Vec::new(),
            app_theme: None,
            wallpaper: None,
            section_layout: "horizontal".to_string(),
            default_section_weight: 1,
            default_bg_color: None,
            default_text_color: None,
            anchor: None,
            align: None,
            thickness: 32,
            sensor_defaults: SensorDefaults::default(),
            section_defaults: SectionDefaults::default(),
            notifications: None,
            sections: Vec::new(),
        };

        if let Some(font) = panel.get("font").and_then(|v| v.as_str()) {
            self.config.font = Some(font.to_string());
        }
        if let Some(fs_) = panel.get("font_size").and_then(|v| v.as_u64()) {
            self.config.font_size = fs_ as u32;
        }
        // Absent `bold` stays `None` (reset default): the bar reads false.
        self.config.bold = panel.get("bold").and_then(|v| v.as_bool());
        // Absent `icon_theme` stays `None`: the bar falls back to the
        // system icon-theme setting (roadmap N1).
        self.config.icon_theme = panel
            .get("icon_theme")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        // Absent `autostart` stays `None` (reset default): the bar reads
        // false and never scans the autostart directories (roadmap N2).
        self.config.autostart = panel.get("autostart").and_then(|v| v.as_bool());
        // Absent `autostart_disabled` stays empty (reset default): the
        // bar only skips what the spec itself skips. Stale names are
        // pruned at save and again by the bar's next start.
        self.config.autostart_disabled = panel
            .get("autostart_disabled")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        // Absent `app_theme` stays `None`: no bridge file is written or
        // removed (roadmap N3).
        self.config.app_theme = panel.get("app_theme").and_then(|v| v.as_bool());
        // Absent `wallpaper` stays `None`: the bar draws no background
        // (roadmap2 #1). A partial block falls back per field.
        self.config.wallpaper = panel.get("wallpaper").and_then(|v| {
            v.as_object().map(|o| WallpaperCfg {
                enabled: o.get("enabled").and_then(JsonValue::as_bool).unwrap_or(false),
                path: o.get("path").and_then(JsonValue::as_str).unwrap_or_default().to_string(),
            })
        });
        if let Some(sl) = panel.get("section_layout").and_then(|v| v.as_str()) {
            self.config.section_layout = sl.to_string();
        }
        if let Some(w) = panel.get("default_section_weight").and_then(|v| v.as_u64()) {
            self.config.default_section_weight = w as u32;
        }
        if let Some(bg) = panel.get("default_bg_color").and_then(|v| v.as_str()) {
            self.config.default_bg_color = Some(bg.to_string());
        }
        if let Some(tc) = panel.get("default_text_color").and_then(|v| v.as_str()) {
            self.config.default_text_color = Some(tc.to_string());
        }
        if let Some(anchor) = panel.get("anchor").and_then(|v| v.as_str()) {
            self.config.anchor = Some(anchor.to_string());
        }
        if let Some(align) = panel.get("align").and_then(|v| v.as_str()) {
            self.config.align = Some(align.to_string());
        }
        // Absent thickness resets to 32 (the config was reset at the top).
        self.config.thickness = panel.get("thickness").and_then(|v| v.as_u64()).map(|t| t as u32).unwrap_or(32);
        // `panel.notifications`: block presence gates piuma's D-Bus server;
        // missing/negative keys resolve to piuma's runtime defaults (server
        // 5000/20/true, popup white on black with a grey border).
        if let Some(block) = panel.get("notifications") {
            if block.is_object() {
                let num = |k: &str| block.get(k).and_then(|v| v.as_u64());
                let sign = |k: &str| block.get(k).and_then(|v| v.as_i64()).map(|v| v as i32);
                let str_ = |k: &str| block.get(k).and_then(|v| v.as_str()).map(str::to_string);
                // Absent or negative numerics mean "use the default".
                let pos = |v: Option<i32>, d: i32| v.filter(|x| *x >= 0).unwrap_or(d);
                let d = NotificationsCfg::default();
                self.config.notifications = Some(NotificationsCfg {
                    default_timeout_ms: num("default_timeout_ms")
                        .unwrap_or(5_000)
                        .min(u64::from(u32::MAX)) as u32,
                    history_size: num("history_size").unwrap_or(20).min(u64::from(u32::MAX)) as u32,
                    replace_existing: block
                        .get("replace_existing")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    font_size: num("font_size")
                        .filter(|v| (8..=64).contains(v))
                        .map(|v| v as u32)
                        .unwrap_or(d.font_size),
                    text_color: str_("text_color").unwrap_or(d.text_color),
                    text_color_hover: str_("text_color_hover").unwrap_or(d.text_color_hover),
                    bg_color: str_("bg_color").unwrap_or(d.bg_color),
                    bg_color_hover: str_("bg_color_hover").unwrap_or(d.bg_color_hover),
                    border_color: str_("border_color").unwrap_or(d.border_color),
                    border_color_hover: str_("border_color_hover")
                        .unwrap_or(d.border_color_hover),
                    border_width: pos(sign("border_width"), d.border_width),
                    border_radius: pos(sign("border_radius"), d.border_radius),
                    border_type: str_("border_type")
                        .filter(|t| {
                            matches!(
                                t.as_str(),
                                "full" | "corners" | "underline" | "upperline" | "left" | "right"
                            )
                        })
                        .unwrap_or(d.border_type),
                    padding: pos(sign("padding"), d.padding),
                    popup_position: str_("popup_position")
                        .filter(|p| {
                            matches!(
                                p.as_str(),
                                "auto"
                                    | "top-left"
                                    | "top-center"
                                    | "top-right"
                                    | "bottom-left"
                                    | "bottom-center"
                                    | "bottom-right"
                            )
                        })
                        .unwrap_or(d.popup_position),
                    popup_margin_h: pos(sign("popup_margin_h"), d.popup_margin_h),
                    popup_margin_v: pos(sign("popup_margin_v"), d.popup_margin_v),
                });
            }
        }
        if let Some(obj) = panel.get("sensor_defaults").and_then(|v| v.as_object()) {
            let d = &mut self.config.sensor_defaults;
            for (key, field) in [
                ("text_color", &mut d.text_color),
                ("bg_color", &mut d.bg_color),
                ("text_color_hover", &mut d.text_color_hover),
                ("bg_color_hover", &mut d.bg_color_hover),
                ("border_color", &mut d.border_color),
                ("border_color_hover", &mut d.border_color_hover),
                ("border_type", &mut d.border_type),
            ] {
                if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                    *field = Some(v.to_string());
                }
            }
            for (key, field) in [
                ("border_width", &mut d.border_width),
                ("border_width_hover", &mut d.border_width_hover),
                ("border_radius", &mut d.border_radius),
                ("padding", &mut d.padding),
            ] {
                // as_i64: -1 (disabled) must load too, as_u64 would drop it
                if let Some(v) = obj.get(key).and_then(|v| v.as_i64()) {
                    *field = Some(v as i32);
                }
            }
        }
        // Absent block stays at the all-None default (reset at the top).
        if let Some(obj) = panel.get("section_defaults").and_then(|v| v.as_object()) {
            let d = &mut self.config.section_defaults;
            for (key, field) in [
                ("bg_color", &mut d.bg_color),
                ("text_color", &mut d.text_color),
                ("border_color", &mut d.border_color),
                ("border_type", &mut d.border_type),
            ] {
                if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                    *field = Some(v.to_string());
                }
            }
            for (key, field) in [("border_width", &mut d.border_width), ("border_radius", &mut d.border_radius)] {
                // as_i64: -1 (disabled) must load too, as_u64 would drop it
                if let Some(v) = obj.get(key).and_then(|v| v.as_i64()) {
                    *field = Some(v as i32);
                }
            }
            // Roadmap M3: absent key stays `None` = false (reset default).
            d.respect_screen_edges = obj
                .get("respect_screen_edges")
                .and_then(|v| v.as_bool());
        }

        if let Some(sections) = panel.get("sections").and_then(|v| v.as_array()) {
            self.config.sections = sections.iter().filter_map(|s| self.load_section(s)).collect();
        }

        Ok(())
    }

    /// Parses one section with an all-absent seed: every key missing from the
    /// file stays missing on save (no default/template injection).
    fn load_section(&self, json: &JsonValue) -> Option<Section> {
        let obj = json.as_object()?;
        let mut section = Section {
            name: None,
            layout: None,
            weight: None,
            bg_color: None,
            text_color: None,
            border_color: None,
            border_width: None,
            border_radius: None,
            border_type: None,
            sensors: Vec::new(),
        };

        section.name = obj.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
        section.layout = obj.get("layout").and_then(|v| v.as_str()).map(|s| s.to_string());
        section.weight = obj.get("weight").and_then(|v| v.as_u64()).map(|w| w as u32);
        section.bg_color = obj.get("bg_color").and_then(|v| v.as_str()).map(|s| s.to_string());
        section.text_color = obj.get("text_color").and_then(|v| v.as_str()).map(|s| s.to_string());
        section.border_color = obj.get("border_color").and_then(|v| v.as_str()).map(|s| s.to_string());
        // as_i64: -1 (disabled) must load too, as_u64 would drop it
        section.border_width = obj.get("border_width").and_then(|v| v.as_i64()).map(|v| v as i32);
        section.border_radius = obj.get("border_radius").and_then(|v| v.as_i64()).map(|v| v as i32);
        section.border_type = obj.get("border_type").and_then(|v| v.as_str()).map(|s| s.to_string());
        if let Some(sensors) = obj.get("sensors").and_then(|v| v.as_array()) {
            section.sensors = sensors.iter().filter_map(|s| self.load_sensor(s)).collect();
        }

        Some(section)
    }

    /// Parses one sensor with an all-absent seed (the `Sensor::default()`
    /// name "CPU" is only for sensors created in the UI).
    fn load_sensor(&self, json: &JsonValue) -> Option<Sensor> {
        let obj = json.as_object()?;
        let mut sensor = Sensor::default();
        sensor.name = None;

        if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
            sensor.name = Some(name.to_string());
        }
        if let Some(desc) = obj.get("description").and_then(|v| v.as_str()) {
            sensor.description = Some(desc.to_string());
        }
        if let Some(icon) = obj.get("icon").and_then(|v| v.as_str()) {
            sensor.icon = Some(icon.to_string());
        }
        if let Some(source) = obj.get("source").and_then(|v| v.as_object()) {
            if let Some(type_) = source.get("type").and_then(|v| v.as_str()) {
                // Single conversion point: legacy names (ProcCpu, Network…)
                // become canonical here, so the picklist shows the new names
                // and Save writes them to piuma.json; piuma's serde aliases
                // read either spelling identically.
                sensor.source_type = canonical_type(type_).to_string();
            }
            if let Some(iface) = source.get("interface").and_then(|v| v.as_str()) {
                sensor.source_config.interface = Some(iface.to_string());
            }
            if let Some(device) = source.get("device").and_then(|v| v.as_str()) {
                sensor.source_config.device = Some(device.to_string());
            }
            if let Some(format) = source.get("format").and_then(|v| v.as_str()) {
                sensor.source_config.format = Some(format.to_string());
            }
            if let Some(command) = source.get("command").and_then(|v| v.as_str()) {
                sensor.source_config.command = Some(command.to_string());
            }
            if let Some(ev) = source.get("env_var").and_then(|v| v.as_str()) {
                sensor.source_config.env_var = Some(ev.to_string());
            }
            // Key must match the real schema: piuma and source_to_json both
            // use "static_text" ("text" was an internal-UI key, never a file key).
            if let Some(st) = source.get("static_text").and_then(|v| v.as_str()) {
                sensor.source_config.static_text = Some(st.to_string());
            }
        }

        for (key, field) in [
            ("text_color", &mut sensor.text_color),
            ("bg_color", &mut sensor.bg_color),
            ("text_color_hover", &mut sensor.text_color_hover),
            ("bg_color_hover", &mut sensor.bg_color_hover),
            ("border_color", &mut sensor.border_color),
            ("border_color_hover", &mut sensor.border_color_hover),
            ("border_type", &mut sensor.border_type),
        ] {
            if let Some(val) = obj.get(key).and_then(|v| v.as_str()) {
                *field = Some(val.to_string());
            }
        }

        for (key, field) in [
            ("border_width", &mut sensor.border_width),
            ("border_width_hover", &mut sensor.border_width_hover),
            ("border_radius", &mut sensor.border_radius),
            ("padding", &mut sensor.padding),
        ] {
            // as_i64: -1 (disabled) must load too, as_u64 would drop it
            if let Some(val) = obj.get(key).and_then(|v| v.as_i64()) {
                *field = Some(val as i32);
            }
        }

        // Action (click behaviour): loaded like every other field, otherwise
        // sensor_to_json omits it and a re-save silently drops it.
        if let Some(val) = obj.get("action").and_then(|v| v.as_str()) {
            sensor.action = Some(val.to_string());
        }
        // Graph toggle: absent = off (the config default).
        sensor.draw_graph = obj.get("draw_graph").and_then(|v| v.as_bool()).unwrap_or(false);

        Some(sensor)
    }
}

fn update(state: &mut PiumaSetup, message: Message) -> Task<Message> {
    match message {
        Message::FontChanged(value) => state.config.font = if value.is_empty() { None } else { Some(value) },
        Message::FontSizeChanged(value) => state.config.font_size = value,
        Message::BoldChanged(value) => {
            // On = explicit true; off drops back to absent (≈ false).
            state.config.bold = value.then_some(true);
        }
        Message::IconThemeChanged(value) => {
            state.config.icon_theme = if value.is_empty() { None } else { Some(value) };
        }
        Message::AutostartChanged(value) => {
            // On = explicit true; off drops back to absent (≈ false).
            state.config.autostart = value.then_some(true);
        }
        Message::AutostartAppToggled(file, enabled) => {
            // The selection lives in the config (`panel.autostart_disabled`):
            // the switch rewrites that list and the tab re-reads the
            // effective state; the Save persists it. No autostart file is
            // ever written, so other desktop environments keep their own,
            // unaffected view.
            match autostart::editor::toggle_disabled(
                &mut state.config.autostart_disabled,
                &file,
                enabled,
            ) {
                Ok(()) => {
                    state.autostart_apps =
                        autostart::editor::apps(&state.config.autostart_disabled);
                    let verb = if enabled { "enabled" } else { "disabled" };
                    state.set_status(format!("{} {}", verb, file), false);
                }
                Err(e) => state.set_status(format!("{}: {}", file, e), true),
            }
        }
        Message::AppThemeChanged(value) => {
            // On = explicit true; off drops back to absent (≈ false).
            state.config.app_theme = value.then_some(true);
        }
        Message::WallpaperEnabled(value) => {
            // The toggle only flips `enabled`: the chosen path survives
            // both directions (the surface opens/closes on save + reload).
            wallpaper_set(&mut state.config, |w| w.enabled = value);
        }
        Message::WallpaperPathChanged(path) => {
            // List pick and typed path share the destination; the bar
            // checks existence itself, so a not-yet-there path is fine.
            wallpaper_set(&mut state.config, |w| w.path = path);
        }
        Message::TabSelected(tab) => state.active_tab = tab,
        Message::ThicknessChanged(value) => state.config.thickness = value,
        Message::DefaultsStyleChanged(field, value) => {
            let d = &mut state.config.sensor_defaults;
            let v = if value.is_empty() { None } else { Some(value) };
            match field.as_str() {
                "text_color" => d.text_color = v,
                "bg_color" => d.bg_color = v,
                "text_color_hover" => d.text_color_hover = v,
                "bg_color_hover" => d.bg_color_hover = v,
                "border_color" => d.border_color = v,
                "border_color_hover" => d.border_color_hover = v,
                "border_type" => d.border_type = v,
                _ => {}
            }
        }
        Message::DefaultsNumberChanged(field, value) => {
            let d = &mut state.config.sensor_defaults;
            match field.as_str() {
                "border_width" => d.border_width = Some(value),
                "border_width_hover" => d.border_width_hover = Some(value),
                "border_radius" => d.border_radius = Some(value),
                "padding" => d.padding = Some(value),
                _ => {}
            }
        }
        Message::SectionStyleChanged(index, field, value) => {
            if index < state.config.sections.len() {
                // Border style keys; section bg/text keep their own messages.
                let v = if value.is_empty() { None } else { Some(value) };
                let s = &mut state.config.sections[index];
                match field.as_str() {
                    "border_color" => s.border_color = v,
                    "border_type" => s.border_type = v,
                    _ => {}
                }
            }
        }
        Message::SectionNumberChanged(index, field, value) => {
            if index < state.config.sections.len() {
                let s = &mut state.config.sections[index];
                match field.as_str() {
                    "border_width" => s.border_width = Some(value),
                    "border_radius" => s.border_radius = Some(value),
                    _ => {}
                }
            }
        }
        Message::SecDefaultsStyleChanged(field, value) => {
            let d = &mut state.config.section_defaults;
            let v = if value.is_empty() { None } else { Some(value) };
            match field.as_str() {
                "bg_color" => d.bg_color = v,
                "text_color" => d.text_color = v,
                "border_color" => d.border_color = v,
                "border_type" => d.border_type = v,
                _ => {}
            }
        }
        Message::SecDefaultsNumberChanged(field, value) => {
            let d = &mut state.config.section_defaults;
            match field.as_str() {
                "border_width" => d.border_width = Some(value),
                "border_radius" => d.border_radius = Some(value),
                _ => {}
            }
        }
        Message::SecDefaultsRespectEdges(value) => {
            // Off = absent key (the bar reads false), on = explicit true.
            state.config.section_defaults.respect_screen_edges =
                value.then_some(true);
        }
        Message::SectionLayoutChanged(value) => state.config.section_layout = value,
        Message::DefaultSectionWeightChanged(value) => state.config.default_section_weight = value,
        Message::DefaultBgColorChanged(value) => state.config.default_bg_color = if value.is_empty() { None } else { Some(value) },
        Message::DefaultTextColorChanged(value) => state.config.default_text_color = if value.is_empty() { None } else { Some(value) },
        Message::AnchorChanged(value) => state.config.anchor = if value.is_empty() { None } else { Some(value) },
        Message::AlignChanged(value) => state.config.align = if value.is_empty() { None } else { Some(value) },
        Message::NotificationsEnabled(on) => {
            // Enabling from scratch seeds the defaults; disabling drops the
            // block, which is what turns piuma's D-Bus server off.
            state.config.notifications = match (on, state.config.notifications.take()) {
                (true, Some(prev)) => Some(prev),
                (true, None) => Some(NotificationsCfg::default()),
                (false, _) => None,
            };
        }
        Message::NotificationsTimeoutChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.default_timeout_ms = value;
            }
        }
        Message::NotificationsHistoryChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.history_size = value;
            }
        }
        Message::NotificationsReplaceChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.replace_existing = value;
            }
        }
        Message::NotifStyleChanged(key, value) => {
            if let Some(n) = &mut state.config.notifications {
                set_notif_style(n, &key, &value);
            }
        }
        Message::NotifNumberChanged(key, value) => {
            if let Some(n) = &mut state.config.notifications {
                set_notif_number(n, &key, value);
            }
        }
        Message::NotifPositionChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.popup_position = value;
            }
        }
        Message::AddSectionPressed => {
            state.modal = Some(Modal::AddSection { name: String::new() });
        }
        Message::RemoveSectionPressed(index) => {
            // The last section is never removable (the view also disables
            // its button); the confirmation names the collateral sensors.
            if state.config.sections.len() > 1 && index < state.config.sections.len() {
                state.modal = Some(Modal::ConfirmRemoveSection(index));
            }
        }
        Message::SectionMoveUp(index) => {
            if index > 0 {
                state.swap_sections(index - 1, index);
            }
        }
        Message::SectionMoveDown(index) => {
            if index + 1 < state.config.sections.len() {
                state.swap_sections(index, index + 1);
            }
        }
        Message::SectionNameChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].name = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::SectionLayoutChangedIndex(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].layout = Some(value);
            }
        }
        Message::SectionWeightChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].weight = Some(value);
            }
        }
        Message::SectionBgColorChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].bg_color = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::SectionTextColorChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].text_color = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::AddSensorPressed(section_index) => {
            if section_index < state.config.sections.len() {
                state.modal = Some(Modal::AddSensor {
                    section_index,
                    kind: "perc_cpu".to_string(),
                    name: String::new(),
                });
            }
        }
        Message::RemoveSensorPressed(section_index, sensor_index) => {
            if section_index < state.config.sections.len()
                && sensor_index < state.config.sections[section_index].sensors.len()
            {
                state.modal = Some(Modal::ConfirmRemoveSensor(section_index, sensor_index));
            }
        }
        Message::SensorDuplicated(section_index, sensor_index) => {
            if let Some(mut copy) = state
                .config
                .sections
                .get(section_index)
                .and_then(|sec| sec.sensors.get(sensor_index))
                .cloned()
            {
                // The copy is named after the original so it is recognizable.
                copy.name = copy.name.map(|n| format!("{n} (copy)"));
                let new_index = sensor_index + 1;
                state.remap_sensor_keys(|s, i| {
                    Some((s, if s == section_index && i >= new_index { i + 1 } else { i }))
                });
                state.config.sections[section_index].sensors.insert(new_index, copy);
                state.sensors_open.insert((section_index, new_index));
                state.sections_collapsed.remove(&section_index);
                // Focus the copy's name so renaming it is the next keystroke.
                return operation::focus(sensor_name_id(section_index, new_index));
            }
            return Task::none();
        }
        Message::SensorMoveTo(section_index, sensor_index, target) => {
            let in_bounds = section_index < state.config.sections.len()
                && sensor_index < state.config.sections[section_index].sensors.len()
                && target < state.config.sections.len();
            if in_bounds {
                let was_open = state.sensors_open.contains(&(section_index, sensor_index));
                let was_styled = state.sensor_style_open.contains(&(section_index, sensor_index));
                let sensor = state.config.sections[section_index].sensors.remove(sensor_index);
                // Removal first, then insertion at the target's end: for the
                // same section this reads as "move to its own end".
                state.remap_sensor_keys(|s, i| {
                    if s == section_index && i == sensor_index {
                        None
                    } else if s == section_index && i > sensor_index {
                        Some((s, i - 1))
                    } else {
                        Some((s, i))
                    }
                });
                let target_len = state.config.sections[target].sensors.len();
                state.config.sections[target].sensors.insert(target_len, sensor);
                state.remap_sensor_keys(|s, i| {
                    Some((s, if s == target && i >= target_len { i + 1 } else { i }))
                });
                // Open blocks follow the sensor; the target section unfolds
                // so the move is visible where it landed.
                if was_open {
                    state.sensors_open.insert((target, target_len));
                }
                if was_styled {
                    state.sensor_style_open.insert((target, target_len));
                }
                state.sections_collapsed.remove(&target);
            }
        }
        Message::SensorMoveUp(section_index, sensor_index) => {
            if section_index < state.config.sections.len() && sensor_index > 0 {
                state.swap_sensors(section_index, sensor_index - 1);
            }
        }
        Message::SensorMoveDown(section_index, sensor_index) => {
            if section_index < state.config.sections.len()
                && sensor_index + 1 < state.config.sections[section_index].sensors.len()
            {
                state.swap_sensors(section_index, sensor_index);
            }
        }
        Message::SensorNameChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].name = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorDescriptionChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].description = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorIconChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].icon = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::BrowseIconsPressed(section_index, sensor_index) => {
            let in_bounds = section_index < state.config.sections.len()
                && sensor_index < state.config.sections[section_index].sensors.len();
            if in_bounds {
                // The search starts from the current name, so refining an
                // existing icon needs no retype.
                let current = state.config.sections[section_index].sensors[sensor_index]
                    .icon
                    .clone()
                    .unwrap_or_default();
                state.modal = Some(Modal::BrowseIcons {
                    section_index,
                    sensor_index,
                    query: current,
                });
            }
        }
        Message::IconPicked(section_index, sensor_index, name) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].icon =
                        if name.is_empty() { None } else { Some(name) };
                }
            }
            state.modal = None;
        }
        Message::SensorSourceTypeChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].source_type = value;
                }
            }
        }
        Message::SensorSourceTypeConfigChanged(section_index, sensor_index, key, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    let sensor = &mut section.sensors[sensor_index];
                    match key.as_str() {
                        "interface" => sensor.source_config.interface = if value.is_empty() { None } else { Some(value) },
                        "device" => sensor.source_config.device = if value.is_empty() { None } else { Some(value) },
                        "format" => sensor.source_config.format = if value.is_empty() { None } else { Some(value) },
                        "command" => sensor.source_config.command = if value.is_empty() { None } else { Some(value) },
                        "env_var" => sensor.source_config.env_var = if value.is_empty() { None } else { Some(value) },
                        "text" => sensor.source_config.static_text = if value.is_empty() { None } else { Some(value) },
                        _ => {}
                    }
                }
            }
        }
        Message::SensorTextColorChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].text_color = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBgColorChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].bg_color = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorTextColorHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].text_color_hover = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBgColorHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].bg_color_hover = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBorderColorChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_color = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBorderColorHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_color_hover = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBorderWidthChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_width = Some(value);
                }
            }
        }
        Message::SensorBorderWidthHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_width_hover = Some(value);
                }
            }
        }
        Message::SensorBorderRadiusChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_radius = Some(value);
                }
            }
        }
        Message::SensorBorderTypeChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_type = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorPaddingChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].padding = Some(value);
                }
            }
        }
        Message::SensorActionChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].action = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorDrawGraphChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].draw_graph = value;
                }
            }
        }
        Message::SaveConfig => match state.serialize() {
            // Serialization errors surface in the status bar; otherwise the
            // dirty-check decides between straight save and confirmation.
            Err(e) => state.set_status(e, true),
            Ok(current) => {
                if current == state.snapshot {
                    let name = state.current_file.clone();
                    let _ = state.save_reporting(&name);
                } else {
                    // Unsaved changes: ask before overwriting the file.
                    state.modal = Some(Modal::ConfirmSave);
                }
            }
        },
        Message::SaveAs => {
            state.modal = Some(Modal::SaveAs {
                name: state.current_file.clone(),
                exists: file_path(&state.current_file).exists(),
            });
        }
        Message::LoadConfig => {
            // Picker lists the *.json of the config dir, preselecting the file in use.
            let files = list_config_files();
            let selected = if files.contains(&state.current_file) {
                state.current_file.clone()
            } else {
                files.first().cloned().unwrap_or_default()
            };
            state.modal = Some(Modal::LoadFile { selected, files });
        }
        Message::ModalTextChanged(value) => match state.modal.as_mut() {
            Some(Modal::SaveAs { name, exists }) => {
                *exists = sanitize_file_name(&value).map(|n| file_path(&n).exists()).unwrap_or(false);
                *name = value;
            }
            Some(Modal::LoadFile { selected, .. }) => *selected = value,
            Some(Modal::AddSection { name }) | Some(Modal::AddSensor { name, .. }) => *name = value,
            Some(Modal::BrowseIcons { query, .. }) => *query = value,
            _ => {}
        },
        Message::ModalTypeChanged(kind) => {
            if let Some(Modal::AddSensor { kind: slot, .. }) = state.modal.as_mut() {
                *slot = kind;
            }
        }
        Message::ModalConfirm => match state.modal.take() {
            Some(Modal::ConfirmSave) => {
                let name = state.current_file.clone();
                if state.save_reporting(&name).is_err() {
                    state.modal = Some(Modal::ConfirmSave);
                }
            }
            Some(Modal::SaveAs { name, exists }) => match sanitize_file_name(&name) {
                Ok(target) => {
                    if file_path(&target).exists() {
                        // Existing target: extra confirmation before overwrite.
                        state.modal = Some(Modal::ConfirmSaveAs(target));
                    } else {
                        let _ = state.save_reporting(&target);
                    }
                }
                Err(e) => {
                    // Invalid name: keep the form open and show the hint in it.
                    state.set_status(e, true);
                    state.modal = Some(Modal::SaveAs { name, exists });
                }
            },
            Some(Modal::ConfirmSaveAs(name)) => {
                if state.save_reporting(&name).is_err() {
                    state.modal = Some(Modal::ConfirmSaveAs(name));
                }
            }
            Some(Modal::LoadFile { selected, files }) => {
                if selected.is_empty() {
                    state.set_status("No config file found to load".to_string(), true);
                    state.modal = Some(Modal::LoadFile { selected, files });
                } else if state.load_reporting(&selected).is_err() {
                    state.modal = Some(Modal::LoadFile { selected, files });
                }
            }
            Some(Modal::AddSection { name }) => {
                let mut section = Section::default();
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    section.name = Some(trimmed.to_string());
                }
                // Appending a section shifts no existing sensor key; the
                // fresh section starts open with its default sensor open.
                state.config.sections.push(section);
                let new_index = state.config.sections.len() - 1;
                state.sections_collapsed.remove(&new_index);
                state.sensors_open.insert((new_index, 0));
                return operation::snap_to_end(content_scroll_id());
            }
            Some(Modal::AddSensor { section_index, kind, name }) => {
                if section_index < state.config.sections.len() {
                    let mut sensor = Sensor::default();
                    sensor.source_type = kind;
                    let trimmed = name.trim();
                    // No name = absent key: the bar renders the value alone.
                    sensor.name = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
                    let new_index = state.config.sections[section_index].sensors.len();
                    state.remap_sensor_keys(|s, i| {
                        Some((s, if s == section_index && i >= new_index { i + 1 } else { i }))
                    });
                    state.config.sections[section_index].sensors.push(sensor);
                    state.sensors_open.insert((section_index, new_index));
                    state.sections_collapsed.remove(&section_index);
                    let focus = operation::focus(sensor_name_id(section_index, new_index));
                    return if section_index + 1 == state.config.sections.len() {
                        // Appended to the last section: the row sits at the
                        // scrollable's bottom, so a plain snap lands on it.
                        Task::batch([focus, operation::snap_to_end(content_scroll_id())])
                    } else {
                        focus
                    };
                }
            }
            Some(Modal::ConfirmRemoveSection(index)) => {
                if state.config.sections.len() > 1 && index < state.config.sections.len() {
                    state.config.sections.remove(index);
                    state.sections_collapsed = state
                        .sections_collapsed
                        .drain()
                        .filter(|&s| s != index)
                        .map(|s| if s > index { s - 1 } else { s })
                        .collect();
                    state.remap_sensor_keys(|s, i| {
                        if s == index {
                            None
                        } else {
                            Some((if s > index { s - 1 } else { s }, i))
                        }
                    });
                }
            }
            Some(Modal::ConfirmRemoveSensor(section_index, sensor_index)) => {
                let in_bounds = section_index < state.config.sections.len()
                    && sensor_index < state.config.sections[section_index].sensors.len();
                if in_bounds {
                    state.config.sections[section_index].sensors.remove(sensor_index);
                    state.remap_sensor_keys(|s, i| {
                        if s == section_index && i == sensor_index {
                            None
                        } else if s == section_index && i > sensor_index {
                            Some((s, i - 1))
                        } else {
                            Some((s, i))
                        }
                    });
                }
            }
            Some(m @ Modal::BrowseIcons { .. }) => {
                // No confirm action in the browser: a picked cell already
                // applies, so Enter keeps the modal open as it is.
                state.modal = Some(m);
            }
            None => {}
        },
        Message::ModalCancel => state.modal = None,
        Message::ToggleDefaults => state.defaults_expanded = !state.defaults_expanded,
        Message::ToggleSecDefaults => state.sec_defaults_expanded = !state.sec_defaults_expanded,
        Message::ToggleSensorStyle(section, sensor) => {
            // Insert-or-remove keeps the set as the single source of truth.
            if !state.sensor_style_open.remove(&(section, sensor)) {
                state.sensor_style_open.insert((section, sensor));
            }
        }
        Message::ToggleSection(index) => {
            if !state.sections_collapsed.remove(&index) {
                state.sections_collapsed.insert(index);
            }
        }
        Message::ToggleSensor(section, sensor) => {
            if !state.sensors_open.remove(&(section, sensor)) {
                state.sensors_open.insert((section, sensor));
            }
        }
        Message::ResetConfig => {
            state.config = PiumaConfig::default();
            state.set_status("Configuration reset to defaults".to_string(), false);
        }
        Message::Status(_) => {}
    }
    Task::none()
}

/// The GUI theme:iced's palette fed from the bar's own config, so inputs,
/// pick lists, togglers and scrollbars follow the bar's colours instead of
/// a fixed dark variant.
fn theme(state: &PiumaSetup) -> Theme {
    let pal = palette(&state.config);
    Theme::custom(
        "Piuma",
        IcedPalette {
            background: pal.bg,
            text: pal.fg,
            primary: pal.accent,
            success: IcedPalette::DARK.success,
            warning: IcedPalette::DARK.warning,
            danger: Color::from_rgb(0.9, 0.3, 0.3),
        },
    )
}

// ---------------------------------------------------------------------------
// View helpers: cards, aligned form rows and collapsible blocks
// ---------------------------------------------------------------------------

/// Fixed label column width so every form row lines up and labels never
/// overlap their controls (`Length` only accepts u32).
const LABEL_WIDTH: u32 = 110;

/// Body text size: the iced default (16 pt) reduced by 2 pt as requested.
const BODY: u32 = 14;

/// Bar fallbacks mirrored from `src/main.rs` (DEFAULT_FONT / DEFAULT_*_COLOR),
/// alpha forced opaque: this is a window, not a translucent bar layer.
const SETUP_DEFAULT_FONT: &str = "Noto Sans";
const SETUP_DEFAULT_BG: Color = Color::from_rgba(0.06, 0.06, 0.08, 1.0);
const SETUP_DEFAULT_FG: Color = Color::from_rgb(0.88, 0.88, 0.94);

/// Visual tokens derived from the bar's own config: every surface, border
/// and text colour below comes from `piuma.json` through the same
/// `config::parse_color` the bar uses, so setup and bar can't disagree.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color,           // window background (`default_bg_color`)
    fg: Color,           // primary text (`default_text_color`)
    muted: Color,        // secondary text (fg blended toward bg)
    card: Color,         // card surface (`section_defaults.bg_color`)
    card_border: Color,  // card hairline (section border, kept visible)
    chrome: Color,       // rail/footer (bg blended toward card)
    inset: Color,        // child surface (card blended toward bg)
    accent: Color,       // primary/selected (`sensor_defaults.bg_color`)
    accent_text: Color,  // text on accent (hover text → normal text → fg)
    accent_hover: Color, // hover ring (`border_color_hover`)
    radius: f32,         // card corners (`section_defaults.border_radius`)
    pill: f32,           // button corners (`sensor_defaults.border_radius`)
}

/// Linear blend `a → b` by `t` (0 = a, 1 = b).
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// GTK windows (and this one) are opaque: surfaces drop their alpha, the
/// same way the N3 theme bridge drops it in its generated file.
fn opaque(c: Color) -> Color {
    Color { a: 1.0, ..c }
}

/// Resolve a config colour like the bar does (main.rs): `config::parse_color`,
/// else the mirrored bar default.
fn resolved(value: &Option<String>, fallback: Color) -> Color {
    opaque(
        value
            .as_deref()
            .and_then(|s| config::parse_color(s).ok())
            .unwrap_or(fallback),
    )
}

/// The whole token set from the loaded config (cheap: re-derived per call).
fn palette(cfg: &PiumaConfig) -> Palette {
    let bg = resolved(&cfg.default_bg_color, SETUP_DEFAULT_BG);
    let fg = resolved(&cfg.default_text_color, SETUP_DEFAULT_FG);
    let opaque_visible = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|s| config::parse_color(s).ok())
            .filter(|c| c.a > 0.0)
            .map(opaque)
    };
    // Surfaces: the configured colour when it draws something, otherwise a
    // token re-derived from bg/fg so the GUI keeps visible structure.
    let card = opaque_visible(&cfg.section_defaults.bg_color)
        .unwrap_or_else(|| mix(bg, Color::WHITE, 0.07));
    // A transparent section border draws nothing on the bar; the GUI still
    // needs a hairline, re-derived from the text colour.
    let card_border =
        opaque_visible(&cfg.section_defaults.border_color).unwrap_or_else(|| mix(fg, bg, 0.82));
    let accent = opaque_visible(&cfg.sensor_defaults.bg_color).unwrap_or_else(|| mix(fg, bg, 0.75));
    let accent_text = cfg
        .sensor_defaults
        .text_color_hover
        .as_deref()
        .or(cfg.sensor_defaults.text_color.as_deref())
        .and_then(|s| config::parse_color(s).ok())
        .map(opaque)
        .unwrap_or(fg);
    let accent_hover = opaque_visible(&cfg.sensor_defaults.border_color_hover)
        .unwrap_or_else(|| mix(accent, Color::WHITE, 0.25));
    let radius = cfg
        .section_defaults
        .border_radius
        .map(|r| (r.max(0) as f32).min(16.0))
        .unwrap_or(6.0);
    let pill = cfg
        .sensor_defaults
        .border_radius
        .map(|r| (r.max(0) as f32).min(16.0))
        .unwrap_or(6.0);
    Palette {
        bg,
        fg,
        muted: mix(fg, bg, 0.45),
        card,
        card_border,
        chrome: mix(bg, card, 0.4),
        inset: mix(card, bg, 0.5),
        accent,
        accent_text,
        accent_hover,
        radius,
        pill,
    }
}

/// Text style in the palette's primary colour (shared by the explicit
/// `.style` calls of the editor).
fn fg_text(pal: Palette) -> impl Fn(&Theme) -> TextStyle {
    move |_t: &Theme| TextStyle {
        color: Some(pal.fg),
        ..TextStyle::default()
    }
}

/// Text style in the palette's secondary colour.
fn muted_text(pal: Palette) -> impl Fn(&Theme) -> TextStyle {
    move |_t: &Theme| TextStyle {
        color: Some(pal.muted),
        ..TextStyle::default()
    }
}

/// Primary button skin: the bar's sensor language — accent pill, the hover
/// ring (`border_color_hover`) appearing only while hovered.
fn btn_primary(pal: Palette, t: &Theme, s: button::Status) -> button::Style {
    let mut st = button::primary(t, s); // the built-in disabled handling stays
    match s {
        button::Status::Disabled => {}
        button::Status::Hovered => {
            st.background = Some(Background::Color(mix(pal.accent, Color::WHITE, 0.12)));
            st.text_color = pal.accent_text;
            st.border = Border {
                width: 1.0,
                color: pal.accent_hover,
                radius: pal.pill.into(),
            };
        }
        _ => {
            st.background = Some(Background::Color(pal.accent));
            st.text_color = pal.accent_text;
            st.border = Border {
                width: 0.0,
                color: pal.accent_hover,
                radius: pal.pill.into(),
            };
        }
    }
    st
}

/// Secondary button skin: the card surface + the config hairline.
fn btn_secondary(pal: Palette, t: &Theme, s: button::Status) -> button::Style {
    let mut st = button::secondary(t, s);
    if !matches!(s, button::Status::Disabled) {
        let bg = match s {
            button::Status::Hovered => mix(pal.card, Color::WHITE, 0.06),
            _ => pal.card,
        };
        st.background = Some(Background::Color(bg));
        st.text_color = pal.fg;
        st.border = Border {
            width: 1.0,
            color: pal.card_border,
            radius: pal.pill.into(),
        };
    }
    st
}

/// Parses "#rgb"/"#rrggbb"/"#rrggbbaa" (alpha last, as piuma reads it) into a
/// preview color; invalid or empty = neutral gray.
fn hex_color(value: &str) -> Color {
    let v = value.trim().trim_start_matches('#');
    // Guard against multi-byte input: the slice indexes below are byte based.
    if !v.is_ascii() {
        return Color::from_rgb(0.35, 0.35, 0.35);
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    let rgba = match v.len() {
        8 => (byte(&v[0..2]), byte(&v[2..4]), byte(&v[4..6]), byte(&v[6..8])),
        6 => (byte(&v[0..2]), byte(&v[2..4]), byte(&v[4..6]), Some(255)),
        // Shorthand "#rgb": each digit is duplicated, alpha opaque.
        3 => (byte(&v[0..1]).map(|b| b * 17), byte(&v[1..2]).map(|b| b * 17), byte(&v[2..3]).map(|b| b * 17), Some(255)),
        _ => (None, None, None, None),
    };
    match rgba {
        (Some(r), Some(g), Some(b), Some(a)) => Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 },
        _ => Color::from_rgb(0.35, 0.35, 0.35),
    }
}

/// Shared card style: the section surface, its hairline and the radius
/// from `panel.section_defaults`.
fn card_style(pal: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_t: &Theme| container::Style {
        background: Some(Background::Color(pal.card)),
        border: Border {
            width: 1.0,
            color: pal.card_border,
            radius: pal.radius.into(),
        },
        ..container::Style::default()
    }
}

/// The visual unit of the nested layout (panel > section > sensor).
fn card<'a>(pal: Palette, content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content.into())
        .padding(12)
        .width(Length::Fill)
        .style(card_style(pal))
}

/// Inset background marking content as a child of the surrounding card.
fn inset_style(pal: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_t: &Theme| container::Style {
        background: Some(Background::Color(pal.inset)),
        border: Border {
            width: 1.0,
            color: pal.card_border,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

/// One label + control pair filling half of a form row.
fn form_cell<'a>(label: &'a str, control: Element<'a, Message>) -> Row<'a, Message> {
    Row::new()
        .push(text(label).size(BODY).width(LABEL_WIDTH))
        .push(control)
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill)
}

/// Text input using the standard body font size.
fn input<'a>(placeholder: &'a str, value: &'a str, on_input: impl Fn(String) -> Message + 'a) -> TextInput<'a, Message> {
    TextInput::new(placeholder, value).on_input(on_input).size(BODY)
}

/// Pick list using the standard body font size.
fn picklist<'a, T, L, V>(
    options: L,
    selected: Option<V>,
    on_select: impl Fn(T) -> Message + 'a,
) -> PickList<'a, T, L, V, Message>
where
    T: ToString + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
{
    PickList::new(options, selected, on_select).text_size(BODY)
}

/// Small arrow button reordering a list item; without `enabled` it carries no
/// action, which iced renders as disabled (used at the list boundaries).
fn move_btn<'a>(
    pal: Palette,
    label: &'a str,
    enabled: bool,
    on_press: Message,
) -> Button<'a, Message> {
    let b = button(text(label).size(BODY))
        .padding(5)
        .style(move |t, s| btn_secondary(pal, t, s));
    if enabled {
        b.on_press(on_press)
    } else {
        b
    }
}

// --- Sensor icons: WYSIWYG preview + theme-name browser ----------------

/// The icon theme and size exactly as the bar computes them (config
/// theme or the system one, font size clamped to u16): the preview must
/// agree with the bar, size variance included.
fn icon_theme_and_size(cfg: &PiumaConfig) -> (&str, u16) {
    let theme = cfg.icon_theme.as_deref().unwrap_or_else(|| icons::icon_theme());
    let size = cfg.font_size.min(u32::from(u16::MAX)) as u16;
    (theme, size)
}

/// The themed icon widget at the bar's size, or `None` when the name is
/// empty or the theme has no such icon — the one resolve path shared by
/// the preview box, the browser cells and the compact sensor rows.
fn resolved_icon<'a>(cfg: &PiumaConfig, name: Option<&str>) -> Option<Element<'a, Message>> {
    let (theme, size) = icon_theme_and_size(cfg);
    name.filter(|n| !n.is_empty())
        .and_then(|n| icons::resolve(theme, n, size))
        .map(|path| icons::widget(&path, size as f32))
}

/// Live preview beside the icon input: the resolved icon in a small
/// inset box, a red ✕ for an unknown name, the empty box while nothing
/// is typed — the name's fate is visible while typing it.
fn icon_preview<'a>(cfg: &PiumaConfig, name: Option<&str>) -> Element<'a, Message> {
    let pal = palette(cfg);
    let (_, size) = icon_theme_and_size(cfg);
    let inner: Element<'a, Message> = match name.filter(|n| !n.is_empty()) {
        Some(n) => match resolved_icon(cfg, Some(n)) {
            Some(icon) => icon,
            None => text("✕")
                .size(BODY)
                .style(|_t: &Theme| TextStyle {
                    color: Some(Color::from_rgb(1.0, 0.35, 0.35)),
                    ..TextStyle::default()
                })
                .into(),
        },
        None => text("").into(),
    };
    let box_size = f32::from(size) + 12.0;
    container(inner)
        .width(Length::Fixed(box_size))
        .height(Length::Fixed(box_size))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(inset_style(pal))
        .into()
}

/// Sentinel label of the "no configured theme" picklist entry: the bar
/// then falls back to the system icon-theme setting.
const SYSTEM_ICON_THEME: &str = "system default";

/// Icon-theme picklist options: sentinel first, then the installed
/// themes sorted — by directory name, the lookup's own keys, so every
/// option resolves (the index's display Name= would not); a hand-typed
/// name missing from the list joins it, so the current value never
/// falls out of the options.
fn icon_theme_options(current: Option<&str>) -> Vec<String> {
    let mut themes = installed_theme_names();
    if let Some(c) = current.filter(|c| !c.is_empty() && !themes.iter().any(|t| t == c)) {
        themes.insert(0, c.to_string());
    }
    themes.insert(0, SYSTEM_ICON_THEME.to_string());
    themes
}

/// The panel's icon-theme picker; the empty config maps to the sentinel
/// and back (empty = the bar's system fallback).
fn icon_theme_picker(state: &PiumaSetup) -> Element<'_, Message> {
    let current = state.config.icon_theme.clone().unwrap_or_default();
    let selected = if current.is_empty() {
        SYSTEM_ICON_THEME.to_string()
    } else {
        current
    };
    picklist(
        icon_theme_options(state.config.icon_theme.as_deref()),
        Some(selected),
        |value: String| {
            Message::IconThemeChanged(if value == SYSTEM_ICON_THEME { String::new() } else { value })
        },
    )
    .width(Length::Fill)
    .into()
}

/// The XDG data dirs of the lookup: the user's data home, then
/// $XDG_DATA_DIRS (defaulting to /usr/local/share and /usr/share).
fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut dirs_: Vec<PathBuf> = dirs::data_dir().into_iter().collect();
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    dirs_.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    dirs_
}

/// Theme roots: the XDG data dirs' `icons` plus the legacy ~/.icons.
fn icon_theme_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = xdg_data_dirs().iter().map(|d| d.join("icons")).collect();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".icons"));
    }
    roots
}

/// Flat pixmaps dirs of the same data dirs: the lookup's last resort,
/// where distro logos live outside every theme (/usr/share/pixmaps).
fn icon_pixmap_dirs() -> Vec<PathBuf> {
    xdg_data_dirs().into_iter().map(|d| d.join("pixmaps")).collect()
}

/// Whether a file is an icon image (the extensions the lookup builds).
fn is_icon_image(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_ascii_lowercase().as_str(),
            "svg" | "svgz" | "png" | "xpm"
        )
    })
}

/// The `Inherits=` parents of a theme's index.theme (unlocalized key):
/// the fallback chain the lookup walks one level deep.
fn index_theme_inherits(dir: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(dir.join("index.theme")) else {
        return Vec::new();
    };
    text.lines()
        .find_map(|line| line.strip_prefix("Inherits=").map(str::to_string))
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Dirs of every root hosting a child with this exact theme name: the
/// lookup keys themes by directory name, so the match is exact ("Breeze"
/// with only `breeze` installed misses, exactly like the bar misses).
fn theme_dirs_by_key(theme: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in icon_theme_roots() {
        let Ok(entries) = fs::read_dir(&root) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let named = path.file_name().is_some_and(|n| n == std::ffi::OsStr::new(theme));
            if named && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// Sorted theme names of one icons root: a directory is a theme when it
/// carries an index.theme, and the name the lookup keys by is the
/// directory name itself (never the index's display `Name=`).
fn theme_names_in_root(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("index.theme").exists())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    names.sort();
    names
}

/// Every installed theme name across the roots (sorted, deduped): the
/// picklist offers lookup keys, so a picked option always resolves.
fn installed_theme_names() -> Vec<String> {
    let mut names: Vec<String> =
        icon_theme_roots().iter().flat_map(|root| theme_names_in_root(root)).collect();
    names.sort();
    names.dedup();
    names
}

/// Recursively collects the icon stems (image files) of a theme dir:
/// the theme module of freedesktop-icons is private, so the browser
/// does its own one-shot directory walk.
fn collect_icon_names(dir: &Path, out: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_icon_names(&path, out);
        } else if let (true, Some(stem)) = (is_icon_image(&path), path.file_stem().and_then(|s| s.to_str())) {
            out.insert(stem.to_string());
        }
    }
}

/// Image stems directly inside a flat dir (no recursion: the base-path
/// fallback of the lookup only checks files at its top level).
fn collect_flat_icon_names(dir: &Path, out: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let (true, Some(stem)) = (is_icon_image(&path), path.file_stem().and_then(|s| s.to_str())) {
            out.insert(stem.to_string());
        }
    }
}

/// Icon names the bar can resolve under `theme`, mirroring the lookup
/// chain of freedesktop-icons: the theme's own dirs (keyed by dir name,
/// an unknown name starts from hicolor), the `Inherits` parents, then
/// the flat base/pixmaps dirs. Sorted, deduped, cached per theme name.
fn theme_icon_names(theme: &str) -> Vec<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(names) = guard.get(theme) {
        return names.clone();
    }
    let mut set = HashSet::new();
    // 1) the theme's dirs; an unknown theme name starts from hicolor.
    let mut dirs = theme_dirs_by_key(theme);
    if dirs.is_empty() {
        dirs = theme_dirs_by_key("hicolor");
    }
    let mut parents: Vec<String> = Vec::new();
    for dir in &dirs {
        collect_icon_names(dir, &mut set);
        parents.extend(index_theme_inherits(dir));
    }
    // 2) the inherited parents, one level like the lookup walks them.
    let mut seen = HashSet::new();
    for parent in parents {
        if !seen.insert(parent.to_ascii_lowercase()) {
            continue;
        }
        for dir in theme_dirs_by_key(&parent) {
            collect_icon_names(&dir, &mut set);
        }
    }
    // 3) the flat base dirs (icons roots + pixmaps): the last resort,
    // where distro logos live outside every theme.
    for dir in icon_theme_roots().into_iter().chain(icon_pixmap_dirs()) {
        collect_flat_icon_names(&dir, &mut set);
    }
    let mut names: Vec<String> = set.into_iter().collect();
    names.sort();
    guard.insert(theme.to_string(), names.clone());
    names
}

/// Grid bounds of the icon browser: a broad query still renders a
/// bounded widget count (names shown = cap at most).
const ICON_GRID_CAP: usize = 120;
const ICONS_PER_ROW: usize = 4;

/// Case-insensitive substring filter over the theme names, sorted and
/// capped for the browser grid.
fn filter_icon_names<'a>(names: &'a [String], query: &str) -> Vec<&'a str> {
    let q = query.trim().to_ascii_lowercase();
    let mut hits: Vec<&'a str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| q.is_empty() || n.to_ascii_lowercase().contains(&q))
        .collect();
    hits.sort_unstable();
    hits.truncate(ICON_GRID_CAP);
    hits
}

/// Number of names the query matches, uncapped (the grid's caption).
fn count_icon_matches(names: &[String], query: &str) -> usize {
    let q = query.trim().to_ascii_lowercase();
    names
        .iter()
        .filter(|n| q.is_empty() || n.to_ascii_lowercase().contains(&q))
        .count()
}

/// One browser cell: the icon at the bar's size over its name; pressing
/// writes the name into the sensor's field and closes the modal.
fn icon_grid_cell(
    pal: Palette,
    cfg: &PiumaConfig,
    name: &str,
    section_index: usize,
    sensor_index: usize,
) -> Element<'static, Message> {
    // The name is copied into the widgets, so the cell keeps no borrow:
    // it can outlive the filtered list that produced it.
    let preview: Element<'static, Message> =
        resolved_icon(cfg, Some(name)).unwrap_or_else(|| text("✕").size(BODY).into());
    button(
        column![preview, text(name.to_string()).size(9)]
            .spacing(4)
            .align_x(Alignment::Center)
            .width(Length::Fill),
    )
    .on_press(Message::IconPicked(section_index, sensor_index, name.to_string()))
    .padding(6)
    .width(Length::Fixed(96.0))
    .style(move |t, s| btn_secondary(pal, t, s))
    .into()
}

/// Scrollable holding the tab content: the add-section/add-sensor tasks
/// share its id to snap the freshly created row into view.
fn content_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("content")
}

/// Text-input id of one sensor's name field, focused after add/duplicate.
fn sensor_name_id(section: usize, sensor: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("sensor-name-{section}-{sensor}"))
}

/// Two-column form grid: fields alternate left/right, one per cell, so long
/// forms stay compact and their labels line up in fixed columns.
fn form_grid<'a>(fields: Vec<(&'a str, Element<'a, Message>)>) -> Element<'a, Message> {
    let mut rows = Column::new().spacing(8).width(Length::Fill);
    let mut iter = fields.into_iter();
    while let Some(first) = iter.next() {
        let mut row = Row::new().spacing(20).width(Length::Fill)
            .push(form_cell(first.0, first.1));
        if let Some(second) = iter.next() {
            row = row.push(form_cell(second.0, second.1));
        }
        rows = rows.push(row);
    }
    rows.into()
}

/// Color input filling its cell: text field plus a live 16x16 preview swatch.
fn color_control<'a>(value: &'a str, on_input: impl Fn(String) -> Message + 'a) -> Element<'a, Message> {
    let swatch_color = hex_color(value);
    let swatch = container(text(""))
        .width(16)
        .height(16)
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(swatch_color)),
            border: Border { width: 1.0, color: Color::from_rgb(0.45, 0.45, 0.45), radius: 3.0.into() },
            ..container::Style::default()
        });
    Row::new()
        .push(input("#rrggbb", value, on_input).width(Length::Fill))
        .push(swatch)
        .spacing(6)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// Slider control filling its cell: slider takes the space, value on the
/// right (style numbers: -1 = disabled, 0..=N = enabled).
fn slider_control<'a>(range: RangeInclusive<i32>, value: i32, on_input: impl Fn(i32) -> Message + 'a) -> Element<'a, Message> {
    Row::new()
        .push(Slider::new(range, value, on_input).width(Length::Fill))
        .push(text(value.to_string()).size(BODY).width(40))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// Collapsible header: chevron + title + summary; pressing toggles the block.
/// The title is owned so section/sensor rows can pass their dynamic names.
fn collapsible_header<'a>(
    pal: Palette,
    title: String,
    summary: String,
    expanded: bool,
    on_toggle: Message,
) -> Element<'a, Message> {
    button(
        Row::new()
            .push(text(if expanded { "▾" } else { "▸" }).size(BODY).width(14))
            .push(text(title).size(13))
            .push(text(summary).size(10).style(muted_text(pal)))
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(6)
    .style(move |t, s| btn_secondary(pal, t, s))
    .on_press(on_toggle)
    .into()
}

/// One-line summary of the active style overrides for collapsed headers:
/// colors by field name, numbers with value, "none" when all is inherited.
fn style_summary(colors: &[(&str, &Option<String>)], numbers: &[(&str, &Option<i32>)]) -> String {
    let mut parts = Vec::new();
    let set_colors: Vec<&str> = colors.iter().filter(|(_, v)| v.is_some()).map(|(k, _)| *k).collect();
    if !set_colors.is_empty() {
        parts.push(set_colors.join(" "));
    }
    let set_numbers: Vec<String> = numbers.iter().filter_map(|(k, v)| v.map(|n| format!("{} {}", k, n))).collect();
    if !set_numbers.is_empty() {
        parts.push(set_numbers.join(" "));
    }
    if parts.is_empty() { "none".to_string() } else { parts.join(" · ") }
}

/// Root view: the scrollable column of nested cards with the action bar
/// underneath; a modal, when open, is stacked over everything on a dimmed layer.
fn view(state: &PiumaSetup) -> Element<'_, Message> {
    // Roadmap N4: a left tab rail picks the category and only that
    // category's card renders — the header (file info) and the footer
    // (Load/Save/status) stay visible on every tab.
    let pal = palette(&state.config);
    let content = column![config_header(state)]
        .push(active_card(state))
        .padding(20)
        .spacing(15)
        .width(Length::Fill);

    let base = container(
        column![
            Row::new()
                .push(sidebar(state))
                .push(
                    container(scrollable(content).id(content_scroll_id()).width(Length::Fill).height(Length::Fill))
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill),
            footer(state),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_t: &Theme| container::Style {
        background: Some(Background::Color(pal.bg)),
        ..container::Style::default()
    });

    // One stable root type: swapping Stack↔Container when a modal opens or
    // closes makes Tree::diff replace the whole widget tree (tag mismatch),
    // dropping the content scrollable's state — the page jumped back to the
    // top after every modal, e.g. after picking an icon. Keeping `base` at
    // stack index 0 in both states preserves the scroll position.
    let mut root = stack![base];
    if state.modal.is_some() {
        root = root.push(modal_overlay(state));
    }
    root.into()
}

/// The one card belonging to the active tab (roadmap N4).
fn active_card(state: &PiumaSetup) -> Element<'_, Message> {
    match state.active_tab {
        Tab::Panel => panel_settings(state),
        Tab::Notifications => notifications_settings(state),
        Tab::SensorDefaults => sensor_defaults_view(state),
        Tab::SectionDefaults => section_defaults_view(state),
        Tab::Sections => sections_list(state),
        Tab::Wallpaper => wallpaper_settings(state),
        Tab::AutostartApps => autostart_apps_settings(state),
    }
}

/// Left tab rail (roadmap N4): one button per category, the active one
/// highlighted; edits live in `state.config`, so switching tabs never
/// loses unsaved work.
fn sidebar(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut rail = Column::new().spacing(4).padding(Padding {
        top: 20.0,
        right: 10.0,
        bottom: 10.0,
        left: 10.0,
    });
    for tab in Tab::ALL {
        let selected = state.active_tab == tab;
        rail = rail.push(
            button(text(tab.label()).size(BODY))
                .on_press(Message::TabSelected(tab))
                .padding(8)
                .width(Length::Fill)
                .style(move |t, s| {
                    if selected {
                        btn_primary(pal, t, s)
                    } else {
                        btn_secondary(pal, t, s)
                    }
                }),
        );
    }
    container(rail)
        .width(Length::Fixed(170.0))
        .height(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(pal.chrome)),
            border: Border {
                width: 1.0,
                color: pal.card_border,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Fixed bottom bar: edited file + status on the left, actions on the right.
fn footer(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let info = column![
        text(format!("editing: {}", state.current_file))
            .size(BODY)
            .style(muted_text(pal)),
        status_bar(state),
    ]
    .spacing(2);

    let bar = Row::new()
        .push(container(info).width(Length::Fill))
        .push(action_buttons(pal))
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    container(bar)
        .padding(Padding { top: 10.0, right: 20.0, bottom: 10.0, left: 20.0 })
        .width(Length::Fill)
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(pal.chrome)),
            border: Border {
                width: 1.0,
                color: pal.card_border,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Dimmed full-window layer with the active modal card centered on top.
fn modal_overlay(state: &PiumaSetup) -> Element<'_, Message> {
    container(modal_card(state))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 })),
            ..container::Style::default()
        })
        .into()
}

/// The card of the currently open modal (an empty element when none).
fn modal_card(state: &PiumaSetup) -> Element<'_, Message> {
    // Errors are rendered inside the modal so they are visible over the overlay.
    let error = if state.status_is_error {
        state.status_message.clone()
    } else {
        None
    };
    let pal = palette(&state.config);

    match &state.modal {
        Some(Modal::ConfirmSave) => dialog_frame(
            pal,
            "Unsaved changes",
            text(format!("Overwrite {} with the current configuration?", state.current_file)).size(BODY).into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Overwrite"),
        ),
        Some(Modal::SaveAs { name, exists }) => {
            let mut body = column![
                input("my-styles.json", name, Message::ModalTextChanged)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .width(Length::Fill);
            if *exists {
                body = body.push(
                    text("File exists: it will be overwritten").size(10)
                        .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(1.0, 0.6, 0.2)), ..TextStyle::default() }),
                );
            }
            dialog_frame(
                pal,
                "Save as",
                body.into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Save"),
            )
        }
        Some(Modal::ConfirmSaveAs(file)) => dialog_frame(
            pal,
            "File exists",
            text(format!("The file {} already exists. Overwrite it?", file)).size(BODY).into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Overwrite"),
        ),
        Some(Modal::LoadFile { selected, files }) => {
            // Files as buttons (not a pick list): every choice stays visible
            // and no dropdown overlay can swallow clicks around the dialog.
            let mut list = column![].spacing(6).width(Length::Fill);
            for f in files {
                let is_selected = *selected == *f;
                list = list.push(
                    button(text(f.clone()).size(BODY))
                        .on_press(Message::ModalTextChanged(f.clone()))
                        .padding(8)
                        .width(Length::Fill)
                        .style(move |t, s| {
                            if is_selected {
                                btn_primary(pal, t, s)
                            } else {
                                btn_secondary(pal, t, s)
                            }
                        }),
                );
            }
            let body: Element<Message> = if files.is_empty() {
                text("No .json files found in the config directory").size(BODY).into()
            } else {
                list.into()
            };
            dialog_frame(
                pal,
                "Load configuration",
                body,
                error,
                dialog_actions(pal, Message::ModalConfirm, "Load"),
            )
        }
        Some(Modal::AddSection { name }) => dialog_frame(
            pal,
            "New section",
            input("Name (optional)", name, Message::ModalTextChanged)
                .width(Length::Fill)
                .into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Add"),
        ),
        Some(Modal::AddSensor { kind, name, .. }) => {
            // Type combo of the canonical list, plus an optional name; the
            // muted line under the fields explains what the type does.
            let body = column![
                form_cell("Type", picklist(
                    available_source_types(kind),
                    Some(kind.clone()),
                    Message::ModalTypeChanged,
                ).width(Length::Fill).into()),
                form_cell("Name", input("optional", name, Message::ModalTextChanged)
                    .width(Length::Fill)
                    .into()),
                text(source_label(canonical_type(kind))).size(10).style(muted_text(pal)),
            ]
            .spacing(8)
            .width(Length::Fill);
            dialog_frame(
                pal,
                "New sensor",
                body.into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Add"),
            )
        }
        Some(Modal::ConfirmRemoveSection(index)) => {
            let section = &state.config.sections[*index];
            let name = section.name.as_deref().unwrap_or("(unnamed)");
            let count = section.sensors.len();
            dialog_frame(
                pal,
                "Remove section",
                text(format!(
                    "Remove \"{name}\" and its {count} sensor(s)? Changes apply on Save."
                ))
                .size(BODY)
                .into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Remove"),
            )
        }
        Some(Modal::ConfirmRemoveSensor(section_index, sensor_index)) => {
            let sensor = &state.config.sections[*section_index].sensors[*sensor_index];
            let name = sensor
                .name
                .clone()
                .unwrap_or_else(|| source_label(canonical_type(&sensor.source_type)).to_string());
            dialog_frame(
                pal,
                "Remove sensor",
                text(format!("Remove \"{name}\"? Changes apply on Save."))
                    .size(BODY)
                    .into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Remove"),
            )
        }
        Some(Modal::BrowseIcons { section_index, sensor_index, query }) => {
            // Theme names as a grid of live previews: type to narrow the
            // list, press a cell to pick; the directory walk is cached
            // per theme, so only filtering happens per keystroke.
            let theme = state
                .config
                .icon_theme
                .clone()
                .unwrap_or_else(|| icons::icon_theme().to_string());
            let names = theme_icon_names(&theme);
            let hits = filter_icon_names(&names, query);
            let mut grid = column![].spacing(8).width(Length::Fill);
            let mut cells = Row::new().spacing(8);
            for (i, name) in hits.iter().enumerate() {
                cells = cells.push(icon_grid_cell(
                    pal,
                    &state.config,
                    name,
                    *section_index,
                    *sensor_index,
                ));
                if (i + 1) % ICONS_PER_ROW == 0 {
                    grid = grid.push(cells);
                    cells = Row::new().spacing(8);
                }
            }
            if hits.len() % ICONS_PER_ROW != 0 {
                grid = grid.push(cells);
            }
            let results: Element<Message> = if hits.is_empty() {
                text("no match").size(BODY).into()
            } else {
                scrollable(grid.width(Length::Fill))
                    .height(Length::Fixed(320.0))
                    .width(Length::Fill)
                    .into()
            };
            let body = column![
                input("search (substring)", query, Message::ModalTextChanged).width(Length::Fill),
                text(format!(
                    "{} icons available · {} match",
                    names.len(),
                    count_icon_matches(&names, query)
                ))
                .size(10)
                .style(muted_text(pal)),
                results,
            ]
            .spacing(8)
            .width(Length::Fill);
            dialog_frame(
                pal,
                "Pick icon",
                body.into(),
                error,
                // A single close button: a picked cell already applies.
                Row::new().push(
                    button(text("Close").size(BODY))
                        .on_press(Message::ModalCancel)
                        .padding(8)
                        .style(move |t, s| btn_primary(pal, t, s)),
                ),
            )
        }
        None => text("").into(),
    }
}

/// Modal card frame: title, body, optional error hint, action buttons.
fn dialog_frame<'a>(
    pal: Palette,
    title: &'a str,
    body: Element<'a, Message>,
    error: Option<String>,
    actions: Row<'a, Message>,
) -> Element<'a, Message> {
    let mut col = column![text(title).size(16)]
        .spacing(10)
        .width(Length::Fixed(420.0));
    col = col.push(body);
    if let Some(e) = error {
        col = col.push(text(e).size(10)
            .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.9, 0.3, 0.3)), ..TextStyle::default() }));
    }
    col = col.push(actions);

    container(col)
        .padding(16)
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(pal.card)),
            border: Border {
                width: 1.0,
                color: pal.card_border,
                radius: pal.radius.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Cancel + primary action button row shared by all modals.
fn dialog_actions<'a>(pal: Palette, action: Message, label: &'a str) -> Row<'a, Message> {
    Row::new()
        .push(
            button(text("Cancel").size(BODY))
                .on_press(Message::ModalCancel)
                .padding(8)
                .style(move |t, s| btn_secondary(pal, t, s)),
        )
        .push(
            button(text(label).size(BODY))
                .on_press(action)
                .padding(8)
                .style(move |t, s| btn_primary(pal, t, s)),
        )
        .spacing(10)
        .align_y(Alignment::Center)
}

fn config_header(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let title = text("Piuma Configuration Setup")
        .size(22)
        .style(fg_text(pal));
    let subtitle = text("Configure your Piuma status bar")
        .size(14)
        .style(muted_text(pal));

    column![text(""), title, subtitle, text("")]
        .spacing(10)
        .align_x(Alignment::Center)
        .into()
}

/// Freedesktop wallpaper folders (base-dir + user-dirs specs): the
/// per-user data dir, every system data dir (`…/wallpapers`) and the
/// user's pictures directory. Missing folders are skipped by the scan.
fn wallpaper_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(base) = data_home {
        dirs.push(base.join("wallpapers"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(
            data_dirs
                .split(':')
                .filter(|d| !d.is_empty())
                .map(|d| PathBuf::from(d).join("wallpapers")),
        );
    }
    if let Some(pictures) = user_pictures_dir() {
        dirs.push(pictures);
    }
    dirs
}

/// Pictures folder from `~/.config/user-dirs.dirs`: the value is quoted
/// shell-ish text with `$HOME` placeholders, expanded here.
fn user_pictures_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let content = fs::read_to_string(home.join(".config/user-dirs.dirs")).ok()?;
    let raw = parse_user_dirs_pictures(&content)?;
    Some(PathBuf::from(raw.replace("$HOME", home.to_str()?)))
}

/// `XDG_PICTURES_DIR` value of `user-dirs.dirs` content; comments and
/// other keys are ignored, an empty value means "no pictures dir".
fn parse_user_dirs_pictures(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("XDG_PICTURES_DIR=")
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .filter(|v| !v.is_empty())
}

/// Every image under `dirs`, sorted and deduplicated: common raster
/// formats only (what the bar's image widget decodes).
fn scan_wallpaper_dirs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        collect_images(dir, &mut found, 6);
    }
    found.sort();
    found.dedup();
    found
}

/// Recursive worker of `scan_wallpaper_dirs`: unreadable entries are
/// skipped and `depth` caps the walk, so a symlink cycle can never hang
/// the setup window.
fn collect_images(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, out, depth - 1);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push(path);
        }
    }
}

/// Applies one wallpaper edit and collapses a block the user backed out
/// of (disabled + empty path) back to `None`, so a save never injects
/// the key unless the section was really used.
fn wallpaper_set(config: &mut PiumaConfig, edit: impl FnOnce(&mut WallpaperCfg)) {
    let w = config.wallpaper.get_or_insert_with(WallpaperCfg::default);
    edit(w);
    if !w.enabled && w.path.is_empty() {
        config.wallpaper = None;
    }
}

/// The `panel.wallpaper` editor (roadmap2 #1): the bar draws the image
/// itself, so this tab only chooses it — enable toggle, the images found
/// in the freedesktop folders (list + preview) and a hand-typed path.
/// Saving is the global Save; the bar applies the change on its tick.
fn wallpaper_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let cfg = state.config.wallpaper.clone().unwrap_or_default();
    let current = cfg.path.clone();
    let pal = palette(&state.config);

    // The scanned images as a pickable list; the current path is marked.
    let mut files = column![].spacing(2).width(Length::Fill);
    for file in &state.wallpaper_files {
        let label = file.to_string_lossy().to_string();
        let selected = current == label;
        files = files.push(
            button(
                text(if selected {
                    format!("• {label}")
                } else {
                    label.clone()
                })
                .size(BODY),
            )
            .on_press(Message::WallpaperPathChanged(label))
            .style(move |t, s| {
                if selected {
                    btn_primary(pal, t, s)
                } else {
                    btn_secondary(pal, t, s)
                }
            })
            .width(Length::Fill),
        );
    }
    let list = scrollable(files).height(Length::Fixed(240.0)).width(Length::Fill);

    // Preview of the current path: the image when the file is there, a
    // placeholder otherwise (a typed path may not exist yet).
    let preview: Element<Message> = if !current.is_empty() && PathBuf::from(&current).is_file() {
        iced::widget::image(iced::widget::image::Handle::from_path(current.clone()))
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(iced::ContentFit::Contain)
            .into()
    } else {
        text("No preview — pick an image or check the path").size(BODY).into()
    };

    card(
        pal,
        column![
            text("Wallpaper").size(16).style(fg_text(pal)),
            toggler(cfg.enabled)
                .label("Draw the wallpaper behind the desktop")
                .on_toggle(Message::WallpaperEnabled),
            Row::new()
                .push(list)
                .push(
                    container(preview)
                        .width(Length::Fixed(360.0))
                        .height(Length::Fixed(240.0))
                        .style(move |_t: &Theme| container::Style {
                            border: Border {
                                color: pal.card_border,
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            ..container::Style::default()
                        }),
                )
                .spacing(10)
                .width(Length::Fill),
            input(
                "/path/to/image.png",
                state.config.wallpaper.as_ref().map(|w| w.path.as_str()).unwrap_or_default(),
                Message::WallpaperPathChanged,
            )
            .width(Length::Fill),
            text("Searched: $XDG_DATA_HOME/wallpapers, $XDG_DATA_DIRS/wallpapers and your pictures folder (freedesktop).")
                .size(BODY),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// The dedicated "Autostart apps" tab (roadmap2 #2): the global launch
/// switch, plus one row per freedesktop autostart `.desktop` with its
/// own switch — the selection lives in `panel.autostart_disabled` (the
/// Save persists it, both the bar and every save prune stale names)
/// and no autostart file is ever written, so other desktop environments
/// keep their own, unaffected view. The bar applies the selection at
/// its next start.
fn autostart_apps_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut apps = column![].spacing(4).width(Length::Fill);
    if state.autostart_apps.is_empty() {
        apps = apps.push(text("No autostart .desktop files found.").size(BODY));
    }
    for app in &state.autostart_apps {
        // A spec-off app (its own Hidden/GNOME switches keep it out) has
        // no piuma-side choice to make: its switch is inert.
        let toggle = if app.spec {
            toggler(app.enabled)
                .on_toggle(move |value| Message::AutostartAppToggled(app.file.clone(), value))
        } else {
            toggler(app.enabled)
        };
        apps = apps.push(
            Row::new()
                .push(text(&app.label).size(BODY).width(Length::Fill))
                .push(toggle)
                .align_y(Alignment::Center)
                .spacing(8)
                .width(Length::Fill),
        );
    }

    card(
        pal,
        column![
            text("Autostart apps").size(16).style(fg_text(pal)),
            toggler(state.config.autostart.unwrap_or(false))
                .label("Launch freedesktop autostart apps")
                .on_toggle(Message::AutostartChanged),
            apps,
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// Panel-wide settings as a two-column form grid inside a card.
fn panel_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let font_size = state.config.font_size;
    let thickness = state.config.thickness;
    let anchor = state.config.anchor.clone().unwrap_or_default();
    let align = state.config.align.clone().unwrap_or_default();

    // Font Size / Thickness keep their u32 sliders inline: a shared generic
    // slider helper would need `num_traits` bounds, which this binary doesn't depend on.
    let fields = vec![
        ("Font", input("Font name", state.config.font.as_deref().unwrap_or(""), Message::FontChanged)
            .width(Length::Fill)
            .into()),
        ("Font Size", Row::new()
            .push(Slider::new(8..=32, font_size, Message::FontSizeChanged).width(Length::Fill))
            .push(text(format!("{}pt", font_size)).size(BODY).width(40))
            .spacing(8)
            .width(Length::Fill)
            .into()),
        ("Bold", toggler(state.config.bold.unwrap_or(false))
            .label("Bold text")
            .on_toggle(Message::BoldChanged)
            .into()),
        ("Icon Theme", icon_theme_picker(state).into()),
        // Autostart lives in its own tab now (roadmap2 #2).
        ("App Theme", toggler(state.config.app_theme.unwrap_or(false))
            .label("Generate GTK theme bridge")
            .on_toggle(Message::AppThemeChanged)
            .into()),
        ("Thickness", Row::new()
            .push(Slider::new(8..=64, thickness, Message::ThicknessChanged).width(Length::Fill))
            .push(text(format!("{}px", thickness)).size(BODY).width(40))
            .spacing(8)
            .width(Length::Fill)
            .into()),
        ("Section Layout", picklist(
            vec!["horizontal".to_string(), "vertical".to_string()],
            Some(state.config.section_layout.clone()),
            Message::SectionLayoutChanged,
        ).width(Length::Fill).into()),
        ("Default BG", color_control(state.config.default_bg_color.as_deref().unwrap_or(""), Message::DefaultBgColorChanged)),
        ("Default Text", color_control(state.config.default_text_color.as_deref().unwrap_or(""), Message::DefaultTextColorChanged)),
        ("Anchor", picklist(
            vec!["top".to_string(), "bottom".to_string(), "left".to_string(), "right".to_string(),
                 "top-left".to_string(), "top-right".to_string(), "bottom-left".to_string(), "bottom-right".to_string()],
            Some(anchor),
            Message::AnchorChanged,
        ).width(Length::Fill).into()),
        ("Align", picklist(
            vec!["left".to_string(), "center".to_string(), "right".to_string(), "fill".to_string()],
            Some(align),
            Message::AlignChanged,
        ).width(Length::Fill).into()),
    ];

    card(
        pal,
        column![
            text("Panel Settings").size(16).style(fg_text(pal)),
            form_grid(fields),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// The `panel.notifications` editor: the enable toggle is the block's
/// presence (it turns piuma's D-Bus server on/off), the three fields
/// are the block's settings; they only exist while the block does.
fn notifications_settings<'a>(state: &'a PiumaSetup) -> Element<'a, Message> {
    let pal = palette(&state.config);
    let enabled = state.config.notifications.is_some();
    let mut body = column![
        toggler(enabled)
            .label("Enable desktop notifications")
            .on_toggle(Message::NotificationsEnabled)
    ]
    .spacing(10);

    if let Some(n) = &state.config.notifications {
        let (timeout, history, replace) =
            (n.default_timeout_ms, n.history_size, n.replace_existing);
        body = body.push(form_grid(vec![
            ("Default Timeout", Row::new()
                .push(Slider::new(500..=60_000u32, timeout, Message::NotificationsTimeoutChanged).width(Length::Fill))
                .push(text(format!("{}ms", timeout)).size(BODY).width(64))
                .spacing(8)
                .width(Length::Fill)
                .into()),
            ("History Size", Row::new()
                .push(Slider::new(1..=100u32, history, Message::NotificationsHistoryChanged).width(Length::Fill))
                .push(text(format!("{}", history)).size(BODY).width(64))
                .spacing(8)
                .width(Length::Fill)
                .into()),
            ("App Updates", toggler(replace)
                .label("Update the existing card (no duplicates)")
                .on_toggle(Message::NotificationsReplaceChanged)
                .into()),
        ]));

        // Popup placement: where the cards appear on screen (roadmap M4) —
        // position picklist plus the two screen-edge margins.
        body = body
            .push(
                text("Popup placement — where the cards appear on screen")
                    .size(10)
                    .style(muted_text(pal)),
            )
            .push(form_grid(vec![
                ("Position", picklist(
                    vec![
                        "auto".to_string(),
                        "top-left".to_string(),
                        "top-center".to_string(),
                        "top-right".to_string(),
                        "bottom-left".to_string(),
                        "bottom-center".to_string(),
                        "bottom-right".to_string(),
                    ],
                    Some(n.popup_position.clone()),
                    |value| Message::NotifPositionChanged(value),
                ).width(Length::Fill).into()),
                ("Margin H", slider_control(0..=100, n.popup_margin_h, |value| Message::NotifNumberChanged("popup_margin_h".to_string(), value))),
                ("Margin V", slider_control(0..=100, n.popup_margin_v, |value| Message::NotifNumberChanged("popup_margin_v".to_string(), value))),
            ]));

        // Popup style: the same controls as Sensor Defaults (colors, border,
        // padding) plus the font size; the defaults are white text on black
        // with a grey border and every key is written out explicitly, like
        // the server keys above.
        body = body
            .push(
                text("Popup style — how the notification cards look")
                    .size(10)
                    .style(muted_text(pal)),
            )
            .push(form_grid(vec![
                ("Font Size", slider_control(8..=32, n.font_size as i32, |value| Message::NotifNumberChanged("font_size".to_string(), value))),
                ("Text", color_control(&n.text_color, |value| Message::NotifStyleChanged("text_color".to_string(), value))),
                ("Hover Text", color_control(&n.text_color_hover, |value| Message::NotifStyleChanged("text_color_hover".to_string(), value))),
                ("BG", color_control(&n.bg_color, |value| Message::NotifStyleChanged("bg_color".to_string(), value))),
                ("Hover BG", color_control(&n.bg_color_hover, |value| Message::NotifStyleChanged("bg_color_hover".to_string(), value))),
                ("Border", color_control(&n.border_color, |value| Message::NotifStyleChanged("border_color".to_string(), value))),
                ("Border Hover", color_control(&n.border_color_hover, |value| Message::NotifStyleChanged("border_color_hover".to_string(), value))),
                ("Border Width", slider_control(0..=10, n.border_width, |value| Message::NotifNumberChanged("border_width".to_string(), value))),
                ("Border Radius", slider_control(0..=20, n.border_radius, |value| Message::NotifNumberChanged("border_radius".to_string(), value))),
                ("Padding", slider_control(0..=24, n.padding, |value| Message::NotifNumberChanged("padding".to_string(), value))),
                ("Border Type", picklist(
                    vec![
                        "full".to_string(),
                        "corners".to_string(),
                        "underline".to_string(),
                        "upperline".to_string(),
                        "left".to_string(),
                        "right".to_string(),
                    ],
                    Some(n.border_type.clone()),
                    |value| Message::NotifStyleChanged("border_type".to_string(), value),
                ).width(Length::Fill).into()),
            ]));
    }

    card(
        pal,
        column![
            text("Desktop Notifications").size(16).style(fg_text(pal)),
            body,
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// The "Sections" block: one card per section, plus the add button.
fn sections_list(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut section_list = Column::new().spacing(15);
    for (i, section) in state.config.sections.iter().enumerate() {
        section_list = section_list.push(section_view(state, i, section));
    }

    let add_section = button(text("+ Add Section").size(BODY))
        .on_press(Message::AddSectionPressed)
        .padding(10);

    column![
        text("Sections").size(16).style(fg_text(pal)),
        section_list,
        add_section,
    ]
    .spacing(10)
    .into()
}

/// Section card (accordion): a compact header — toggle with the section
/// name and a layout/sensor summary, then order and remove — over a body
/// (name, settings, sensors) that collapses to keep the list scannable.
fn section_view<'a>(
    state: &'a PiumaSetup,
    index: usize,
    section: &'a Section,
) -> Element<'a, Message> {
    let pal = palette(&state.config);
    let collapsed = state.sections_collapsed.contains(&index);
    let title = section.name.clone().unwrap_or_else(|| format!("Section {}", index + 1));
    let layout = section.layout.as_deref().unwrap_or(&state.config.section_layout);
    let count = section.sensors.len();
    let summary = format!("{} · {} sensor{}", layout, count, if count == 1 { "" } else { "s" });
    let header = Row::new()
        .push(collapsible_header(
            pal,
            title,
            summary,
            !collapsed,
            Message::ToggleSection(index),
        ))
        .push(move_btn(pal, "▲", index > 0, Message::SectionMoveUp(index)))
        .push(move_btn(
            pal,
            "▼",
            index + 1 < state.config.sections.len(),
            Message::SectionMoveDown(index),
        ))
        .push({
            // The last section keeps its button visible but inert.
            let b = button(text("Remove").size(BODY)).padding(5);
            if state.config.sections.len() > 1 {
                b.on_press(Message::RemoveSectionPressed(index))
            } else {
                b
            }
        })
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if collapsed {
        return card(pal, column![header].width(Length::Fill)).into();
    }

    // Section labels for the sensors' "Move to" picklists: the number
    // prefix keeps duplicates and empty names distinguishable.
    let section_labels: Vec<String> = state
        .config
        .sections
        .iter()
        .enumerate()
        .map(|(n, sec)| format!("{} · {}", n + 1, sec.name.as_deref().unwrap_or("(unnamed)")))
        .collect();

    let name_row = form_cell(
        "Name",
        input(
            "Section name",
            section.name.as_deref().unwrap_or(""),
            move |value| Message::SectionNameChanged(index, value),
        )
        .width(Length::Fill)
        .into(),
    );
    let settings = form_grid(vec![
        ("Layout", picklist(
            vec!["horizontal".to_string(), "vertical".to_string()],
            section.layout.as_ref(),
            move |value| Message::SectionLayoutChangedIndex(index, value),
        ).width(Length::Fill).into()),
        ("BG", color_control(section.bg_color.as_deref().unwrap_or(""), move |value| Message::SectionBgColorChanged(index, value))),
        ("Text", color_control(section.text_color.as_deref().unwrap_or(""), move |value| Message::SectionTextColorChanged(index, value))),
        ("Border", color_control(section.border_color.as_deref().unwrap_or(""), move |value| Message::SectionStyleChanged(index, "border_color".to_string(), value))),
        ("Border Width", slider_control(-1i32..=10, section.border_width.unwrap_or(-1), move |value| Message::SectionNumberChanged(index, "border_width".to_string(), value))),
        ("Border Radius", slider_control(-1i32..=20, section.border_radius.unwrap_or(-1), move |value| Message::SectionNumberChanged(index, "border_radius".to_string(), value))),
        ("Border Type", picklist(
            vec![
                "full".to_string(),
                "corners".to_string(),
                "underline".to_string(),
                "upperline".to_string(),
                "left".to_string(),
                "right".to_string(),
            ],
            Some(section.border_type.clone().unwrap_or_else(|| "full".to_string())),
            move |value| Message::SectionStyleChanged(index, "border_type".to_string(), value),
        ).width(Length::Fill).into()),
    ]);

    let mut sensor_list = Column::new().spacing(8).width(Length::Fill);
    for (j, sensor) in section.sensors.iter().enumerate() {
        sensor_list = sensor_list.push(sensor_view(
            pal,
            &state.config,
            index,
            j,
            section.sensors.len(),
            sensor,
            state.sensors_open.contains(&(index, j)),
            state.sensor_style_open.contains(&(index, j)),
            section_labels.clone(),
        ));
    }
    sensor_list = sensor_list.push(
        button(text("+ Add Sensor").size(BODY))
            .on_press(Message::AddSensorPressed(index))
            .padding(6),
    );
    let sensors = container(sensor_list)
        .padding(10)
        .width(Length::Fill)
        .style(inset_style(pal));

    card(
        pal,
        column![header, name_row, settings, sensors]
            .spacing(10)
            .width(Length::Fill),
    )
    .into()
}

/// Legacy PascalCase source type → canonical prefixed/lowercase name, so
/// labels and inputs work for old configs too (the stored value itself is
/// never rewritten on save).
/// Maps a legacy type name to the canonical round-9 name; anything else
/// (already canonical) passes through. Applied when loading, so the whole
/// editor — picklist, param inputs, save — only ever sees canonical names.
/// Apply one string style field of `panel.notifications` by JSON key:
/// the single place every style control of the card writes through
/// (unknown keys are ignored).
fn set_notif_style(n: &mut NotificationsCfg, key: &str, value: &str) {
    let slot = match key {
        "text_color" => &mut n.text_color,
        "text_color_hover" => &mut n.text_color_hover,
        "bg_color" => &mut n.bg_color,
        "bg_color_hover" => &mut n.bg_color_hover,
        "border_color" => &mut n.border_color,
        "border_color_hover" => &mut n.border_color_hover,
        "border_type" => &mut n.border_type,
        _ => return,
    };
    *slot = value.to_string();
}

/// Numeric sibling of `set_notif_style`; the font size clamps to the
/// readable range its slider offers, the margins to 0..=500 px.
fn set_notif_number(n: &mut NotificationsCfg, key: &str, value: i32) {
    match key {
        "font_size" => n.font_size = value.clamp(8, 64) as u32,
        "border_width" => n.border_width = value,
        "border_radius" => n.border_radius = value,
        "padding" => n.padding = value,
        "popup_margin_h" => n.popup_margin_h = value.clamp(0, 500),
        "popup_margin_v" => n.popup_margin_v = value.clamp(0, 500),
        _ => {}
    }
}

fn canonical_type<'a>(source_type: &'a str) -> &'a str {
    match source_type {
        "ProcCpu" => "perc_cpu",
        "NvidiaGpu" => "perc_gpu",
        "ProcMem" => "detail_mem",
        "ProcDisk" => "perc_disk",
        "Audio" => "perc_volume",
        "Network" => "net_state",
        "Uptime" => "uptime",
        "Clock" => "clock",
        "Window" => "window",
        "Workspace" => "workspace",
        "Command" => "command",
        "EnvVar" => "env_var",
        "StaticText" => "static_text",
        other => other,
    }
}

/// Canonical source types for the picklist, grouped by naming convention:
/// Every canonical source type; the combo shows them sorted
/// alphabetically (see [`available_source_types`]).
const SOURCE_TYPES: &[&str] = &[
    // perc_ — percentages
    "perc_cpu", "perc_gpu", "perc_mem", "perc_swap", "perc_disk",
    "perc_battery", "perc_backlight", "perc_volume",
    // used_ — numeric consumption
    "used_mem", "used_swap", "used_disk", "used_battery",
    "used_netdown", "used_netup",
    // total_ — full values
    "total_mem", "total_swap", "total_disk", "total_battery",
    // detail_ — used/total ratios
    "detail_mem", "detail_swap", "detail_disk",
    // temp_ — temperatures
    "temp_cpu",
    // no numeric category
    "net_state", "uptime", "clock", "calendar", "window", "workspace",
    "command", "env_var", "static_text", "tray", "notifications",
];

/// The sensor-type combo's options (roadmap2 #6): every canonical type
/// in alphabetical order; an unknown type from the edited file joins
/// the list (also sorted) so the selection stays visible.
fn available_source_types(current: &str) -> Vec<String> {
    let mut types: Vec<String> = SOURCE_TYPES.iter().map(|s| s.to_string()).collect();
    if !types.iter().any(|t| t == current) {
        types.push(current.to_string());
    }
    types.sort();
    types
}

/// Friendly label for a canonical source type (shown when it needs no
/// extra configuration).
fn source_label(canonical: &str) -> &'static str {
    match canonical {
        "perc_cpu" => "CPU usage",
        "perc_gpu" => "GPU usage",
        "perc_mem" => "Memory %",
        "perc_swap" => "Swap %",
        "perc_disk" => "Disk %",
        "perc_battery" => "Battery %",
        "perc_backlight" => "Backlight %",
        "perc_volume" => "Volume",
        "used_mem" => "Memory used",
        "used_swap" => "Swap used",
        "used_disk" => "Disk used",
        "used_battery" => "Battery used",
        "used_netdown" => "Net download rate",
        "used_netup" => "Net upload rate",
        "total_mem" => "Memory total",
        "total_swap" => "Swap total",
        "total_disk" => "Disk total",
        "total_battery" => "Battery total",
        "detail_mem" => "Memory used/total",
        "detail_swap" => "Swap used/total",
        "detail_disk" => "Disk used/total",
        "temp_cpu" => "CPU temperature",
        "net_state" => "Net state",
        "uptime" => "Uptime",
        "clock" => "Clock",
        "calendar" => "Calendar",
        "window" => "Focused window",
        "workspace" => "Workspace",
        "tray" => "System tray",
        "notifications" => "Desktop notifications",
        "command" => "Shell command",
        "env_var" => "Environment variable",
        "static_text" => "Static text",
        _ => "Sensor",
    }
}

/// Compact "type · 'param' · graph" summary of a sensor's collapsed row.
fn sensor_summary(sensor: &Sensor) -> String {
    let cfg = &sensor.source_config;
    let param = cfg
        .device
        .as_deref()
        .or(cfg.interface.as_deref())
        .or(cfg.format.as_deref())
        .or(cfg.command.as_deref())
        .or(cfg.env_var.as_deref())
        .or(cfg.static_text.as_deref())
        .filter(|p| !p.is_empty());
    let mut parts = vec![canonical_type(&sensor.source_type).to_string()];
    if let Some(p) = param {
        parts.push(format!("'{p}'"));
    }
    if sensor.draw_graph {
        parts.push("graph".to_string());
    }
    parts.join(" · ")
}

/// Sensor row (accordion): the compact header carries the order, duplicate
/// and remove controls; the form (name, description, source, graph, style)
/// is the body, expanded on demand. `section_labels` feeds the cross-
/// section "Move to" picklist (its number prefix is the 1-based index).
fn sensor_view<'a>(
    pal: Palette,
    cfg: &'a PiumaConfig,
    section_index: usize,
    sensor_index: usize,
    sensor_count: usize,
    sensor: &'a Sensor,
    open: bool,
    style_open: bool,
    section_labels: Vec<String>,
) -> Element<'a, Message> {
    let title = sensor
        .name
        .clone()
        .unwrap_or_else(|| source_label(canonical_type(&sensor.source_type)).to_string());
    let mut header = Row::new();
    // The sensor's own icon leads the row, exactly as the bar draws it.
    if let Some(icon) = resolved_icon(cfg, sensor.icon.as_deref()) {
        header = header.push(icon);
    }
    let header = header
        .push(collapsible_header(
            pal,
            title,
            sensor_summary(sensor),
            open,
            Message::ToggleSensor(section_index, sensor_index),
        ))
        .push(move_btn(
            pal,
            "▲",
            sensor_index > 0,
            Message::SensorMoveUp(section_index, sensor_index),
        ))
        .push(move_btn(
            pal,
            "▼",
            sensor_index + 1 < sensor_count,
            Message::SensorMoveDown(section_index, sensor_index),
        ))
        .push(
            button(text("⧉").size(BODY))
                .on_press(Message::SensorDuplicated(section_index, sensor_index))
                .padding(5)
                .style(move |t, s| btn_secondary(pal, t, s)),
        )
        .push(
            button(text("Remove").size(BODY))
                .on_press(Message::RemoveSensorPressed(section_index, sensor_index))
                .padding(5),
        )
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if !open {
        return card(pal, column![header].width(Length::Fill)).into();
    }

    // Identity fields: the name input carries the id that add/duplicate
    // focus, so renaming the fresh row is the next keystroke.
    let fields = form_grid(vec![
        ("Name", input("Name", sensor.name.as_deref().unwrap_or(""), move |value| {
            Message::SensorNameChanged(section_index, sensor_index, value)
        })
        .id(sensor_name_id(section_index, sensor_index))
        .width(Length::Fill)
        .into()),
        ("Description", input(
            "Description",
            sensor.description.as_deref().unwrap_or(""),
            move |value| Message::SensorDescriptionChanged(section_index, sensor_index, value),
        )
        .width(Length::Fill)
        .into()),
    ]);

    // Options = the canonical types, alphabetical (roadmap2 #6); an
    // unknown/typo'd type from the file joins the list — still sorted —
    // so the current value never falls out of the options.
    let source_types = available_source_types(&sensor.source_type);
    let source_type_picklist = picklist(
        source_types,
        Some(&sensor.source_type),
        move |value| Message::SensorSourceTypeChanged(section_index, sensor_index, value),
    ).width(130);

    let source_config: Element<Message> = match canonical_type(&sensor.source_type) {
        "net_state" | "used_netdown" | "used_netup" => input("Interface", sensor.source_config.interface.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "interface".to_string(), value))
            .width(Length::Fill)
            .into(),
        "perc_volume" => input("Device", sensor.source_config.device.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "device".to_string(), value))
            .width(Length::Fill)
            .into(),
        "clock" | "calendar" => input("Format", sensor.source_config.format.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "format".to_string(), value))
            .width(Length::Fill)
            .into(),
        "command" => input("Command", sensor.source_config.command.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "command".to_string(), value))
            .width(Length::Fill)
            .into(),
        "env_var" => input("Env Var", sensor.source_config.env_var.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "env_var".to_string(), value))
            .width(Length::Fill)
            .into(),
        "static_text" => input("Text", sensor.source_config.static_text.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "text".to_string(), value))
            .width(Length::Fill)
            .into(),
        other => Row::new()
            .push(text(format!("{} (no extra config)", source_label(other))).size(BODY))
            .spacing(5)
            .into(),
    };

    let source = form_cell("Source", Row::new()
        .push(source_type_picklist)
        .push(source_config)
        .spacing(8)
        .width(Length::Fill)
        .into());

    // Icon name, live preview at the bar's size and a browse modal: the
    // preview resolves exactly like the bar, so it cannot lie.
    let icon = form_cell("Icon", Row::new()
        .push(input("icon name (theme)", sensor.icon.as_deref().unwrap_or(""), move |value| Message::SensorIconChanged(section_index, sensor_index, value))
            .width(Length::Fill))
        .push(button(text("…").size(BODY))
            .on_press(Message::BrowseIconsPressed(section_index, sensor_index))
            .padding(5)
            .style(move |t, s| btn_secondary(pal, t, s)))
        .push(icon_preview(cfg, sensor.icon.as_deref()))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into());

    let action = form_cell("Action", input("close_panel / command / ...", sensor.action.as_deref().unwrap_or(""), move |value| Message::SensorActionChanged(section_index, sensor_index, value))
        .width(Length::Fill)
        .into());

    // Hover trend popup; only the numeric sources (perc_/used_/temp_) graph.
    let draw_graph = form_cell("Draw graph", toggler(sensor.draw_graph)
        .on_toggle(move |value| Message::SensorDrawGraphChanged(section_index, sensor_index, value))
        .into());

    // Cross-section ordering: only when there is somewhere else to go.
    let move_to = if section_labels.len() > 1 {
        Some(form_cell(
            "Move to",
            picklist(section_labels, None::<String>, move |label: String| {
                // The label prefix is the 1-based number of the target.
                let target = label
                    .split(' ')
                    .next()
                    .and_then(|n| n.parse::<usize>().ok())
                    .map(|n| n - 1)
                    .unwrap_or(section_index);
                Message::SensorMoveTo(section_index, sensor_index, target)
            })
            .width(Length::Fill)
            .into(),
        ))
    } else {
        None
    };

    // Collapsed header shows which overrides are actually set.
    let summary = style_summary(
        &[
            ("text", &sensor.text_color), ("hover-text", &sensor.text_color_hover),
            ("bg", &sensor.bg_color), ("hover-bg", &sensor.bg_color_hover),
            ("border", &sensor.border_color), ("border-hover", &sensor.border_color_hover),
            ("border-type", &sensor.border_type),
        ],
        &[
            ("brd", &sensor.border_width), ("rad", &sensor.border_radius), ("pad", &sensor.padding),
        ],
    );
    let style_header = collapsible_header(
        pal,
        "Style overrides".to_string(),
        summary,
        style_open,
        Message::ToggleSensorStyle(section_index, sensor_index),
    );

    // The header (title + controls) stays visible in both states, so an
    // expanded sensor can still be collapsed, reordered, copied, removed.
    let mut root = column![header, fields, source, icon, action, draw_graph]
        .spacing(6)
        .width(Length::Fill);
    if let Some(move_to) = move_to {
        root = root.push(move_to);
    }
    root = root.push(style_header);

    // Style numbers: -1 = disabled (the defaults apply), 0..=N = enabled.
    if style_open {
        let fields = vec![
            ("Text", color_control(sensor.text_color.as_deref().unwrap_or(""), move |value| Message::SensorTextColorChanged(section_index, sensor_index, value))),
            ("Hover Text", color_control(sensor.text_color_hover.as_deref().unwrap_or(""), move |value| Message::SensorTextColorHoverChanged(section_index, sensor_index, value))),
            ("BG", color_control(sensor.bg_color.as_deref().unwrap_or(""), move |value| Message::SensorBgColorChanged(section_index, sensor_index, value))),
            ("Hover BG", color_control(sensor.bg_color_hover.as_deref().unwrap_or(""), move |value| Message::SensorBgColorHoverChanged(section_index, sensor_index, value))),
            ("Border", color_control(sensor.border_color.as_deref().unwrap_or(""), move |value| Message::SensorBorderColorChanged(section_index, sensor_index, value))),
            ("Border Hover", color_control(sensor.border_color_hover.as_deref().unwrap_or(""), move |value| Message::SensorBorderColorHoverChanged(section_index, sensor_index, value))),
            ("Border Width", slider_control(-1i32..=10, sensor.border_width.unwrap_or(-1), move |value| Message::SensorBorderWidthChanged(section_index, sensor_index, value))),
            ("Border Radius", slider_control(-1i32..=20, sensor.border_radius.unwrap_or(-1), move |value| Message::SensorBorderRadiusChanged(section_index, sensor_index, value))),
            ("Padding", slider_control(-1i32..=20, sensor.padding.unwrap_or(-1), move |value| Message::SensorPaddingChanged(section_index, sensor_index, value))),
            ("Border Type", picklist(
                vec![
                    "full".to_string(),
                    "corners".to_string(),
                    "underline".to_string(),
                    "upperline".to_string(),
                    "left".to_string(),
                    "right".to_string(),
                ],
                Some(sensor.border_type.clone().unwrap_or_else(|| "full".to_string())),
                move |value| Message::SensorBorderTypeChanged(section_index, sensor_index, value),
            ).width(Length::Fill).into()),
        ];
        root = root.push(
            container(form_grid(fields))
                .padding(10)
                .width(Length::Fill)
                .style(inset_style(pal)),
        );
    }

    card(pal, root).into()
}

/// The "Sensor Defaults" card: global style applied to every sensor,
/// collapsed by default because it only overrides group-wide values.
fn sensor_defaults_view(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let d = &state.config.sensor_defaults;

    let summary = style_summary(
        &[
            ("text", &d.text_color), ("hover-text", &d.text_color_hover),
            ("bg", &d.bg_color), ("hover-bg", &d.bg_color_hover),
            ("border", &d.border_color), ("border-hover", &d.border_color_hover),
            ("border-type", &d.border_type),
        ],
        &[
            ("brd", &d.border_width), ("rad", &d.border_radius), ("pad", &d.padding),
        ],
    );
    let header = collapsible_header(
        pal,
        "Sensor Defaults".to_string(),
        summary,
        state.defaults_expanded,
        Message::ToggleDefaults,
    );

    let mut root = column![header].spacing(10).width(Length::Fill);
    if state.defaults_expanded {
        root = root.push(
            text("Style applied to every sensor; single sensors override these values.")
                .size(10)
                .style(muted_text(pal)),
        );

        // Style numbers: -1 = disabled (the defaults apply), 0..=N = enabled.
        let fields = vec![
            ("Text", color_control(d.text_color.as_deref().unwrap_or(""), |value| Message::DefaultsStyleChanged("text_color".to_string(), value))),
            ("Hover Text", color_control(d.text_color_hover.as_deref().unwrap_or(""), |value| Message::DefaultsStyleChanged("text_color_hover".to_string(), value))),
            ("BG", color_control(d.bg_color.as_deref().unwrap_or(""), |value| Message::DefaultsStyleChanged("bg_color".to_string(), value))),
            ("Hover BG", color_control(d.bg_color_hover.as_deref().unwrap_or(""), |value| Message::DefaultsStyleChanged("bg_color_hover".to_string(), value))),
            ("Border", color_control(d.border_color.as_deref().unwrap_or(""), |value| Message::DefaultsStyleChanged("border_color".to_string(), value))),
            ("Border Hover", color_control(d.border_color_hover.as_deref().unwrap_or(""), |value| Message::DefaultsStyleChanged("border_color_hover".to_string(), value))),
            ("Border Width", slider_control(-1i32..=10, d.border_width.unwrap_or(-1), |value| Message::DefaultsNumberChanged("border_width".to_string(), value))),
            ("Border Radius", slider_control(-1i32..=20, d.border_radius.unwrap_or(-1), |value| Message::DefaultsNumberChanged("border_radius".to_string(), value))),
            ("Padding", slider_control(-1i32..=20, d.padding.unwrap_or(-1), |value| Message::DefaultsNumberChanged("padding".to_string(), value))),
            ("Border Type", picklist(
                vec![
                    "full".to_string(),
                    "corners".to_string(),
                    "underline".to_string(),
                    "upperline".to_string(),
                    "left".to_string(),
                    "right".to_string(),
                ],
                Some(d.border_type.clone().unwrap_or_else(|| "full".to_string())),
                |value| Message::DefaultsStyleChanged("border_type".to_string(), value),
            ).width(Length::Fill).into()),
        ];
        root = root.push(form_grid(fields));
    }

    card(pal, root).into()
}

/// The "Section Defaults" card: style applied to every section, overridden
/// per section (same group rule as the sensor defaults).
fn section_defaults_view(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let d = &state.config.section_defaults;

    let summary = style_summary(
        &[
            ("bg", &d.bg_color), ("text", &d.text_color),
            ("border", &d.border_color), ("border-type", &d.border_type),
        ],
        &[("brd", &d.border_width), ("rad", &d.border_radius)],
    );
    let header = collapsible_header(
        pal,
        "Section Defaults".to_string(),
        summary,
        state.sec_defaults_expanded,
        Message::ToggleSecDefaults,
    );

    let mut root = column![header].spacing(10).width(Length::Fill);
    if state.sec_defaults_expanded {
        root = root.push(
            text("Style applied to every section; single sections override these values.")
                .size(10)
                .style(muted_text(pal)),
        );

        // Style numbers: -1 = disabled (built-in fallback), 0..=N = enabled.
        let fields = vec![
            ("BG", color_control(d.bg_color.as_deref().unwrap_or(""), |value| Message::SecDefaultsStyleChanged("bg_color".to_string(), value))),
            ("Text", color_control(d.text_color.as_deref().unwrap_or(""), |value| Message::SecDefaultsStyleChanged("text_color".to_string(), value))),
            ("Border", color_control(d.border_color.as_deref().unwrap_or(""), |value| Message::SecDefaultsStyleChanged("border_color".to_string(), value))),
            ("Border Width", slider_control(-1i32..=10, d.border_width.unwrap_or(-1), |value| Message::SecDefaultsNumberChanged("border_width".to_string(), value))),
            ("Border Radius", slider_control(-1i32..=20, d.border_radius.unwrap_or(-1), |value| Message::SecDefaultsNumberChanged("border_radius".to_string(), value))),
            ("Border Type", picklist(
                vec![
                    "full".to_string(),
                    "corners".to_string(),
                    "underline".to_string(),
                    "upperline".to_string(),
                    "left".to_string(),
                    "right".to_string(),
                ],
                Some(d.border_type.clone().unwrap_or_else(|| "full".to_string())),
                |value| Message::SecDefaultsStyleChanged("border_type".to_string(), value),
            ).width(Length::Fill).into()),
            // Roadmap M3: only sides NOT flush with a screen edge get the
            // rounded corners; the flag comes from these defaults alone.
            ("Screen Edges", toggler(d.respect_screen_edges.unwrap_or(false))
                .label("Borders respect screen edges")
                .on_toggle(Message::SecDefaultsRespectEdges)
                .into()),
        ];
        root = root.push(form_grid(fields));
    }

    card(pal, root).into()
}

/// Load / Save / Save As / Reset actions for the fixed bottom bar.
fn action_buttons(pal: Palette) -> Element<'static, Message> {
    let load_btn = button(text("Load…").size(BODY))
        .on_press(Message::LoadConfig)
        .padding(10)
        .style(move |t, s| btn_secondary(pal, t, s));
    let save_btn = button(text("Save").size(BODY))
        .on_press(Message::SaveConfig)
        .padding(10)
        .style(move |t, s| btn_primary(pal, t, s));
    let save_as_btn = button(text("Save As…").size(BODY))
        .on_press(Message::SaveAs)
        .padding(10)
        .style(move |t, s| btn_secondary(pal, t, s));
    let reset_btn = button(text("Reset").size(BODY))
        .on_press(Message::ResetConfig)
        .padding(10);

    Row::new()
        .push(load_btn)
        .push(save_btn)
        .push(save_as_btn)
        .push(reset_btn)
        .spacing(10)
        .into()
}

fn status_bar(state: &PiumaSetup) -> Element<'_, Message> {
    match &state.status_message {
        Some(msg) => {
            let is_error = state.status_is_error;
            text(msg).size(10).style(move |_t: &Theme| TextStyle { color: Some(if is_error { Color::from_rgb(0.9, 0.3, 0.3) } else { Color::from_rgb(0.3, 0.8, 0.4) }), ..TextStyle::default() }).into()
        }
        None => text("").into(),
    }
}

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
    iced::application(PiumaSetup::new, update, view)
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
mod tests {
    use super::*;

    /// The user's real piuma.json before conversion: legacy and canonical
    /// type names mixed, plus actions, style overrides and -1 disabled
    /// values that must all survive a load→save round-trip.
    const LEGACY_CONFIG: &str = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "default_bg_color": "#00000000",
    "default_section_weight": 1,
    "default_text_color": "#eeeeeeee",
    "font": "Noto Sans",
    "font_size": 11,
    "section_defaults": {
      "bg_color": "#30609010",
      "border_color": "#30609020",
      "border_radius": 8,
      "border_type": "full",
      "border_width": 2
    },
    "section_layout": "horizontal",
    "sections": [
      {
        "layout": "horizontal",
        "name": "hyprland",
        "sensors": [
          {
            "action": "hyprctl dispatch 'hl.dsp.focus({ workspace = \"1\" })'",
            "name": "1",
            "source": { "type": "Command" }
          },
          {
            "action": "hyprctl dispatch 'hl.dsp.focus({ workspace = \"2\" })'",
            "name": "2",
            "source": { "type": "static_text" }
          },
          {
            "name": "Window",
            "source": { "type": "Window" }
          }
        ],
        "text_color": "#ff6b6b"
      },
      {
        "layout": "horizontal",
        "name": "System",
        "sensors": [
          {
            "border_radius": -1,
            "border_type": "corners",
            "border_width": -1,
            "name": "CPU%",
            "padding": -1,
            "source": { "type": "ProcCpu" }
          },
          {
            "name": "GPU%",
            "source": { "type": "NvidiaGpu" }
          },
          {
            "name": "MEM",
            "source": { "type": "ProcMem" }
          },
          {
            "name": "NET",
            "source": { "interface": "wlan0", "type": "Network" }
          },
          {
            "name": "VOL",
            "source": { "device": "Master", "type": "Audio" }
          },
          {
            "name": "CAL",
            "source": { "command": "date", "type": "Command" }
          },
          {
            "name": "SHELL",
            "source": { "env_var": "SHELL", "type": "EnvVar" }
          }
        ]
      }
    ],
    "sensor_defaults": { "padding": 3 },
    "thickness": 26
  }
}"##;

    /// A PiumaSetup that never touches the filesystem or config dir.
    fn fixture() -> PiumaSetup {
        PiumaSetup {
            config: PiumaConfig::default(),
            active_tab: Tab::Panel,
            status_message: None,
            status_is_error: false,
            defaults_expanded: true,
            sec_defaults_expanded: true,
            sensor_style_open: HashSet::new(),
            sections_collapsed: HashSet::new(),
            sensors_open: HashSet::new(),
            current_file: "piuma.json".to_string(),
            snapshot: String::new(),
            modal: None,
            wallpaper_files: Vec::new(),
            autostart_apps: Vec::new(),
        }
    }

    /// Load→save of a legacy config: only the type names may change
    /// (to their canonical spelling); params, actions, names, styles,
    /// section names and -1 values must come out untouched.
    #[test]
    fn legacy_types_convert_and_nothing_else_changes() {
        let mut state = fixture();
        state.load_from_json(LEGACY_CONFIG).expect("fixture must parse");
        let out: JsonValue =
            serde_json::from_str(&state.serialize().expect("serialization must succeed")).expect("saved JSON must parse");

        // Independent oracle: the input with only the five legacy names
        // of this fixture mapped by hand (no canonical_type call here).
        let mut expected: JsonValue = serde_json::from_str(LEGACY_CONFIG).unwrap();
        for section in expected["panel"]["sections"].as_array_mut().unwrap() {
            for sensor in section["sensors"].as_array_mut().unwrap() {
                let ty = sensor["source"]["type"].as_str().unwrap().to_string();
                let mapped = match ty.as_str() {
                    "ProcCpu" => "perc_cpu",
                    "NvidiaGpu" => "perc_gpu",
                    "ProcMem" => "detail_mem",
                    "Window" => "window",
                    "Command" => "command",
                    "Network" => "net_state",
                    "Audio" => "perc_volume",
                    "EnvVar" => "env_var",
                    other => other,
                };
                sensor["source"]["type"] = JsonValue::String(mapped.to_string());
            }
        }
        assert_eq!(out, expected, "save must rewrite only source.type, never drop or inject anything");
    }

    /// A converted config that is loaded and saved again must not change
    /// any further time (conversion is idempotent).
    #[test]
    fn conversion_is_idempotent() {
        let mut state = fixture();
        state.load_from_json(LEGACY_CONFIG).unwrap();
        let first = state.serialize().unwrap();
        state.load_from_json(&first).unwrap();
        assert_eq!(state.serialize().unwrap(), first);
    }

    /// Save must fail with the serde reason — before touching any file —
    /// for a config piuma would reject (the fixture holds sensors with a
    /// required parameter missing: command and static_text).
    #[test]
    fn save_refuses_config_piuma_would_reject() {
        let mut state = fixture();
        state.load_from_json(LEGACY_CONFIG).unwrap();
        let err = state.save_config("never-written.json").unwrap_err();
        assert!(err.contains("Save rejected") && err.contains("missing field"),
            "error must state the rejection and its reason, got: {}", err);
        assert!(!file_path("never-written.json").exists(), "no file may be written");
    }

    /// A tray sensor must survive load→save untouched (type name included)
    /// and be accepted by the same parse `save_config` validates with.
    #[test]
    fn tray_sensor_roundtrips_and_validates() {
        let json = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "section_layout": "horizontal",
    "sections": [
      {
        "layout": "horizontal",
        "name": "sys",
        "sensors": [ { "name": "Tray", "source": { "type": "tray" } } ]
      }
    ],
    "thickness": 26
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("tray config must load");
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["sections"][0]["sensors"][0]["source"]["type"], "tray");
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
    }

    /// `panel.notifications` and a notifications sensor must survive
    /// load→save untouched (values and presence included).
    #[test]
    fn notifications_block_and_sensor_roundtrip() {
        let json = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "notifications": { "default_timeout_ms": 4000, "history_size": 7, "replace_existing": false, "font_size": 16, "text_color": "#ffe066", "bg_color": "#101010", "border_width": 2, "border_radius": 16, "border_type": "corners", "padding": 8 },
    "section_layout": "horizontal",
    "sections": [
      { "layout": "horizontal", "name": "sys",
        "sensors": [ { "name": "Notifiche", "source": { "type": "notifications" } } ] }
    ],
    "thickness": 26
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        let n = state.config.notifications.as_ref().expect("block must load");
        assert_eq!(
            (n.default_timeout_ms, n.history_size, n.replace_existing),
            (4000, 7, false)
        );
        assert_eq!(
            (
                n.font_size,
                n.text_color.as_str(),
                n.bg_color.as_str(),
                n.border_width,
                n.border_radius,
                n.border_type.as_str(),
                n.padding
            ),
            (16, "#ffe066", "#101010", 2, 16, "corners", 8)
        );
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["notifications"]["default_timeout_ms"], 4000);
        assert_eq!(out["panel"]["notifications"]["history_size"], 7);
        assert_eq!(out["panel"]["notifications"]["replace_existing"], false);
        assert_eq!(out["panel"]["notifications"]["font_size"], 16);
        assert_eq!(out["panel"]["notifications"]["text_color"], "#ffe066");
        assert_eq!(out["panel"]["notifications"]["border_width"], 2);
        assert_eq!(out["panel"]["notifications"]["border_type"], "corners");
        assert_eq!(
            out["panel"]["sections"][0]["sensors"][0]["source"]["type"],
            "notifications"
        );
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
    }

    /// The roadmap M2–M4 keys (`bold`, `respect_screen_edges`,
    /// `popup_position` + margins) roundtrip load→save with their values,
    /// and the result still parses as piuma's own schema.
    #[test]
    fn roadmap_keys_roundtrip() {
        let json = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "bold": true,
    "notifications": { "popup_position": "bottom-left", "popup_margin_h": 20, "popup_margin_v": 4 },
    "section_defaults": { "respect_screen_edges": true },
    "section_layout": "horizontal",
    "sections": [],
    "thickness": 26
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        assert_eq!(state.config.bold, Some(true));
        let n = state.config.notifications.as_ref().expect("block must load");
        assert_eq!(
            (n.popup_position.as_str(), n.popup_margin_h, n.popup_margin_v),
            ("bottom-left", 20, 4)
        );
        assert_eq!(
            state.config.section_defaults.respect_screen_edges,
            Some(true)
        );
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["bold"], true);
        assert_eq!(out["panel"]["notifications"]["popup_position"], "bottom-left");
        assert_eq!(out["panel"]["notifications"]["popup_margin_h"], 20);
        assert_eq!(out["panel"]["notifications"]["popup_margin_v"], 4);
        assert_eq!(out["panel"]["section_defaults"]["respect_screen_edges"], true);
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
    }

    /// The roadmap N1 keys (`panel.icon_theme` and the per-sensor `icon`)
    /// roundtrip load→save with their values, and the result still parses
    /// as piuma's own schema.
    #[test]
    fn icon_keys_roundtrip() {
        let json = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "icon_theme": "Papirus",
    "section_layout": "horizontal",
    "sections": [
      { "name": "sys", "sensors": [ { "name": "CPU", "icon": "cpu", "source": { "type": "perc_cpu" } } ] }
    ],
    "thickness": 26
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        assert_eq!(state.config.icon_theme.as_deref(), Some("Papirus"));
        assert_eq!(
            state.config.sections[0].sensors[0].icon.as_deref(),
            Some("cpu")
        );
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["icon_theme"], "Papirus");
        assert_eq!(out["panel"]["sections"][0]["sensors"][0]["icon"], "cpu");
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
        // A save never injects the keys when they were absent.
        state.config.icon_theme = None;
        state.config.sections[0].sensors[0].icon = None;
        let bare: JsonValue =
            serde_json::from_str(&state.serialize().expect("serialize must succeed")).unwrap();
        assert!(bare["panel"].get("icon_theme").is_none());
        assert!(bare["panel"]["sections"][0]["sensors"][0].get("icon").is_none());
    }

    /// The roadmap N2 key (`panel.autostart`) roundtrips load→save with
    /// its value, the result still parses as piuma's own schema, and a
    /// save never injects the key when it was absent.
    #[test]
    fn autostart_key_roundtrip() {
        let json = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "autostart": true,
    "section_layout": "horizontal",
    "sections": [],
    "thickness": 26
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        assert_eq!(state.config.autostart, Some(true));
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["autostart"], true);
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
        // Toggle off = key absent (the bar reads false).
        state.config.autostart = None;
        let bare: JsonValue =
            serde_json::from_str(&state.serialize().expect("serialize must succeed")).unwrap();
        assert!(bare["panel"].get("autostart").is_none());
    }

    /// The per-app autostart selection (`panel.autostart_disabled`)
    /// roundtrips load→save with its names in order, the result still
    /// parses as piuma's own schema, and an empty list never injects
    /// the key.
    #[test]
    fn autostart_disabled_key_roundtrip() {
        let json = r##"{
  "panel": {
    "autostart": true,
    "autostart_disabled": ["b.desktop", "a.desktop"],
    "section_layout": "horizontal",
    "sections": []
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        assert_eq!(
            state.config.autostart_disabled,
            vec!["b.desktop".to_string(), "a.desktop".to_string()]
        );
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["autostart_disabled"][0], "b.desktop");
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
        // An empty list = key absent (a save never injects it).
        state.config.autostart_disabled = Vec::new();
        let bare: JsonValue =
            serde_json::from_str(&state.serialize().expect("serialize must succeed")).unwrap();
        assert!(bare["panel"].get("autostart_disabled").is_none());
    }

    /// The roadmap N3 key (`panel.app_theme`) roundtrips load→save with
    /// its value, the result still parses as piuma's own schema, and a
    /// save never injects the key when it was absent.
    #[test]
    fn app_theme_key_roundtrip() {
        let json = r##"{
  "panel": {
    "align": "fill",
    "anchor": "top",
    "app_theme": true,
    "section_layout": "horizontal",
    "sections": [],
    "thickness": 26
  }
}"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        assert_eq!(state.config.app_theme, Some(true));
        let saved = state.serialize().expect("serialize must succeed");
        let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
        assert_eq!(out["panel"]["app_theme"], true);
        serde_json::from_str::<config::Config>(&saved)
            .expect("piuma must accept the saved config (pre-save validation)");
        // Toggle off = key absent.
        state.config.app_theme = None;
        let bare: JsonValue =
            serde_json::from_str(&state.serialize().expect("serialize must succeed")).unwrap();
        assert!(bare["panel"].get("app_theme").is_none());
    }

    /// Roadmap N4: the sidebar switches the visible category while the
    /// edited config is carried over untouched (edits live in
    /// `state.config`, not in the cards, so no tab loses work).
    #[test]
    fn sidebar_tab_selection_switches_the_active_tab() {
        let mut state = fixture();
        let before = state.serialize().expect("serialize must succeed");
        assert_eq!(state.active_tab, Tab::Panel);
        let _ = update(&mut state, Message::TabSelected(Tab::Sections));
        assert_eq!(state.active_tab, Tab::Sections);
        let _ = update(&mut state, Message::TabSelected(Tab::Notifications));
        assert_eq!(state.active_tab, Tab::Notifications);
        assert_eq!(
            state.serialize().expect("serialize must succeed"),
            before,
            "switching tabs must not change the edited config"
        );
    }

    /// The editor writes all three keys when enabled (defaults on a fresh
    /// enable) and drops the block entirely when disabled — that toggle is
    /// what starts/stops piuma's D-Bus server.
    #[test]
    fn notifications_editor_writes_and_clears_the_block() {
        let mut state = fixture();
        state.config.notifications = Some(NotificationsCfg::default());
        let out: JsonValue = serde_json::from_str(&state.serialize().unwrap()).unwrap();
        assert_eq!(out["panel"]["notifications"]["default_timeout_ms"], 5000);
        assert_eq!(out["panel"]["notifications"]["history_size"], 20);
        assert_eq!(out["panel"]["notifications"]["replace_existing"], true);
        assert_eq!(out["panel"]["notifications"]["font_size"], 12);
        assert_eq!(out["panel"]["notifications"]["text_color"], "#ffffff");
        assert_eq!(out["panel"]["notifications"]["border_color"], "#808080");
        assert_eq!(out["panel"]["notifications"]["padding"], 12);
        serde_json::from_str::<config::Config>(&out.to_string())
            .expect("piuma must accept the defaults block");

        state.config.notifications = None;
        let out: JsonValue = serde_json::from_str(&state.serialize().unwrap()).unwrap();
        assert!(
            out["panel"].get("notifications").is_none(),
            "disabled must serialize without the block"
        );
    }

    /// Absent or negative style keys resolve to the popup defaults on
    /// load: the editor then shows (and rewrites) the resolved values,
    /// exactly like it does for the server keys.
    #[test]
    fn notifications_style_defaults_resolve_on_load() {
        let json = r##"{ "panel": { "notifications": { "padding": -1, "border_type": "kittens" }, "sections": [] } }"##;
        let mut state = fixture();
        state.load_from_json(json).expect("config must load");
        let n = state.config.notifications.as_ref().expect("block must load");
        assert_eq!(
            (
                n.padding,
                n.font_size,
                n.text_color.as_str(),
                n.border_color.as_str(),
                n.border_type.as_str()
            ),
            (12, 12, "#ffffff", "#808080", "full"),
            "negative padding and an unknown border type fall back"
        );
    }

    /// Roadmap2 #1: the wallpaper block roundtrips load→save whole and
    /// absent, and a partial block falls back per field — exactly what
    /// the save path's `config::Config` validation accepts.
    #[test]
    fn wallpaper_key_roundtrip() {
        let mut state = fixture();
        state
            .load_from_json(r#"{"panel":{"wallpaper":{"enabled":true,"path":"/tmp/a.png"}}}"#)
            .expect("wallpaper block must parse");
        let wp = state.config.wallpaper.as_ref().expect("block present");
        assert!(wp.enabled);
        assert_eq!(wp.path, "/tmp/a.png");
        let out: JsonValue = serde_json::from_str(&state.serialize().expect("serialize")).expect("json");
        assert_eq!(out["panel"]["wallpaper"]["enabled"], true);
        assert_eq!(out["panel"]["wallpaper"]["path"], "/tmp/a.png");

        // A partial block falls back per field…
        let mut state = fixture();
        state
            .load_from_json(r#"{"panel":{"wallpaper":{}}}"#)
            .expect("partial block must parse");
        let wp = state.config.wallpaper.as_ref().expect("block present");
        assert!(!wp.enabled);
        assert!(wp.path.is_empty());

        // …and an absent block stays absent (no key is ever written).
        let mut state = fixture();
        state
            .load_from_json(r#"{"panel":{}}"#)
            .expect("minimal config must parse");
        assert!(state.config.wallpaper.is_none());
        let out: JsonValue = serde_json::from_str(&state.serialize().expect("serialize")).expect("json");
        assert!(out["panel"].get("wallpaper").is_none());
    }

    /// Roadmap2 #1: the scanner picks images recursively and sorted and
    /// ignores everything else (freedesktop folders mix metadata with
    /// pictures); unknown folders are skipped without error.
    #[test]
    fn scan_finds_images_recursively() {
        let root = std::env::temp_dir().join(format!("piuma-wp-scan-{}", std::process::id()));
        let nested = root.join("bundle").join("contents");
        fs::create_dir_all(&nested).expect("temp dirs");
        fs::write(root.join("b.jpg"), b"").expect("file");
        fs::write(nested.join("a.png"), b"").expect("file");
        fs::write(root.join("notes.txt"), b"").expect("file");
        let found = scan_wallpaper_dirs(&[root.clone(), PathBuf::from("/does/not/exist")]);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        // Sorted by full path: root/b.jpg sorts before root/bundle/….
        assert_eq!(names, vec!["b.jpg".to_string(), "a.png".to_string()]);
        fs::remove_dir_all(&root).ok();
    }

    /// Roadmap2 #1: `XDG_PICTURES_DIR` is read from user-dirs.dirs —
    /// comments and other keys ignored, quotes stripped, empty dropped.
    #[test]
    fn user_dirs_pictures_parses() {
        let content = "# Generated\nXDG_DESKTOP_DIR=\"$HOME/Scrivania\"\nXDG_PICTURES_DIR=\"$HOME/Immagini\"\n";
        assert_eq!(
            parse_user_dirs_pictures(content).as_deref(),
            Some("$HOME/Immagini")
        );
        assert_eq!(parse_user_dirs_pictures("#XDG_PICTURES_DIR=\"$HOME/x\"\n"), None);
        assert_eq!(parse_user_dirs_pictures("XDG_PICTURES_DIR=\"\"\n"), None);
    }

    /// Roadmap2 #6: the sensor-type combo is alphabetical, and an
    /// unknown type from the file joins the list still sorted (the
    /// selection never falls out of the options).
    #[test]
    fn available_source_types_are_sorted() {
        let types = available_source_types("perc_cpu");
        let mut sorted = types.clone();
        sorted.sort();
        assert_eq!(types, sorted, "canonical types must come out sorted");
        assert!(types.contains(&"perc_cpu".to_string()));

        let types = available_source_types("z_custom_type");
        let mut sorted = types.clone();
        sorted.sort();
        assert_eq!(types, sorted, "the unknown type must sort in too");
        assert!(types.contains(&"z_custom_type".to_string()));
    }

    /// State seeded with two sections of one sensor each: enough to drive
    /// the Sections update handlers without touching the user's file.
    fn test_state() -> PiumaSetup {
        PiumaSetup {
            config: PiumaConfig {
                sections: vec![
                    Section {
                        name: Some("Left".to_string()),
                        sensors: vec![Sensor::default()],
                        ..Section::default()
                    },
                    Section {
                        name: Some("Right".to_string()),
                        sensors: vec![Sensor::default()],
                        ..Section::default()
                    },
                ],
                ..PiumaConfig::default()
            },
            active_tab: Tab::Sections,
            ..fixture()
        }
    }

    /// The add-sensor flow: the modal carries the picked type and name
    /// into a fresh sensor, which lands at the section's end already
    /// expanded (and with its section unfolded).
    #[test]
    fn add_sensor_modal_appends_typed_sensor_and_opens_it() {
        let mut s = test_state();
        let _ = update(&mut s, Message::AddSensorPressed(0));
        assert!(matches!(s.modal, Some(Modal::AddSensor { .. })));
        let _ = update(&mut s, Message::ModalTypeChanged("perc_disk".to_string()));
        let _ = update(&mut s, Message::ModalTextChanged("Root".to_string()));
        let _ = update(&mut s, Message::ModalConfirm);
        let sensors = &s.config.sections[0].sensors;
        assert_eq!(sensors.len(), 2);
        assert_eq!(sensors[1].source_type, "perc_disk");
        assert_eq!(sensors[1].name.as_deref(), Some("Root"));
        assert!(s.sensors_open.contains(&(0, 1)));
        assert!(!s.sections_collapsed.contains(&0));
        assert!(s.modal.is_none());
    }

    /// An empty name leaves the key absent (the bar renders value only).
    #[test]
    fn add_sensor_without_name_stays_unnamed() {
        let mut s = test_state();
        let _ = update(&mut s, Message::AddSensorPressed(1));
        let _ = update(&mut s, Message::ModalTextChanged("   ".to_string()));
        let _ = update(&mut s, Message::ModalConfirm);
        let sensors = &s.config.sections[1].sensors;
        assert_eq!(sensors.len(), 2);
        assert!(sensors[1].name.is_none());
    }

    /// Adding at the end shifts the open keys of that section only.
    #[test]
    fn add_sensor_keeps_existing_open_keys_aligned() {
        let mut s = test_state();
        s.sensors_open.insert((0, 0));
        s.sensor_style_open.insert((0, 0));
        let _ = update(&mut s, Message::AddSensorPressed(0));
        let _ = update(&mut s, Message::ModalConfirm);
        assert!(s.sensors_open.contains(&(0, 0)) && s.sensors_open.contains(&(0, 1)));
        assert!(s.sensor_style_open.contains(&(0, 0)), "earlier keys survive the insert");
    }

    /// Duplicating clones the sensor right after itself, marks the copy
    /// with "(copy)" and opens it.
    #[test]
    fn duplicate_sensor_marks_the_copy() {
        let mut s = test_state();
        s.config.sections[0].sensors[0].name = Some("Disk".to_string());
        let _ = update(&mut s, Message::SensorDuplicated(0, 0));
        let sensors = &s.config.sections[0].sensors;
        assert_eq!(sensors.len(), 2);
        assert_eq!(sensors[1].name.as_deref(), Some("Disk (copy)"));
        assert!(s.sensors_open.contains(&(0, 1)));
    }

    /// Removal goes through a confirmation that shifts the open keys of
    /// the section down, so style/accordion blocks follow their sensor.
    #[test]
    fn remove_sensor_confirmation_remaps_open_sets() {
        let mut s = test_state();
        s.config.sections[0].sensors.push(Sensor::default());
        s.sensors_open.insert((0, 1));
        s.sensor_style_open.insert((0, 1));
        let _ = update(&mut s, Message::RemoveSensorPressed(0, 0));
        assert!(matches!(s.modal, Some(Modal::ConfirmRemoveSensor(0, 0))));
        let _ = update(&mut s, Message::ModalConfirm);
        assert_eq!(s.config.sections[0].sensors.len(), 1);
        assert!(s.sensors_open.contains(&(0, 0)));
        assert!(s.sensor_style_open.contains(&(0, 0)));
    }

    /// The last section is never removable: the button is inert and no
    /// confirmation ever opens.
    #[test]
    fn last_section_is_never_removed() {
        let mut s = test_state();
        s.config.sections.truncate(1);
        let _ = update(&mut s, Message::RemoveSectionPressed(0));
        assert!(s.modal.is_none(), "no confirmation for the last section");
    }

    /// Removing a section drops its sensors' keys and shifts the later
    /// sections — both the accordion and the style sets.
    #[test]
    fn remove_section_shifts_later_keys() {
        let mut s = test_state();
        s.sensor_style_open.insert((1, 0));
        let _ = update(&mut s, Message::RemoveSectionPressed(0));
        assert!(matches!(s.modal, Some(Modal::ConfirmRemoveSection(0))));
        let _ = update(&mut s, Message::ModalConfirm);
        assert_eq!(s.config.sections.len(), 1);
        assert_eq!(s.config.sections[0].name.as_deref(), Some("Right"));
        assert!(
            s.sensor_style_open.contains(&(0, 0)),
            "keys follow their section"
        );
    }

    /// Cross-section move: the sensor is carried over to the target's
    /// end and its open blocks follow it there.
    #[test]
    fn move_sensor_to_another_section_preserves_it() {
        let mut s = test_state();
        s.config.sections[0].sensors[0].source_type = "perc_disk".to_string();
        s.sensors_open.insert((0, 0));
        let _ = update(&mut s, Message::SensorMoveTo(0, 0, 1));
        assert!(s.config.sections[0].sensors.is_empty());
        assert_eq!(s.config.sections[1].sensors.len(), 2);
        assert_eq!(s.config.sections[1].sensors[1].source_type, "perc_disk");
        assert!(s.sensors_open.contains(&(1, 1)), "the open body follows");
    }

    /// The accordion toggles flip once per press.
    #[test]
    fn toggle_accordion_flips_once() {
        let mut s = test_state();
        let _ = update(&mut s, Message::ToggleSection(0));
        assert!(s.sections_collapsed.contains(&0));
        let _ = update(&mut s, Message::ToggleSection(0));
        assert!(!s.sections_collapsed.contains(&0));
        let _ = update(&mut s, Message::ToggleSensor(0, 0));
        assert!(s.sensors_open.contains(&(0, 0)));
        let _ = update(&mut s, Message::ToggleSensor(0, 0));
        assert!(!s.sensors_open.contains(&(0, 0)));
    }

    /// The browser filter: case-insensitive substring, sorted, capped.
    #[test]
    fn icon_filter_is_case_insensitive_sorted_and_capped() {
        let names: Vec<String> =
            (0..ICON_GRID_CAP + 50).map(|i| format!("icon-{i:03}")).collect();
        let hits = filter_icon_names(&names, "ICON-0");
        assert!(hits.len() <= ICON_GRID_CAP);
        assert_eq!(hits.first().copied(), Some("icon-000"));
        assert!(filter_icon_names(&names, "no-such-name").is_empty());
        // An empty query shows everything, still capped and sorted.
        let all = filter_icon_names(&names, "");
        assert_eq!(all.len(), ICON_GRID_CAP);
        let mut sorted = all.to_vec();
        sorted.sort_unstable();
        assert_eq!(all, sorted);
        assert_eq!(
            count_icon_matches(&names, "icon"),
            ICON_GRID_CAP + 50,
            "the caption counts past the cap"
        );
    }

    /// The theme walk collects image stems, dedupes them across size
    /// directories and skips non-image files.
    #[test]
    fn scan_theme_dir_collects_image_stems() {
        let root =
            std::env::temp_dir().join(format!("piuma-icons-test-{}-scan", std::process::id()));
        let deep = root.join("scalable").join("apps");
        let sized = root.join("48x48").join("apps");
        fs::create_dir_all(&deep).expect("fixture dirs");
        fs::create_dir_all(&sized).expect("fixture dirs");
        fs::write(deep.join("firefox.svg"), b"<svg/>").expect("svg");
        fs::write(sized.join("firefox.png"), [0u8]).expect("png");
        fs::write(sized.join("index.theme"), b"[Icon Theme]").expect("not an icon");
        let mut names = HashSet::new();
        collect_icon_names(&root, &mut names);
        assert_eq!(names, HashSet::from(["firefox".to_string()]));
        let _ = fs::remove_dir_all(&root);
    }

    /// The index.theme parser reads the unlocalized `Inherits` key and
    /// ignores the localized variants, splitting and trimming the list.
    #[test]
    fn index_theme_inherits_skips_localized_keys() {
        let dir =
            std::env::temp_dir().join(format!("piuma-icons-test-{}-index", std::process::id()));
        fs::create_dir_all(&dir).expect("fixture dir");
        fs::write(
            dir.join("index.theme"),
            "[Icon Theme]\nName=Breeze\nName[fr]=Brisa\nInherits=hicolor, Adwaita\n",
        )
        .expect("index");
        assert_eq!(
            index_theme_inherits(&dir),
            vec!["hicolor".to_string(), "Adwaita".to_string()]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A root's theme names: only dirs carrying an index.theme count,
    /// and the offered name is the directory one — the lookup's key.
    #[test]
    fn theme_names_in_root_need_an_index() {
        let root =
            std::env::temp_dir().join(format!("piuma-icons-test-{}-root", std::process::id()));
        fs::create_dir_all(root.join("breeze")).expect("theme dir");
        fs::write(root.join("breeze").join("index.theme"), b"[Icon Theme]\n").expect("index");
        fs::create_dir_all(root.join("not-a-theme")).expect("plain dir");
        fs::write(root.join("loose.png"), [0u8]).expect("file");
        assert_eq!(theme_names_in_root(&root), vec!["breeze".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    /// The theme options lead with the system-default entry and keep a
    /// hand-typed theme the installed list does not know.
    #[test]
    fn icon_theme_options_keep_the_current_value() {
        let opts = icon_theme_options(Some("piuma-not-installed"));
        assert_eq!(opts[0], SYSTEM_ICON_THEME);
        assert!(opts.contains(&"piuma-not-installed".to_string()));
        assert_eq!(icon_theme_options(None)[0], SYSTEM_ICON_THEME);
    }

    /// The preview resolves with the bar's own rule: configured theme
    /// or system one, font size clamped to u16.
    #[test]
    fn icon_preview_uses_the_bar_theme_and_size() {
        let mut cfg = PiumaConfig::default();
        cfg.font_size = 70_000; // above u16::MAX
        let (theme, size) = icon_theme_and_size(&cfg);
        assert_eq!(size, u16::MAX);
        assert_eq!(theme, icons::icon_theme());
        cfg.icon_theme = Some("Papirus".to_string());
        let (theme, size) = icon_theme_and_size(&cfg);
        assert_eq!(theme, "Papirus");
        assert_eq!(size, u16::MAX);
        assert!(resolved_icon(&cfg, None).is_none(), "no name, no icon");
    }

    /// The browser flow: the modal opens for the sensor, the query
    /// narrows it, picking writes the name and closes the modal.
    #[test]
    fn browse_icons_flow_sets_the_sensor_icon() {
        let mut s = test_state();
        let _ = update(&mut s, Message::BrowseIconsPressed(0, 0));
        assert!(matches!(
            s.modal,
            Some(Modal::BrowseIcons { section_index: 0, sensor_index: 0, .. })
        ));
        let _ = update(&mut s, Message::ModalTextChanged("firefox".to_string()));
        assert!(
            matches!(&s.modal, Some(Modal::BrowseIcons { query, .. }) if query == "firefox"),
            "the query follows the typed text"
        );
        let _ = update(&mut s, Message::IconPicked(0, 0, "firefox".to_string()));
        assert_eq!(s.config.sections[0].sensors[0].icon.as_deref(), Some("firefox"));
        assert!(s.modal.is_none());
    }
}
