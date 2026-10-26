//! The application state: the message enum, the tab enum, the config-file
//! helpers and `PiumaSetup` holding the edited config plus load, save and
//! live validation.
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::autostart;
use crate::config;
use crate::modal::Modal;
use crate::model::*;
use crate::view_wallpaper::{scan_wallpaper_dirs, wallpaper_dirs};

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
    /// Section title toggle: draw each section's title above its
    /// stacked sensors.
    SectionTitleChanged(bool),
    /// Wallpaper enable toggle (roadmap2 #1).
    WallpaperEnabled(bool),
    /// Wallpaper image path, from the scanned list or a hand-typed
    /// custom path (roadmap2 #1); one destination for both sources.
    WallpaperPathChanged(String),
    /// Wallpaper background mode, `"image"` or `"color"`.
    WallpaperModeChanged(String),
    /// Solid background colour (colour mode).
    WallpaperColorChanged(String),
    /// Toggle Hyprland's own default wallpaper/logo in its config.
    HyprWallpaperToggled(bool),
    /// Wallpaper-palette switch (runtime tones from the wallpaper image).
    PaletteFromWallpaper(bool),
    /// Rounded screen corners (roadmap angoli-schermo): enable toggle,
    /// mask colour and corner radius in px.
    CornersEnabled(bool),
    CornersColorChanged(String),
    CornersRadiusChanged(u32),
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
    /// Opens the threshold colour-rules editor for one sensor (the "…"
    /// right of the Text colour field).
    ColorRulesPressed(usize, usize),
    /// Appends an empty rule row to the open colour-rules editor.
    ColorRuleAdd,
    /// Removes one rule row (its index) of the open colour-rules editor.
    ColorRuleRemove(usize),
    /// `min` typed in one rule row of the open colour-rules editor.
    ColorRuleMinChanged(usize, String),
    /// Colour typed in one rule row of the open colour-rules editor.
    ColorRuleColorChanged(usize, String),
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
    /// Deletes one saved window layout (its automatic name): opens
    /// the confirmation modal.
    LayoutDeletePressed(String),
    /// Deletes every saved window layout: opens the confirmation modal.
    LayoutsClearPressed,
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


/// Default config file: the one the running piuma reads.
pub(crate) const DEFAULT_FILE: &str = "piuma.json";

/// Directory holding the piuma configs (~/.config/piuma).
pub(crate) fn config_dir() -> PathBuf {
    dirs::config_dir().map(|d| d.join("piuma")).unwrap_or_else(|| PathBuf::from("."))
}

/// Full path of a config file name inside the piuma config directory.
pub(crate) fn file_path(name: &str) -> PathBuf {
    config_dir().join(name)
}

/// Sorted *.json file names of the piuma config directory (load picker).
pub(crate) fn list_config_files() -> Vec<String> {
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
pub(crate) fn sanitize_file_name(raw: &str) -> Result<String, String> {
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
    Layouts,
}

impl Tab {
    /// Sidebar order.
    pub(crate) const ALL: [Tab; 8] = [
        Tab::Panel,
        Tab::Notifications,
        Tab::SectionDefaults,
        Tab::SensorDefaults,
        Tab::Sections,
        Tab::Wallpaper,
        Tab::AutostartApps,
        Tab::Layouts,
    ];

    /// Label shown on the sidebar button.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Panel => "Panel",
            Tab::Notifications => "Notifications",
            Tab::SensorDefaults => "Sensor Defaults",
            Tab::SectionDefaults => "Section Defaults",
            Tab::Sections => "Sections",
            Tab::Wallpaper => "Wallpaper",
            Tab::AutostartApps => "Autostart apps",
            Tab::Layouts => "Layouts",
        }
    }
}

