//! Piuma Setup - GUI Configuration Tool
//! A visual tool to configure Piuma panel settings using iced 0.14

use iced::widget::{button, column, container, scrollable, text, Column, Row, TextInput, Slider, PickList};
use iced::{Alignment, Background, Color, Element, Length, Task, Theme};
use iced::widget::text::Style as TextStyle;
use serde_json::{Map, Value as JsonValue};
use std::fs;

// Configuration structure for GUI - mirrors src/config.rs
#[derive(Debug, Clone)]
pub struct PiumaConfig {
    pub font: Option<String>,
    pub font_size: u32,
    pub section_layout: String,
    pub default_section_weight: u32,
    pub default_bg_color: Option<String>,
    pub default_text_color: Option<String>,
    pub anchor: Option<String>,
    pub align: Option<String>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: Option<String>,
    pub layout: String,
    pub weight: Option<u32>,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
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
    pub border_width: Option<u32>,
    pub border_width_hover: Option<u32>,
    pub border_radius: Option<u32>,
    pub padding: Option<u32>,
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
            section_layout: "horizontal".to_string(),
            default_section_weight: 1,
            default_bg_color: Some("#1a1b26".to_string()),
            default_text_color: Some("#e8e8e8".to_string()),
            anchor: Some("top".to_string()),
            align: Some("center".to_string()),
            sections: vec![Section::default()],
        }
    }
}

impl Default for Section {
    fn default() -> Self {
        Section {
            name: Some("System".to_string()),
            layout: "horizontal".to_string(),
            weight: None,
            bg_color: Some("#2d2d3a".to_string()),
            text_color: Some("#ff6b6b".to_string()),
            sensors: vec![Sensor::default()],
        }
    }
}

