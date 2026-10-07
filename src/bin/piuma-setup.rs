//! Piuma Setup - GUI Configuration Tool
//! A visual tool to configure Piuma panel settings using iced 0.14

use iced::widget::{button, column, container, scrollable, stack, text, toggler, Button, Column, Container, Row, TextInput, Slider, PickList};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Task, Theme};
use iced::widget::text::Style as TextStyle;
use serde_json::{Map, Value as JsonValue};
use std::borrow::Borrow;
use std::collections::HashSet;
use std::fs;
use std::ops::RangeInclusive;
use std::path::PathBuf;

// piuma's real config schema (same package, different bin target): the
// save path validates against it before writing, so a config piuma
// would reject is reported here instead of failing silently at load.
#[path = "../config.rs"]
mod config;

// Configuration structure for GUI - mirrors src/config.rs
#[derive(Debug, Clone)]
pub struct PiumaConfig {
    pub font: Option<String>,
    pub font_size: u32,
    /// Bold panel text (roadmap M2): `None` = key absent (the bar reads
    /// false) — written only when set, so a save never injects the key.
    pub bold: Option<bool>,
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
    SectionAdded,
    SectionRemoved(usize),
    /// Moves a section one position up/down in the panel order.
    SectionMoveUp(usize),
    SectionMoveDown(usize),
    SectionNameChanged(usize, String),
    SectionLayoutChangedIndex(usize, String),
    SectionWeightChanged(usize, u32),
    SectionBgColorChanged(usize, String),
    SectionTextColorChanged(usize, String),
    SensorAdded(usize),
    SensorRemoved(usize, usize),
    /// Moves a sensor one position up/down inside its section.
    SensorMoveUp(usize, usize),
    SensorMoveDown(usize, usize),
    SensorNameChanged(usize, usize, String),
    SensorDescriptionChanged(usize, usize, String),
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
    /// Toggles the collapsible "Sensor Defaults" card.
    ToggleDefaults,
    /// Toggles the collapsible "Section Defaults" card.
    ToggleSecDefaults,
    /// Toggles the collapsible style block of one sensor: (section, sensor).
    ToggleSensorStyle(usize, usize),
    /// Save: asks confirmation when the config was modified, then writes.
    SaveConfig,
    /// Opens the "save as" modal (write the config under a new name).
    SaveAs,
    /// Opens the modal to pick and load any *.json config of the directory.
    LoadConfig,
    /// Text typed in the active modal (file name / picked file).
    ModalTextChanged(String),
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
pub struct PiumaSetup {
    config: PiumaConfig,
    status_message: Option<String>,
    status_is_error: bool,
    /// Whether the collapsible "Sensor Defaults" card is open.
    defaults_expanded: bool,
    /// Whether the collapsible "Section Defaults" card is open.
    sec_defaults_expanded: bool,
    /// Indices of the sensors whose "Style overrides" block is open.
    sensor_style_open: HashSet<(usize, usize)>,
    /// Config file currently edited (name inside the piuma config directory).
    current_file: String,
    /// Serialized config at the last load/save: differs = unsaved changes.
    snapshot: String,
    /// Modal dialog stacked over the view, when one is open.
    modal: Option<Modal>,
}

impl PiumaSetup {
    /// Builds the initial state, loading the config in use (piuma.json) when
    /// present so the user starts from the real configuration, not from defaults.
    fn new() -> Self {
        let mut state = PiumaSetup {
            config: PiumaConfig::default(),
            status_message: None,
            status_is_error: false,
            defaults_expanded: false,
            sec_defaults_expanded: false,
            sensor_style_open: HashSet::new(),
            current_file: DEFAULT_FILE.to_string(),
            snapshot: String::new(),
            modal: None,
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
            "clock" => {
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
    fn swap_sensors(&mut self, section_index: usize, sensor_index: usize) {
        let (a, b) = (sensor_index, sensor_index + 1);
        let flags = &mut self.sensor_style_open;
        let open_a = flags.contains(&(section_index, a));
        let open_b = flags.contains(&(section_index, b));
        if open_a { flags.insert((section_index, b)); } else { flags.remove(&(section_index, b)); }
        if open_b { flags.insert((section_index, a)); } else { flags.remove(&(section_index, a)); }
        self.config.sections[section_index].sensors.swap(a, b);
    }

    /// Swaps two sections, remapping the sensors' style flags of both so
    /// the expanded blocks stay attached to their sensors.
    fn swap_sections(&mut self, a: usize, b: usize) {
        self.config.sections.swap(a, b);
        self.sensor_style_open = self
            .sensor_style_open
            .iter()
            .map(|&(s, i)| if s == a { (b, i) } else if s == b { (a, i) } else { (s, i) })
            .collect();
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
        Message::SectionAdded => {
            state.config.sections.push(Section::default());
        }
        Message::SectionRemoved(index) => {
            if state.config.sections.len() > 1 {
                state.config.sections.remove(index);
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
        Message::SensorAdded(section_index) => {
            if section_index < state.config.sections.len() {
                state.config.sections[section_index].sensors.push(Sensor::default());
            }
        }
        Message::SensorRemoved(section_index, sensor_index) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors.remove(sensor_index);
                }
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
            _ => {}
        },
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
        Message::ResetConfig => {
            state.config = PiumaConfig::default();
            state.set_status("Configuration reset to defaults".to_string(), false);
        }
        Message::Status(_) => {}
    }
    Task::none()
}

fn theme(_state: &PiumaSetup) -> Theme {
    Theme::Dark
}

// ---------------------------------------------------------------------------
// View helpers: cards, aligned form rows and collapsible blocks
// ---------------------------------------------------------------------------

/// Fixed label column width so every form row lines up and labels never
/// overlap their controls (`Length` only accepts u32).
const LABEL_WIDTH: u32 = 110;

/// Body text size: the iced default (16 pt) reduced by 2 pt as requested.
const BODY: u32 = 14;

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

/// Shared card style: subtle background, thin border, rounded corners.
fn card_style(bg: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_t: &Theme| container::Style {
        background: Some(Background::Color(bg)),
        border: Border { width: 1.0, color: Color::from_rgb8(0x38, 0x38, 0x3e), radius: radius.into() },
        ..container::Style::default()
    }
}

/// The visual unit of the nested layout (panel > section > sensor).
fn card<'a>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content.into())
        .padding(12)
        .width(Length::Fill)
        .style(card_style(Color::from_rgb8(0x26, 0x26, 0x2b), 6.0))
}

/// Inset background marking content as a child of the surrounding card.
fn inset_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x1e, 0x1e, 0x22))),
        border: Border { width: 1.0, color: Color::from_rgb8(0x33, 0x33, 0x39), radius: 4.0.into() },
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
fn move_btn<'a>(label: &'a str, enabled: bool, on_press: Message) -> Button<'a, Message> {
    let b = button(text(label).size(BODY)).padding(5).style(button::secondary);
    if enabled { b.on_press(on_press) } else { b }
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
fn collapsible_header<'a>(title: &'a str, summary: String, expanded: bool, on_toggle: Message) -> Element<'a, Message> {
    button(
        Row::new()
            .push(text(if expanded { "▾" } else { "▸" }).size(BODY).width(14))
            .push(text(title).size(13))
            .push(text(summary).size(10)
                .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.55, 0.55, 0.6)), ..TextStyle::default() }))
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(6)
    .style(button::secondary)
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
fn view(state: &PiumaSetup) -> Element<Message> {
    let content = column![
        config_header(state),
        panel_settings(state),
        notifications_settings(state),
        sensor_defaults_view(state),
        section_defaults_view(state),
        sections_list(state),
    ]
    .padding(20)
    .spacing(15)
    .width(Length::Fill);

    let base = container(
        column![
            container(scrollable(content).width(Length::Fill).height(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill),
            footer(state),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_state| container::Style {
        background: Some(Background::Color(Color::from_rgb(0.12, 0.12, 0.12))),
        ..container::Style::default()
    });

    match &state.modal {
        Some(_) => stack![base, modal_overlay(state)].into(),
        None => base.into(),
    }
}

/// Fixed bottom bar: edited file + status on the left, actions on the right.
fn footer(state: &PiumaSetup) -> Element<Message> {
    let info = column![
        text(format!("editing: {}", state.current_file)).size(BODY)
            .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.5, 0.5, 0.55)), ..TextStyle::default() }),
        status_bar(state),
    ]
    .spacing(2);

    let bar = Row::new()
        .push(container(info).width(Length::Fill))
        .push(action_buttons())
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    container(bar)
        .padding(Padding { top: 10.0, right: 20.0, bottom: 10.0, left: 20.0 })
        .width(Length::Fill)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0x1a, 0x1a, 0x1d))),
            border: Border { width: 1.0, color: Color::from_rgb8(0x38, 0x38, 0x3e), radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}

