//! The editor's own mirror of `piuma.json`: plain structs with defaults,
//! the serializer/deserializer keeping the file's exact shape (absent
//! keys stay absent) and the source-type name mapping shared by the
//! pickers, the labels and the save path.
use serde_json::{Map, Value as JsonValue};

use crate::config;
use crate::controls::sorted_rules;
use crate::state::PiumaSetup;


/// The `panel.wallpaper` block (roadmap2 #1): whether the bar draws the
/// desktop background itself and which image covers the output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WallpaperCfg {
    pub enabled: bool,
    pub path: String,
}

/// The `panel.screen_corners` block (roadmap angoli-schermo): the
/// rounded-corner masks drawn over every screen corner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCornersCfg {
    pub enable: bool,
    pub color: String,
    pub radius: u32,
}

impl Default for ScreenCornersCfg {
    fn default() -> Self {
        // The first edit's defaults = what the bar falls back to when a
        // field is missing (config.rs): on, black, 32 px.
        ScreenCornersCfg { enable: true, color: "#000000".to_string(), radius: 32 }
    }
}

/// Applies one screen-corners edit, creating the block with its
/// defaults on first use — the bar reads an absent key as "off", so the
/// block only ever appears after the user touched the feature.
pub(crate) fn corners_set(config: &mut PiumaConfig, edit: impl FnOnce(&mut ScreenCornersCfg)) {
    edit(config.screen_corners.get_or_insert_with(ScreenCornersCfg::default));
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
    /// Draw each section's title above its sensors (bar default: true).
    pub section_title: Option<bool>,
    /// Desktop background the bar draws itself (roadmap2 #1): `None` =
    /// key absent (the bar opens no background window).
    pub wallpaper: Option<WallpaperCfg>,
    /// Bar colours derived from the wallpaper at runtime: `None` = key
    /// absent. Only this switch is persisted — the extracted tones
    /// never reach the file (they live in the bar's memory).
    pub wallpaper_palette: Option<bool>,
    /// Rounded screen corners (roadmap angoli-schermo): `None` = key
    /// absent (the bar draws no masks).
    pub screen_corners: Option<ScreenCornersCfg>,
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
    /// Threshold colours for the displayed value (`color_rules`), the
    /// highest `min` the reading reaches wins on the bar; `None` = the
    /// sensor's plain `text_color` everywhere (mirrors src/config.rs).
    pub color_rules: Option<Vec<config::ColorRule>>,
}

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub command: Option<String>,
    pub env_var: Option<String>,
    pub static_text: Option<String>,
    pub interface: Option<String>,
    pub device: Option<String>,
    pub format: Option<String>,
    /// The `layout` sensor's layout file name (piuma defaults it to
    /// "default" when absent).
    pub name: Option<String>,
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
            wallpaper_palette: None,
            screen_corners: None,
            section_layout: "horizontal".to_string(),
            section_title: None,
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
            color_rules: None,
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
            name: None,
        }
    }
}