impl Default for Sensor {
    fn default() -> Self {
        Sensor {
            name: Some("CPU".to_string()),
            description: None,
            source_type: "ProcCpu".to_string(),
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
    SectionLayoutChanged(String),
    DefaultSectionWeightChanged(u32),
    DefaultBgColorChanged(String),
    DefaultTextColorChanged(String),
    AnchorChanged(String),
    AlignChanged(String),
    SectionAdded,
    SectionRemoved(usize),
    SectionNameChanged(usize, String),
    SectionLayoutChangedIndex(usize, String),
    SectionWeightChanged(usize, u32),
    SectionBgColorChanged(usize, String),
    SectionTextColorChanged(usize, String),
    SensorAdded(usize),
    SensorRemoved(usize, usize),
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
    SensorBorderWidthChanged(usize, usize, u32),
    SensorBorderWidthHoverChanged(usize, usize, u32),
    SensorBorderRadiusChanged(usize, usize, u32),
    SensorPaddingChanged(usize, usize, u32),
    SensorActionChanged(usize, usize, String),
    SaveConfig,
    LoadConfig,
    ResetConfig,
    Status(String),
}

// Application state
pub struct PiumaSetup {
    config: PiumaConfig,
    status_message: Option<String>,
    status_is_error: bool,
}

impl PiumaSetup {
    fn new() -> Self {
        PiumaSetup {
            config: PiumaConfig::default(),
            status_message: None,
            status_is_error: false,
        }
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
            "Network" => {
                if let Some(iface) = &sensor.source_config.interface {
                    map.insert("interface".to_string(), JsonValue::String(iface.clone()));
                }
            }
            "Audio" => {
                if let Some(device) = &sensor.source_config.device {
                    map.insert("device".to_string(), JsonValue::String(device.clone()));
                }
            }
            "Clock" => {
                if let Some(format) = &sensor.source_config.format {
                    map.insert("format".to_string(), JsonValue::String(format.clone()));
                }
            }
            "Command" => {
                if let Some(cmd) = &sensor.source_config.command {
                    map.insert("command".to_string(), JsonValue::String(cmd.clone()));
                }
            }
            "EnvVar" => {
                if let Some(ev) = &sensor.source_config.env_var {
                    map.insert("env_var".to_string(), JsonValue::String(ev.clone()));
                }
            }
            "StaticText" => {
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
        map.insert("layout".to_string(), JsonValue::String(section.layout.clone()));
        if let Some(w) = section.weight {
            map.insert("weight".to_string(), JsonValue::Number(w.into()));
        }
        if let Some(bg) = &section.bg_color {
            map.insert("bg_color".to_string(), JsonValue::String(bg.clone()));
        }
        if let Some(tc) = &section.text_color {
            map.insert("text_color".to_string(), JsonValue::String(tc.clone()));
        }
        let sensors_vec: Vec<JsonValue> = section.sensors.iter().map(|s| self.sensor_to_json(s)).collect();
        map.insert("sensors".to_string(), JsonValue::Array(sensors_vec));
        JsonValue::Object(map)
    }

    fn save_config(&mut self) -> Task<Message> {
        let json = self.config_to_json();
        let path = dirs::config_dir()
            .map(|d| d.join("piuma").join("piuma.json"))
            .unwrap_or_else(|| std::path::PathBuf::from("./piuma.json"));

        match fs::create_dir_all(path.parent().unwrap()) {
            Ok(_) => match fs::write(&path, serde_json::to_string_pretty(&json).unwrap()) {
                Ok(_) => {
                    self.set_status(format!("Configuration saved to {:?}", path), false);
                    Task::none()
                }
                Err(e) => {
                    self.set_status(format!("Error saving: {}", e), true);
                    Task::none()
                }
            },
            Err(e) => {
                self.set_status(format!("Error creating directory: {}", e), true);
                Task::none()
            }
        }
    }

    fn load_config(&mut self) -> Task<Message> {
        let path = dirs::config_dir()
            .map(|d| d.join("piuma").join("piuma.json"))
            .unwrap_or_else(|| std::path::PathBuf::from("./piuma.json"));

        match fs::read_to_string(&path) {
            Ok(content) => match self.load_from_json(&content) {
                Ok(_) => {
                    self.set_status(format!("Configuration loaded from {:?}", path), false);
                    Task::none()
                }
                Err(e) => {
                    self.set_status(format!("Error parsing config: {}", e), true);
                    Task::none()
                }
            },
            Err(_) => {
                self.set_status(format!("No config file found at {:?}", path), true);
                Task::none()
            }
        }
    }

    fn load_from_json(&mut self, content: &str) -> Result<(), String> {
        let json: JsonValue = serde_json::from_str(content).map_err(|e| e.to_string())?;
        let panel = json.get("panel").ok_or("Missing panel")?;

        if let Some(font) = panel.get("font").and_then(|v| v.as_str()) {
            self.config.font = Some(font.to_string());
        }
        if let Some(fs_) = panel.get("font_size").and_then(|v| v.as_u64()) {
            self.config.font_size = fs_ as u32;
        }
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

        if let Some(sections) = panel.get("sections").and_then(|v| v.as_array()) {
            self.config.sections = sections.iter().filter_map(|s| self.load_section(s)).collect();
        }

        Ok(())
    }

    fn load_section(&self, json: &JsonValue) -> Option<Section> {
        let obj = json.as_object()?;
        let mut section = Section::default();

        if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
            section.name = Some(name.to_string());
        }
        if let Some(layout) = obj.get("layout").and_then(|v| v.as_str()) {
            section.layout = layout.to_string();
        }
        if let Some(w) = obj.get("weight").and_then(|v| v.as_u64()) {
            section.weight = Some(w as u32);
        }
        if let Some(bg) = obj.get("bg_color").and_then(|v| v.as_str()) {
            section.bg_color = Some(bg.to_string());
        }
        if let Some(tc) = obj.get("text_color").and_then(|v| v.as_str()) {
            section.text_color = Some(tc.to_string());
        }
        if let Some(sensors) = obj.get("sensors").and_then(|v| v.as_array()) {
            section.sensors = sensors.iter().filter_map(|s| self.load_sensor(s)).collect();
        }

        Some(section)
    }

    fn load_sensor(&self, json: &JsonValue) -> Option<Sensor> {
        let obj = json.as_object()?;
        let mut sensor = Sensor::default();

        if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
            sensor.name = Some(name.to_string());
        }
        if let Some(desc) = obj.get("description").and_then(|v| v.as_str()) {
            sensor.description = Some(desc.to_string());
        }
        if let Some(source) = obj.get("source").and_then(|v| v.as_object()) {
            if let Some(type_) = source.get("type").and_then(|v| v.as_str()) {
                sensor.source_type = type_.to_string();
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
            if let Some(st) = source.get("text").and_then(|v| v.as_str()) {
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
            if let Some(val) = obj.get(key).and_then(|v| v.as_u64()) {
                *field = Some(val as u32);
            }
        }

        Some(sensor)
    }
}

fn update(state: &mut PiumaSetup, message: Message) -> Task<Message> {
    match message {
        Message::FontChanged(value) => state.config.font = Some(value),
        Message::FontSizeChanged(value) => state.config.font_size = value,
        Message::SectionLayoutChanged(value) => state.config.section_layout = value,
        Message::DefaultSectionWeightChanged(value) => state.config.default_section_weight = value,
        Message::DefaultBgColorChanged(value) => state.config.default_bg_color = if value.is_empty() { None } else { Some(value) },
        Message::DefaultTextColorChanged(value) => state.config.default_text_color = if value.is_empty() { None } else { Some(value) },
        Message::AnchorChanged(value) => state.config.anchor = if value.is_empty() { None } else { Some(value) },
        Message::AlignChanged(value) => state.config.align = if value.is_empty() { None } else { Some(value) },
        Message::SectionAdded => {
            state.config.sections.push(Section::default());
        }
        Message::SectionRemoved(index) => {
            if state.config.sections.len() > 1 {
                state.config.sections.remove(index);
            }
        }
        Message::SectionNameChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].name = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::SectionLayoutChangedIndex(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].layout = value;
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
        Message::SaveConfig => return state.save_config(),
        Message::LoadConfig => return state.load_config(),
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

fn view(state: &PiumaSetup) -> Element<Message> {
    let content = column![
        config_header(state),
        panel_settings(state),
        sections_list(state),
        action_buttons(),
        status_bar(state),
    ]
    .padding(20)
    .spacing(15)
    .width(Length::Fill);

    container(scrollable(content)
        .width(Length::Fill)
        .height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_state| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.12, 0.12, 0.12))),
            ..container::Style::default()
        })
        .into()
}