/// Dimmed full-window layer with the active modal card centered on top.
fn modal_overlay(state: &PiumaSetup) -> Element<Message> {
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
fn modal_card(state: &PiumaSetup) -> Element<Message> {
    // Errors are rendered inside the modal so they are visible over the overlay.
    let error = if state.status_is_error { state.status_message.clone() } else { None };

    match &state.modal {
        Some(Modal::ConfirmSave) => dialog_frame(
            "Unsaved changes",
            text(format!("Overwrite {} with the current configuration?", state.current_file)).size(BODY).into(),
            error,
            dialog_actions(Message::ModalConfirm, "Overwrite"),
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
            dialog_frame("Save as", body.into(), error, dialog_actions(Message::ModalConfirm, "Save"))
        }
        Some(Modal::ConfirmSaveAs(file)) => dialog_frame(
            "File exists",
            text(format!("The file {} already exists. Overwrite it?", file)).size(BODY).into(),
            error,
            dialog_actions(Message::ModalConfirm, "Overwrite"),
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
                        .style(if is_selected { button::primary as fn(&Theme, button::Status) -> button::Style } else { button::secondary as fn(&Theme, button::Status) -> button::Style }),
                );
            }
            let body: Element<Message> = if files.is_empty() {
                text("No .json files found in the config directory").size(BODY).into()
            } else {
                list.into()
            };
            dialog_frame("Load configuration", body, error, dialog_actions(Message::ModalConfirm, "Load"))
        }
        None => text("").into(),
    }
}