pub struct PiumaSetup {
    pub(crate) config: PiumaConfig,
    /// Sidebar tab currently shown (roadmap N4): only its card renders.
    pub(crate) active_tab: Tab,
    pub(crate) status_message: Option<String>,
    pub(crate) status_is_error: bool,
    /// Whether the collapsible "Sensor Defaults" card is open.
    pub(crate) defaults_expanded: bool,
    /// Whether the collapsible "Section Defaults" card is open.
    pub(crate) sec_defaults_expanded: bool,
    /// Indices of the sensors whose "Style overrides" block is open.
    pub(crate) sensor_style_open: HashSet<(usize, usize)>,
    /// Sections collapsed in the Sections accordion (absent = open).
    pub(crate) sections_collapsed: HashSet<usize>,
    /// Sensors whose accordion body is expanded (absent = collapsed).
    pub(crate) sensors_open: HashSet<(usize, usize)>,
    /// Config file currently edited (name inside the piuma config directory).
    pub(crate) current_file: String,
    /// Serialized config at the last load/save: differs = unsaved changes.
    pub(crate) snapshot: String,
    /// Modal dialog stacked over the view, when one is open.
    pub(crate) modal: Option<Modal>,
    /// Values piuma would ignore or clamp (`config::validate_config` on
    /// what Save would write): recomputed after every message, so the
    /// red fields, the section frames and the save guard stay live.
    pub(crate) validation: Vec<config::ConfigError>,
    /// Images found under the freedesktop wallpaper folders (roadmap2 #1),
    /// scanned once at startup for the Wallpaper tab's list.
    pub(crate) wallpaper_files: Vec<PathBuf>,
    /// Whether Hyprland's own default wallpaper/logo is currently off in
    /// its config; `None` when no Hyprland config was found.
    pub(crate) hypr_wallpaper_disabled: Option<bool>,
    /// Autostart `.desktop` files found under the freedesktop dirs
    /// (roadmap2 #2), scanned once at startup for the "Autostart apps"
    /// tab and re-read after every switch — the switches show the spec
    /// state merged with the config's own selection.
    pub(crate) autostart_apps: Vec<autostart::editor::App>,
    /// Saved window layouts (piuma-layouts.json): loaded when the
    /// Layouts tab is selected, refreshed after every delete.
    pub(crate) layouts: Vec<SavedLayout>,
}