fn config_header(_state: &PiumaSetup) -> Element<Message> {
    let title = text("Piuma Configuration Setup").size(24).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() });
    let subtitle = text("Configure your Piuma status bar").size(16).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.6, 0.6, 0.6)), ..TextStyle::default() });

    column![text(""), title, subtitle, text("")]
        .spacing(10)
        .align_x(Alignment::Center)
        .into()
}

fn panel_settings(state: &PiumaSetup) -> Element<Message> {
    let font_row = Row::new()
        .push(text("Font:").width(120))
        .push(TextInput::new("Font name", state.config.font.as_deref().unwrap_or(""))
            .on_input(Message::FontChanged)
            .width(200))
        .spacing(10);

    let font_size_row = Row::new()
        .push(text("Font Size:").width(120))
        .push(Slider::new(8..=32, state.config.font_size, Message::FontSizeChanged)
            .width(150))
        .push(text(format!("{}pt", state.config.font_size)).width(50))
        .spacing(10);

    let section_layout_row = Row::new()
        .push(text("Section Layout:").width(120))
        .push(PickList::new(
            vec!["horizontal".to_string(), "vertical".to_string()],
            Some(state.config.section_layout.clone()),
            Message::SectionLayoutChanged,
        ).width(120))
        .spacing(10);

    let default_bg_color_row = Row::new()
        .push(text("Default BG Color:").width(120))
        .push(TextInput::new("BG Color", state.config.default_bg_color.as_deref().unwrap_or(""))
            .on_input(Message::DefaultBgColorChanged)
            .width(100))
        .spacing(10);

    let default_text_color_row = Row::new()
        .push(text("Default Text Color:").width(120))
        .push(TextInput::new("Text Color", state.config.default_text_color.as_deref().unwrap_or(""))
            .on_input(Message::DefaultTextColorChanged)
            .width(100))
        .spacing(10);

    let anchor_row = Row::new()
        .push(text("Anchor:").width(120))
        .push(PickList::new(
            vec!["top".to_string(), "bottom".to_string(), "left".to_string(), "right".to_string(),
                 "top-left".to_string(), "top-right".to_string(), "bottom-left".to_string(), "bottom-right".to_string()],
            Some(state.config.anchor.clone().unwrap_or_default()),
            Message::AnchorChanged,
        ).width(140))
        .spacing(10);

    let align_row = Row::new()
        .push(text("Align:").width(120))
        .push(PickList::new(
            vec!["left".to_string(), "center".to_string(), "right".to_string()],
            Some(state.config.align.clone().unwrap_or_default()),
            Message::AlignChanged,
        ).width(120))
        .spacing(10);

    column![
        text("Panel Settings").size(18).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() }),
        font_row,
        font_size_row,
        section_layout_row,
        default_bg_color_row,
        default_text_color_row,
        anchor_row,
        align_row,
    ]
    .spacing(8)
    .into()
}