/// Modal card frame: title, body, optional error hint, action buttons.
fn dialog_frame<'a>(
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
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0x26, 0x26, 0x2b))),
            border: Border { width: 1.0, color: Color::from_rgb8(0x4a, 0x4a, 0x52), radius: 8.0.into() },
            ..container::Style::default()
        })
        .into()
}

/// Cancel + primary action button row shared by all modals.
fn dialog_actions<'a>(action: Message, label: &'a str) -> Row<'a, Message> {
    Row::new()
        .push(button(text("Cancel").size(BODY)).on_press(Message::ModalCancel).padding(8).style(button::secondary))
        .push(button(text(label).size(BODY)).on_press(action).padding(8).style(button::primary))
        .spacing(10)
        .align_y(Alignment::Center)
}

fn config_header(_state: &PiumaSetup) -> Element<Message> {
    let title = text("Piuma Configuration Setup").size(22).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() });
    let subtitle = text("Configure your Piuma status bar").size(14).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.6, 0.6, 0.6)), ..TextStyle::default() });

    column![text(""), title, subtitle, text("")]
        .spacing(10)
        .align_x(Alignment::Center)
        .into()
}

/// Panel-wide settings as a two-column form grid inside a card.
fn panel_settings(state: &PiumaSetup) -> Element<Message> {
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
        column![
            text("Panel Settings").size(16).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() }),
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
                    .style(|_t: &Theme| TextStyle {
                        color: Some(Color::from_rgb(0.55, 0.55, 0.6)),
                        ..TextStyle::default()
                    }),
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
                    .style(|_t: &Theme| TextStyle {
                        color: Some(Color::from_rgb(0.55, 0.55, 0.6)),
                        ..TextStyle::default()
                    }),
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
        column![
            text("Desktop Notifications").size(16).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() }),
            body,
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// The "Sections" block: one card per section, plus the add button.
fn sections_list(state: &PiumaSetup) -> Element<Message> {
    let mut section_list = Column::new().spacing(15);
    for (i, section) in state.config.sections.iter().enumerate() {
        section_list = section_list.push(section_view(state, i, section));
    }

    let add_section = button(text("+ Add Section").size(BODY))
        .on_press(Message::SectionAdded)
        .padding(10);

    column![
        text("Sections").size(16).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() }),
        section_list,
        add_section,
    ]
    .spacing(10)
    .into()
}