impl PiumaSetup {
    /// Builds the initial state, loading the config in use (piuma.json) when
    /// present so the user starts from the real configuration, not from defaults.
    pub(crate) fn new() -> Self {
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
            validation: Vec::new(),
            wallpaper_files: scan_wallpaper_dirs(&wallpaper_dirs()),
            hypr_wallpaper_disabled: crate::hyprland_wallpaper::read_disabled(),
            autostart_apps: Vec::new(),
            layouts: Vec::new(),
        };
        match fs::read_to_string(file_path(&state.current_file)) {
            Ok(content) => match state.load_from_json(&content) {
                Ok(()) => {
                    // Snapshot is only the dirty-check: if this ever failed
                    // (it cannot for a parsed config) the first save would
                    // just ask for confirmation and report any real error.
                    state.snapshot = state.serialize().unwrap_or_default();
                    state.set_status(format!("Loaded {}", state.current_file), false);
                    // Values piuma would ignore (a hand-edited file): named
                    // here and highlighted in red — the save stays blocked
                    // until they are fixed.
                    state.refresh_validation();
                    if !state.validation.is_empty() {
                        let n = state.validation.len();
                        state.set_status(
                            format!(
                                "Loaded {}: {} invalid value{} — highlighted; fix them before saving",
                                state.current_file,
                                n,
                                if n == 1 { "" } else { "s" }
                            ),
                            true,
                        );
                    }
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

    pub(crate) fn set_status(&mut self, msg: String, is_error: bool) {
        self.status_message = Some(msg);
        self.status_is_error = is_error;
    }

    /// Recomputes the invalid-value list from the config as edited right
    /// now: serialized, parsed with piuma's real schema, walked by the
    /// same `config::validate_config` the bar logs at load. A schema-
    /// broken config leaves the list empty — the schema check on save
    /// reports that separately.
    pub(crate) fn refresh_validation(&mut self) {
        self.validation = self
            .serialize()
            .ok()
            .and_then(|json| serde_json::from_str::<config::Config>(&json).ok())
            .map(|cfg| config::validate_config(&cfg))
            .unwrap_or_default();
    }

    /// Reason why the value at `path` is invalid, if any: the path strings
    /// match `config::validate_config` (`panel.sections[0].bg_color` …).
    pub(crate) fn field_err(&self, path: &str) -> Option<&str> {
        self.validation
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.message.as_str())
    }

    /// Number of invalid values inside section `index` (and its sensors):
    /// what frames the section card red in the Sections tab.
    pub(crate) fn section_err_count(&self, index: usize) -> usize {
        let prefix = format!("panel.sections[{index}].");
        self.validation.iter().filter(|e| e.path.starts_with(&prefix)).count()
    }

    /// Human label of a validated path for the refusal modal: section and
    /// sensor names instead of indices ("Section «Stats» › perc_cpu ›
    /// Border: …"), field names mapped to their editor labels.
    pub(crate) fn human_error(&self, error: &config::ConfigError) -> String {
        /// UI label of a field's last path segment.
        fn label(field: &str) -> &str {
            match field {
                "text_color" => "Text",
                "text_color_hover" => "Hover text",
                "bg_color" => "BG",
                "bg_color_hover" => "Hover BG",
                "border_color" => "Border",
                "border_color_hover" => "Hover border",
                "border_width" => "Border width",
                "border_width_hover" => "Hover border width",
                "border_radius" => "Border radius",
                "border_type" => "Border type",
                "padding" => "Padding",
                "weight" => "Weight",
                "layout" => "Layout",
                "anchor" => "Anchor",
                "align" => "Align",
                "section_layout" => "Section layout",
                "thickness" => "Thickness",
                "font_size" => "Font size",
                "default_bg_color" => "Default BG",
                "default_text_color" => "Default text",
                "default_section_weight" => "Default section weight",
                "default_timeout_ms" => "Default timeout",
                "history_size" => "History size",
                "popup_margin_h" => "Popup margin H",
                "popup_margin_v" => "Popup margin V",
                other => other,
            }
        }
        let rest = error.path.strip_prefix("panel.").unwrap_or(&error.path);
        // Sections carry their names: "panel.sections[i](.sensors[j])?.rest".
        let Some(tail) = rest.strip_prefix("sections[") else {
            let field = rest.rsplit('.').next().unwrap_or(rest);
            let group = rest[..rest.len() - field.len()].trim_end_matches('.');
            let group = match group {
                "" => "Panel",
                "sensor_defaults" => "Sensor defaults",
                "section_defaults" => "Section defaults",
                "notifications" => "Notifications",
                other => other,
            };
            return format!("{group} › {}: {}", label(field), error.message);
        };
        let (idx, tail) = tail.split_once(']').unwrap_or((tail, ""));
        let i: usize = idx.parse().unwrap_or(0);
        let section = self.config.sections.get(i);
        let mut head = format!(
            "Section «{}»",
            section
                .and_then(|s| s.name.clone())
                .unwrap_or_else(|| format!("{}", i + 1))
        );
        let field = if let Some(tail) = tail.strip_prefix(".sensors[") {
            let (jdx, tail) = tail.split_once(']').unwrap_or((tail, ""));
            let j: usize = jdx.parse().unwrap_or(0);
            let sensor = section.and_then(|s| s.sensors.get(j));
            head.push_str(&format!(
                " › {}",
                sensor
                    .and_then(|s| s.name.clone().or_else(|| s.description.clone()))
                    .unwrap_or_else(|| format!("sensor {}", j + 1))
            ));
            tail.strip_prefix('.').unwrap_or(tail)
        } else {
            tail.strip_prefix('.').unwrap_or(tail)
        };
        // Threshold colours: "color_rules[k].color" → "Colour rule k+1".
        if let Some(rules) = field.strip_prefix("color_rules[") {
            let (k, _) = rules.split_once(']').unwrap_or((rules, ""));
            let n: usize = k.parse().unwrap_or(0) + 1;
            return format!("{head} › Colour rule {n}: {}", error.message);
        }
        format!("{head} › {}: {}", label(field), error.message)
    }

    /// `current_file`/`snapshot` so the modification check restarts clean.
    pub(crate) fn save_config(&mut self, name: &str) -> Result<(), String> {
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
        let parsed: config::Config = serde_json::from_str(&json)
            .map_err(|e| format!("Save rejected, piuma would refuse this config: {}", e))?;
        // Then the values themselves (same list the bar logs at load): a
        // colour or range piuma would silently ignore must not reach the
        // file — the fields are highlighted and the modal lists them.
        let errors = config::validate_config(&parsed);
        self.validation = errors.clone();
        if !errors.is_empty() {
            return Err(format!(
                "Save blocked: {} invalid value{} (highlighted in the editor)",
                errors.len(),
                if errors.len() == 1 { "" } else { "s" }
            ));
        }
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
    pub(crate) fn load_config(&mut self, name: &str) -> Result<(), String> {
        let content = fs::read_to_string(file_path(name))
            .map_err(|_| format!("No config file found: {}", name))?;
        self.load_from_json(&content)?;
        self.current_file = name.to_string();
        self.snapshot = self.serialize().unwrap_or_default();
        // The newly loaded file carries its own selection: refresh the
        // tab so its switches show it (scanned fresh, like at startup).
        self.autostart_apps = autostart::editor::apps(&self.config.autostart_disabled);
        // The newly loaded file may carry values piuma ignores: refresh
        // the list so they show highlighted right away.
        self.refresh_validation();
        Ok(())
    }

    /// Saves to `name` and reports the outcome in the status bar, returning
    /// it so callers can react (e.g. keep a confirmation modal open on error).
    pub(crate) fn save_reporting(&mut self, name: &str) -> Result<(), String> {
        let outcome = self.save_config(name);
        match &outcome {
            Ok(()) => self.set_status(format!("Configuration saved to {:?}", file_path(name)), false),
            Err(e) => {
                self.set_status(e.clone(), true);
                // Invalid values: the list modal names every field to fix,
                // replacing any confirmation dialog still open.
                if !self.validation.is_empty() {
                    let errors = self
                        .validation
                        .iter()
                        .map(|error| self.human_error(error))
                        .collect();
                    self.modal = Some(Modal::InvalidFields { errors });
                }
            }
        }
        outcome
    }

    /// Loads `name` and reports the outcome in the status bar (see save_reporting).
    pub(crate) fn load_reporting(&mut self, name: &str) -> Result<(), String> {
        let outcome = self.load_config(name);
        match &outcome {
            Ok(()) => {
                self.set_status(format!("Loaded {}", name), false);
                // Loaded values piuma would ignore: named here, red in the
                // editor, blocking the save until fixed (see new()).
                if !self.validation.is_empty() {
                    let n = self.validation.len();
                    self.set_status(
                        format!(
                            "Loaded {}: {} invalid value{} — highlighted; fix them before saving",
                            name,
                            n,
                            if n == 1 { "" } else { "s" }
                        ),
                        true,
                    );
                }
            }
            Err(e) => self.set_status(e.clone(), true),
        }
        outcome
    }

    /// Swaps sensors `i` and `i+1` of a section, carrying their
    /// style-block-open flags so an expanded block follows its sensor.
    /// Rewrites both per-sensor open sets (style block + accordion body)
    /// after a structural change: `f` maps every (section, sensor) key to
    /// its new home, or `None` when it disappeared.
    pub(crate) fn remap_sensor_keys(&mut self, f: impl Fn(usize, usize) -> Option<(usize, usize)>) {
        self.sensor_style_open = self.sensor_style_open.drain().filter_map(|(s, i)| f(s, i)).collect();
        self.sensors_open = self.sensors_open.drain().filter_map(|(s, i)| f(s, i)).collect();
    }

    pub(crate) fn swap_sensors(&mut self, section_index: usize, sensor_index: usize) {
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
    pub(crate) fn swap_sections(&mut self, a: usize, b: usize) {
        self.config.sections.swap(a, b);
        self.remap_sensor_keys(|s, i| Some((if s == a { b } else if s == b { a } else { s }, i)));
    }

}