impl PiumaSetup {
    pub(crate) fn config_to_json(&self) -> JsonValue {
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
        if let Some(t) = self.config.section_title {
            panel.insert("section_title".to_string(), JsonValue::Bool(t));
        }
        // Only the toggle travels to the file: the palette it selects is
        // derived at runtime and never serialized.
        if let Some(p) = self.config.wallpaper_palette {
            panel.insert("wallpaper_palette".to_string(), JsonValue::Bool(p));
        }
        // The wallpaper block is written whole (present = configured,
        // absent = untouched, so a save never injects the key).
        if let Some(w) = &self.config.wallpaper {
            let mut wp = Map::new();
            wp.insert("enabled".to_string(), JsonValue::Bool(w.enabled));
            wp.insert("path".to_string(), JsonValue::String(w.path.clone()));
            panel.insert("wallpaper".to_string(), JsonValue::Object(wp));
        }
        // The corners block follows the same rule (roadmap angoli).
        if let Some(c) = &self.config.screen_corners {
            let mut sc = Map::new();
            sc.insert("enable".to_string(), JsonValue::Bool(c.enable));
            sc.insert("color".to_string(), JsonValue::String(c.color.clone()));
            sc.insert("radius".to_string(), JsonValue::Number(c.radius.into()));
            panel.insert("screen_corners".to_string(), JsonValue::Object(sc));
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
            "layout" => {
                if let Some(name) = &sensor.source_config.name {
                    map.insert("name".to_string(), JsonValue::String(name.clone()));
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

        // Threshold colour rules, stored sorted by ascending `min` (the
        // order the bar evaluates them); absent/empty = key omitted, like
        // every other optional style field.
        if let Some(rules) = &sensor.color_rules {
            let array: Vec<JsonValue> = sorted_rules(rules.clone())
                .into_iter()
                .filter_map(|r| {
                    let mut obj = Map::new();
                    // from_f64 fails on NaN/inf, which no JSON file can
                    // carry either — such a rule is dropped, never written.
                    obj.insert("min".to_string(), JsonValue::Number(serde_json::Number::from_f64(r.min)?));
                    obj.insert("color".to_string(), JsonValue::String(r.color));
                    Some(JsonValue::Object(obj))
                })
                .collect();
            if !array.is_empty() {
                map.insert("color_rules".to_string(), JsonValue::Array(array));
            }
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
    pub(crate) fn serialize(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.config_to_json())
            .map_err(|e| format!("Error serializing config: {}", e))
    }
}

impl PiumaSetup {
    pub(crate) fn load_from_json(&mut self, content: &str) -> Result<(), String> {
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
            wallpaper_palette: None,
            screen_corners: None,
            section_layout: "horizontal".to_string(),
            section_title: None,
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
        // Absent `section_title` stays `None` (the bar draws titles by
        // default); an explicit false is kept as-is.
        self.config.section_title = panel.get("section_title").and_then(|v| v.as_bool());
        // Absent `wallpaper` stays `None`: the bar draws no background
        // (roadmap2 #1). A partial block falls back per field.
        self.config.wallpaper = panel.get("wallpaper").and_then(|v| {
            v.as_object().map(|o| WallpaperCfg {
                enabled: o.get("enabled").and_then(JsonValue::as_bool).unwrap_or(false),
                path: o.get("path").and_then(JsonValue::as_str).unwrap_or_default().to_string(),
            })
        });
        // Absent `screen_corners` stays `None`: the bar draws no corner
        // masks (roadmap angoli-schermo). A partial block falls back per
        // field, exactly like the bar's own serde defaults.
        self.config.screen_corners = panel.get("screen_corners").and_then(|v| {
            v.as_object().map(|o| ScreenCornersCfg {
                enable: o.get("enable").and_then(JsonValue::as_bool).unwrap_or(false),
                color: o.get("color").and_then(JsonValue::as_str).unwrap_or("#000000").to_string(),
                radius: o.get("radius").and_then(JsonValue::as_u64).unwrap_or(32) as u32,
            })
        });
        // Absent `wallpaper_palette` stays `None` (reset default): the
        // bar reads false. Only this toggle lives in the file — the
        // extracted tones are runtime-only and never serialized.
        self.config.wallpaper_palette = panel.get("wallpaper_palette").and_then(|v| v.as_bool());
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
            if let Some(n) = source.get("name").and_then(|v| v.as_str()) {
                sensor.source_config.name = Some(n.to_string());
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

        // Threshold colour rules: loaded like `action`, otherwise the next
        // save would drop the key. Entries a rule cannot be made of (wrong
        // types, missing fields) are skipped — piuma ignores them too, and
        // the pre-save schema check still rejects a structurally broken file.
        sensor.color_rules = obj.get("color_rules").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let rule = item.as_object()?;
                    Some(config::ColorRule {
                        min: rule.get("min")?.as_f64()?,
                        color: rule.get("color")?.as_str()?.to_string(),
                    })
                })
                .collect()
        });

        Some(sensor)
    }
}

/// Apply one string style field of `panel.notifications` by JSON key:
/// the single place every style control of the card writes through
/// (unknown keys are ignored).
pub(crate) fn set_notif_style(n: &mut NotificationsCfg, key: &str, value: &str) {
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
pub(crate) fn set_notif_number(n: &mut NotificationsCfg, key: &str, value: i32) {
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

/// Legacy PascalCase source type → canonical prefixed/lowercase name, so
/// labels and inputs work for old configs too (the stored value itself is
/// never rewritten on save).
/// Maps a legacy type name to the canonical round-9 name; anything else
/// (already canonical) passes through. Applied when loading, so the whole
/// editor — picklist, param inputs, save — only ever sees canonical names.
pub(crate) fn canonical_type<'a>(source_type: &'a str) -> &'a str {
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
pub(crate) const SOURCE_TYPES: &[&str] = &[
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
    "taskbar", "layout",
];

/// The sensor-type combo's options (roadmap2 #6): every canonical type
/// in alphabetical order; an unknown type from the edited file joins
/// the list (also sorted) so the selection stays visible.
pub(crate) fn available_source_types(current: &str) -> Vec<String> {
    let mut types: Vec<String> = SOURCE_TYPES.iter().map(|s| s.to_string()).collect();
    if !types.iter().any(|t| t == current) {
        types.push(current.to_string());
    }
    types.sort();
    types
}

/// Friendly label for a canonical source type (shown when it needs no
/// extra configuration).
pub(crate) fn source_label(canonical: &str) -> &'static str {
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
        "taskbar" => "Window taskbar",
        "layout" => "Window layout",
        "command" => "Shell command",
        "env_var" => "Environment variable",
        "static_text" => "Static text",
        _ => "Sensor",
    }
}

/// The layout names already saved by the bar (`~/.config/piuma/
/// layouts/*.json`, sorted): the `layout` sensor's picklist options.
/// Only well-named files count — an interrupted save's temp file and a
/// hand-dropped oddity never become selectable layouts.
pub(crate) fn saved_layout_names() -> Vec<String> {
    let Some(dir) = dirs::config_dir().map(|d| d.join("piuma").join("layouts")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|f| {
                    let fname = f.file_name().to_string_lossy().to_string();
                    let name = fname.strip_suffix(".json")?;
                    config::is_valid_layout_name(name).then(|| name.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}