/// Section card: header with name + remove, settings grid, then the sensor
/// cards nested on an inset background to mark the hierarchy.
fn section_view<'a>(state: &'a PiumaSetup, index: usize, section: &'a Section) -> Element<'a, Message> {
    let header = Row::new()
        .push(text("SECTION").size(9)
            .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.5, 0.5, 0.55)), ..TextStyle::default() }))
        .push(input("Section name", section.name.as_deref().unwrap_or(""), move |value| Message::SectionNameChanged(index, value))
            .width(Length::Fill))
        .push(move_btn("▲", index > 0, Message::SectionMoveUp(index)))
        .push(move_btn("▼", index + 1 < state.config.sections.len(), Message::SectionMoveDown(index)))
        .push(button(text("Remove").size(BODY)).on_press(Message::SectionRemoved(index)).padding(5))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

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
        let style_open = state.sensor_style_open.contains(&(index, j));
        sensor_list = sensor_list.push(sensor_view(index, j, section.sensors.len(), sensor, style_open));
    }
    sensor_list = sensor_list.push(
        button(text("+ Add Sensor").size(BODY)).on_press(Message::SensorAdded(index)).padding(6),
    );
    let sensors = container(sensor_list).padding(10).width(Length::Fill).style(inset_style);

    card(column![header, settings, sensors].spacing(10).width(Length::Fill)).into()
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
/// prefixed categories first (perc_/used_/total_/detail_/temp_), plain
/// lowercase (no numeric category) last.
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
    "net_state", "uptime", "clock", "window", "workspace",
    "command", "env_var", "static_text", "tray", "notifications",
];

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
        "window" => "Focused window",
        "workspace" => "Workspace",
        "tray" => "System tray",
        "notifications" => "Desktop notifications",
        _ => "Sensor",
    }
}

/// Sensor card: identity header, source, action, then the collapsible
/// "Style overrides" block (closed by default: overrides are optional).
/// `sensor_count` bounds the ▼ arrow (disabled on the last sensor).
fn sensor_view(section_index: usize, sensor_index: usize, sensor_count: usize, sensor: &Sensor, style_open: bool) -> Element<Message> {
    let header = Row::new()
        .push(input("Name", sensor.name.as_deref().unwrap_or(""), move |value| Message::SensorNameChanged(section_index, sensor_index, value))
            .width(Length::Fill))
        .push(input("Description", sensor.description.as_deref().unwrap_or(""), move |value| Message::SensorDescriptionChanged(section_index, sensor_index, value))
            .width(Length::Fill))
        .push(move_btn("▲", sensor_index > 0, Message::SensorMoveUp(section_index, sensor_index)))
        .push(move_btn("▼", sensor_index + 1 < sensor_count, Message::SensorMoveDown(section_index, sensor_index)))
        .push(button(text("Remove").size(BODY)).on_press(Message::SensorRemoved(section_index, sensor_index)).padding(5))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // Options = the canonical types; load_sensor already converted any
    // legacy name, so the current value is in this list unless the file
    // holds an unknown/typo'd type — which stays visible to be fixed.
    let mut source_types: Vec<String> = SOURCE_TYPES.iter().map(|s| s.to_string()).collect();
    if !source_types.iter().any(|t| t == &sensor.source_type) {
        source_types.push(sensor.source_type.clone());
    }
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
        "clock" => input("Format", sensor.source_config.format.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "format".to_string(), value))
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

    let action = form_cell("Action", input("close_panel / command / ...", sensor.action.as_deref().unwrap_or(""), move |value| Message::SensorActionChanged(section_index, sensor_index, value))
        .width(Length::Fill)
        .into());

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
        "Style overrides", summary, style_open,
        Message::ToggleSensorStyle(section_index, sensor_index),
    );

    let mut root = column![header, source, action, style_header]
        .spacing(6)
        .width(Length::Fill);

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
        root = root.push(container(form_grid(fields)).padding(10).width(Length::Fill).style(inset_style));
    }

    card(root).into()
}