fn sections_list(state: &PiumaSetup) -> Element<Message> {
    let mut section_list = Column::new().spacing(15);
    for (i, section) in state.config.sections.iter().enumerate() {
        section_list = section_list.push(section_view(i, section));
    }

    let add_section = button("+ Add Section")
        .on_press(Message::SectionAdded)
        .padding(10);

    column![
        text("Sections").size(18).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.8, 0.8, 0.8)), ..TextStyle::default() }),
        section_list,
        add_section,
    ]
    .spacing(10)
    .into()
}

fn section_view(index: usize, section: &Section) -> Element<Message> {
    let name_input = TextInput::new("Section Name", section.name.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SectionNameChanged(index, value))
        .width(150);

    let layout_picklist = PickList::new(
        vec!["horizontal".to_string(), "vertical".to_string()],
        Some(&section.layout),
        move |value| Message::SectionLayoutChangedIndex(index, value),
    ).width(100);

    let bg_color_input = TextInput::new("BG Color", section.bg_color.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SectionBgColorChanged(index, value))
        .width(100);

    let text_color_input = TextInput::new("Text Color", section.text_color.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SectionTextColorChanged(index, value))
        .width(100);

    let section_header = Row::new()
        .push(text("Section:").size(16).style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(0.7, 0.7, 0.7)), ..TextStyle::default() }))
        .push(name_input)
        .push(text("Layout:").width(45))
        .push(layout_picklist)
        .push(text("BG:").width(25))
        .push(bg_color_input)
        .push(text("Text:").width(30))
        .push(text_color_input)
        .push(button("Remove").on_press(Message::SectionRemoved(index))
            .padding(5))
        .spacing(8);

    let add_sensor = button("+ Add Sensor")
        .on_press(Message::SensorAdded(index))
        .padding(5);

    let mut sensor_list = Column::new().spacing(8);
    for (j, sensor) in section.sensors.iter().enumerate() {
        sensor_list = sensor_list.push(sensor_view(index, j, sensor));
    }

    let sensor_list_container = container(sensor_list)
        .id(iced::widget::Id::from(format!("sensor-list-{}", index)));

    column![
        section_header,
        sensor_list_container,
        add_sensor,
    ]
    .spacing(8)
    .padding(10)
    .into()
}