/// The "Sensor Defaults" card: global style applied to every sensor,
/// collapsed by default because it only overrides group-wide values.
fn sensor_defaults_view(state: &PiumaSetup) -> Element<Message> {
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
    let header = collapsible_header("Sensor Defaults", summary, state.defaults_expanded, Message::ToggleDefaults);

    let mut root = column![header].spacing(10).width(Length::Fill);
    if state.defaults_expanded {
        root = root.push(text("Style applied to every sensor; single sensors override these values.")
            .size(10)
            .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.55, 0.55, 0.6)), ..TextStyle::default() }));

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

    card(root).into()
}

/// The "Section Defaults" card: style applied to every section, overridden
/// per section (same group rule as the sensor defaults).
fn section_defaults_view(state: &PiumaSetup) -> Element<Message> {
    let d = &state.config.section_defaults;

    let summary = style_summary(
        &[
            ("bg", &d.bg_color), ("text", &d.text_color),
            ("border", &d.border_color), ("border-type", &d.border_type),
        ],
        &[("brd", &d.border_width), ("rad", &d.border_radius)],
    );
    let header = collapsible_header("Section Defaults", summary, state.sec_defaults_expanded, Message::ToggleSecDefaults);

    let mut root = column![header].spacing(10).width(Length::Fill);
    if state.sec_defaults_expanded {
        root = root.push(text("Style applied to every section; single sections override these values.")
            .size(10)
            .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.55, 0.55, 0.6)), ..TextStyle::default() }));

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

    card(root).into()
}

/// Load / Save / Save As / Reset actions for the fixed bottom bar.
fn action_buttons() -> Element<'static, Message> {
    let load_btn = button(text("Load…").size(BODY)).on_press(Message::LoadConfig).padding(10).style(button::secondary);
    let save_btn = button(text("Save").size(BODY)).on_press(Message::SaveConfig).padding(10).style(button::primary);
    let save_as_btn = button(text("Save As…").size(BODY)).on_press(Message::SaveAs).padding(10).style(button::secondary);
    let reset_btn = button(text("Reset").size(BODY)).on_press(Message::ResetConfig).padding(10);

    Row::new()
        .push(load_btn)
        .push(save_btn)
        .push(save_as_btn)
        .push(reset_btn)
        .spacing(10)
        .into()
}

fn status_bar(state: &PiumaSetup) -> Element<Message> {
    match &state.status_message {
        Some(msg) => {
            let is_error = state.status_is_error;
            text(msg).size(10).style(move |_t: &Theme| TextStyle { color: Some(if is_error { Color::from_rgb(0.9, 0.3, 0.3) } else { Color::from_rgb(0.3, 0.8, 0.4) }), ..TextStyle::default() }).into()
        }
        None => text("").into(),
    }
}

fn app() -> iced::Result {
    iced::application(PiumaSetup::new, update, view)
        .title("Piuma Setup")
        .theme(theme)
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
            status_message: None,
            status_is_error: false,
            defaults_expanded: false,
            sec_defaults_expanded: false,
            sensor_style_open: HashSet::new(),
            current_file: "piuma.json".to_string(),
            snapshot: String::new(),
            modal: None,
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
}