fn sensor_view(section_index: usize, sensor_index: usize, sensor: &Sensor) -> Element<Message> {
    let name_input = TextInput::new("Sensor Name", sensor.name.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorNameChanged(section_index, sensor_index, value))
        .width(120);

    let description_input = TextInput::new("Description", sensor.description.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorDescriptionChanged(section_index, sensor_index, value))
        .width(160);

    let source_type_picklist = PickList::new(
        vec!["ProcCpu", "ProcMem", "ProcDisk", "Uptime", "NvidiaGpu",
             "Network", "Audio", "Clock", "Window", "Workspace",
             "Command", "EnvVar", "StaticText"].into_iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        Some(&sensor.source_type),
        move |value| Message::SensorSourceTypeChanged(section_index, sensor_index, value),
    ).width(110);

    let source_config: Element<Message> = match sensor.source_type.as_str() {
        "ProcCpu" | "ProcMem" | "ProcDisk" | "Uptime" | "NvidiaGpu" | "Window" | "Workspace" => {
            let label = match sensor.source_type.as_str() {
                "ProcCpu" => "CPU usage",
                "ProcMem" => "Memory",
                "ProcDisk" => "Disk",
                "Uptime" => "Uptime",
                "NvidiaGpu" => "Nvidia GPU",
                "Window" => "Focused window",
                "Workspace" => "Workspace",
                _ => "Sensor",
            };
            Row::new()
                .push(text(format!("{} (no extra config)", label)))
                .spacing(5)
                .into()
        }
        "Network" => TextInput::new("Interface", sensor.source_config.interface.as_deref().unwrap_or(""))
            .on_input(move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "interface".to_string(), value))
            .width(100)
            .into(),
        "Audio" => TextInput::new("Device", sensor.source_config.device.as_deref().unwrap_or(""))
            .on_input(move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "device".to_string(), value))
            .width(100)
            .into(),
        "Clock" => TextInput::new("Format", sensor.source_config.format.as_deref().unwrap_or(""))
            .on_input(move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "format".to_string(), value))
            .width(100)
            .into(),
        "Command" => TextInput::new("Command", sensor.source_config.command.as_deref().unwrap_or(""))
            .on_input(move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "command".to_string(), value))
            .width(160)
            .into(),
        "EnvVar" => TextInput::new("Env Var", sensor.source_config.env_var.as_deref().unwrap_or(""))
            .on_input(move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "env_var".to_string(), value))
            .width(100)
            .into(),
        "StaticText" => TextInput::new("Text", sensor.source_config.static_text.as_deref().unwrap_or(""))
            .on_input(move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "text".to_string(), value))
            .width(100)
            .into(),
        _ => return Row::new().spacing(5).into(),
    };

    let text_color_input = TextInput::new("Text", sensor.text_color.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorTextColorChanged(section_index, sensor_index, value))
        .width(80);

    let bg_color_input = TextInput::new("BG", sensor.bg_color.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorBgColorChanged(section_index, sensor_index, value))
        .width(70);

    let hover_text_input = TextInput::new("Hover Text", sensor.text_color_hover.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorTextColorHoverChanged(section_index, sensor_index, value))
        .width(80);

    let hover_bg_input = TextInput::new("Hover BG", sensor.bg_color_hover.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorBgColorHoverChanged(section_index, sensor_index, value))
        .width(70);

    let border_color_input = TextInput::new("Border", sensor.border_color.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorBorderColorChanged(section_index, sensor_index, value))
        .width(80);

    let border_color_hover_input = TextInput::<Message, Theme>::new("Border Hover", sensor.border_color_hover.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorBorderColorHoverChanged(section_index, sensor_index, value))
        .width(90);

    let border_width_slider = Slider::new(0u32..=10u32, sensor.border_width.unwrap_or(0), move |value| Message::SensorBorderWidthChanged(section_index, sensor_index, value))
        .width(80);

    let border_radius_slider = Slider::new(0u32..=20u32, sensor.border_radius.unwrap_or(0), move |value| Message::SensorBorderRadiusChanged(section_index, sensor_index, value))
        .width(80);

    let padding_slider = Slider::new(0u32..=20u32, sensor.padding.unwrap_or(0), move |value| Message::SensorPaddingChanged(section_index, sensor_index, value))
        .width(80);

    let action_input = TextInput::new("Action (e.g. close_panel or command)",
        sensor.action.as_deref().unwrap_or(""))
        .on_input(move |value| Message::SensorActionChanged(section_index, sensor_index, value))
        .width(220);

    let sensor_col = column![
        Row::new()
            .push(name_input)
            .push(description_input)
            .push(source_type_picklist)
            .push(source_config)
            .spacing(5),
        Row::new()
            .push(text("Text:").width(35))
            .push(text_color_input)
            .push(text("Hover:").width(35))
            .push(hover_text_input)
            .push(text("Border:").width(50))
            .push(border_color_input)
            .push(text("BrdW:").width(30))
            .push(border_width_slider)
            .spacing(5),
        Row::new()
            .push(text("BrdH:").width(50))
            .push(border_color_hover_input)
            .push(text("BG:").width(35))
            .push(bg_color_input)
            .push(text("Hover:").width(35))
            .push(hover_bg_input)
            .push(text("BrdR:").width(50))
            .push(border_radius_slider)
            .push(text("Pad:").width(30))
            .push(padding_slider)
            .spacing(5),
        Row::new()
            .push(text("Action:").width(55))
            .push(action_input)
            .spacing(5),
        button("Remove Sensor").on_press(Message::SensorRemoved(section_index, sensor_index)).padding(5),
    ]
    .spacing(4)
    .padding(8);

    container(sensor_col).id(iced::widget::Id::unique()).into()
}

fn action_buttons() -> Element<'static, Message> {
    let save_btn = button("Save Configuration").on_press(Message::SaveConfig).padding(10);
    let load_btn = button("Load Configuration").on_press(Message::LoadConfig).padding(10);
    let reset_btn = button("Reset to Defaults").on_press(Message::ResetConfig).padding(10);

    Row::new()
        .push(save_btn)
        .push(load_btn)
        .push(reset_btn)
        .spacing(10)
        .into()
}

fn status_bar(state: &PiumaSetup) -> Element<Message> {
    match &state.status_message {
        Some(msg) => {
            let is_error = state.status_is_error;
            text(msg).size(12).style(move |_t: &Theme| TextStyle { color: Some(if is_error { Color::from_rgb(0.9, 0.3, 0.3) } else { Color::from_rgb(0.3, 0.8, 0.4) }), ..TextStyle::default() }).into()
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
