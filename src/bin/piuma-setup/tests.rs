//! Unit tests: JSON round-trips against the bar's real config schema, the
//! modal flows, the icon discovery helpers and the live validation.
use super::*;

use std::collections::HashSet;
use std::path::PathBuf;

use crate::controls::*;
use crate::model::*;
use crate::modal::Modal;
use crate::state::Tab;
use crate::update::update;
use crate::view_wallpaper::{parse_user_dirs_pictures, scan_wallpaper_dirs};


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
        validation: Vec::new(),
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

/// The wallpaper-palette toggle survives load→save→load: only the
/// switch travels through the file (the tones are runtime-only),
/// and switching it off drops the key entirely.
#[test]
fn wallpaper_palette_toggle_roundtrips() {
    let mut state = fixture();
    state
        .load_from_json(r##"{"panel":{"wallpaper_palette":true}}"##)
        .unwrap();
    assert_eq!(state.config.wallpaper_palette, Some(true));
    let saved: JsonValue =
        serde_json::from_str(&state.serialize().unwrap()).unwrap();
    assert_eq!(saved["panel"]["wallpaper_palette"].as_bool(), Some(true));
    state.load_from_json(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert_eq!(state.config.wallpaper_palette, Some(true));
    state.config.wallpaper_palette = None;
    let off: JsonValue = serde_json::from_str(&state.serialize().unwrap()).unwrap();
    assert!(off["panel"].get("wallpaper_palette").is_none(), "{off}");
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

/// A `layout` sensor's name must survive load→save — the field that
/// names the saved layout file is the whole point of the source.
#[test]
fn layout_sensor_name_roundtrips() {
    let json = r##"{
  "panel": {
"sections": [
  {
    "name": "win",
    "sensors": [ { "name": "Layout", "source": { "type": "layout", "name": "work" } } ]
  }
]
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("layout config must load");
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    assert_eq!(out["panel"]["sections"][0]["sensors"][0]["source"]["type"], "layout");
    assert_eq!(out["panel"]["sections"][0]["sensors"][0]["source"]["name"], "work");
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

/// The `panel.section_title` key roundtrips load→save with its
/// value, and an untouched config never gets the key injected —
/// unlike app_theme, its bar default is titles drawn.
#[test]
fn section_title_key_roundtrip() {
    let json = r##"{
  "panel": {
"align": "fill",
"anchor": "top",
"section_title": false,
"section_layout": "horizontal",
"sections": [],
"thickness": 26
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("config must load");
    assert_eq!(state.config.section_title, Some(false));
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    assert_eq!(out["panel"]["section_title"], false);
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the saved config (pre-save validation)");
    // Absent stays absent: no key is injected on save.
    let mut bare_state = fixture();
    bare_state
        .load_from_json(r##"{"panel": {"sections": [], "thickness": 26}}"##)
        .expect("config must load");
    let bare: JsonValue =
        serde_json::from_str(&bare_state.serialize().expect("serialize must succeed")).unwrap();
    assert!(bare["panel"].get("section_title").is_none());
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

/// The preview sizes icons to the sensor row's real height, on the
/// bar's own formula (thickness minus the section chrome, clamped),
/// and keeps the theme rule: configured theme or the system one.
#[test]
fn icon_preview_uses_the_row_height() {
    let mut s = test_state();
    s.config.thickness = 30;
    s.config.font_size = 12;
    let section = &s.config.sections[0];
    let sensor = &section.sensors[0];
    // Horizontal section, no padding: 30 − 2×4 of content padding.
    assert_eq!(sensor_icon_size(&s.config, section, sensor), 22);
    // A named vertical title eats its own box: font + 2 + 2×4.
    let mut stacked = section.clone();
    stacked.layout = Some("vertical".to_string());
    s.config.thickness = 44;
    assert_eq!(sensor_icon_size(&s.config, &stacked, sensor), 14);
    // A hairline bar still keeps a visible icon (clamped at 4).
    s.config.thickness = 4;
    assert_eq!(sensor_icon_size(&s.config, section, sensor), 4);
    // Theme: the system one until configured.
    assert_eq!(icon_theme_of(&s.config), icons::icon_theme());
    s.config.icon_theme = Some("Papirus".to_string());
    assert_eq!(icon_theme_of(&s.config), "Papirus");
    assert!(
        resolved_icon(&s.config, 22, None).is_none(),
        "no name, no icon"
    );
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

/// `color_rules` survives load→save sorted by ascending `min` (the
/// order the bar evaluates them) and the pre-save schema check passes.
#[test]
fn color_rules_roundtrip_sorted_and_accepted() {
    let json = r##"{
  "panel": {
"align": "fill",
"anchor": "top",
"section_layout": "horizontal",
"sections": [
  {
    "layout": "horizontal",
    "name": "sys",
    "sensors": [ {
      "name": "CPU",
      "source": { "type": "perc_cpu" },
      "color_rules": [
        { "min": 80, "color": "#ff0000" },
        { "min": 50, "color": "#ffff00" }
      ]
    } ]
  }
],
"thickness": 26
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("config with rules must load");
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    let rules = out["panel"]["sections"][0]["sensors"][0]["color_rules"]
        .as_array()
        .expect("the rules must be written back");
    assert_eq!(rules.len(), 2, "no rule is dropped");
    assert_eq!(rules[0]["min"].as_f64(), Some(50.0), "sorted ascending");
    assert_eq!(rules[0]["color"].as_str(), Some("#ffff00"));
    assert_eq!(rules[1]["min"].as_f64(), Some(80.0));
    assert_eq!(rules[1]["color"].as_str(), Some("#ff0000"));
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the saved config (pre-save validation)");
}

/// The "…" opens the editor with the existing rows sorted by `min`,
/// edits land on the row they came from and Apply stores them sorted.
#[test]
fn color_rules_editor_opens_sorted_and_applies() {
    let mut s = test_state();
    // Stored in file order: the editor must show them evaluated order.
    s.config.sections[0].sensors[0].color_rules = Some(vec![
        config::ColorRule { min: 80.0, color: "#ff0000".to_string() },
        config::ColorRule { min: 50.0, color: "#ffff00".to_string() },
    ]);
    let _ = update(&mut s, Message::ColorRulesPressed(0, 0));
    match &s.modal {
        Some(Modal::ColorRules { rules, .. }) => {
            assert_eq!(rules.len(), 2);
            assert_eq!(rules[0].min, "50");
            assert_eq!(rules[1].min, "80");
        }
        other => panic!("the editor must open sorted, got {other:?}"),
    }
    let _ = update(&mut s, Message::ColorRuleAdd);
    let _ = update(&mut s, Message::ColorRuleMinChanged(2, "120".to_string()));
    let _ = update(&mut s, Message::ColorRuleColorChanged(2, "#00ff00".to_string()));
    let _ = update(&mut s, Message::ModalConfirm);
    assert!(s.modal.is_none(), "Apply closes the editor");
    let rules = s.config.sections[0].sensors[0]
        .color_rules
        .clone()
        .expect("Apply must store the rules");
    let mins: Vec<f64> = rules.iter().map(|r| r.min).collect();
    assert_eq!(mins, vec![50.0, 80.0, 120.0], "stored sorted by min");
}

/// A typed non-number `min` (or a rule without colour) is an error:
/// the popup reopens with its rows, nothing is written. Blank rows are
/// dropped, and an editor left empty clears the key.
#[test]
fn color_rules_editor_validates_and_clears() {
    let mut s = test_state();
    let _ = update(&mut s, Message::ColorRulesPressed(0, 0));
    let _ = update(&mut s, Message::ColorRuleAdd);
    let _ = update(&mut s, Message::ColorRuleMinChanged(0, "abc".to_string()));
    let _ = update(&mut s, Message::ColorRuleColorChanged(0, "#ff0000".to_string()));
    let _ = update(&mut s, Message::ModalConfirm);
    assert!(matches!(s.modal, Some(Modal::ColorRules { .. })), "the popup stays open");
    assert!(s.status_is_error, "the reason is shown as an error");
    assert!(s.config.sections[0].sensors[0].color_rules.is_none(), "nothing written");
    // Fix it: the valid row applies, the still-blank second row is dropped.
    let _ = update(&mut s, Message::ColorRuleMinChanged(0, "30".to_string()));
    let _ = update(&mut s, Message::ModalConfirm);
    assert!(s.modal.is_none());
    let stored = s.config.sections[0].sensors[0].color_rules.clone().expect("stored");
    assert_eq!(
        stored,
        vec![config::ColorRule { min: 30.0, color: "#ff0000".to_string() }],
        "the blank row is dropped"
    );
    // Empty editor = no rules = key absent, like every style field.
    let _ = update(&mut s, Message::ColorRulesPressed(0, 0));
    let _ = update(&mut s, Message::ColorRuleRemove(0));
    let _ = update(&mut s, Message::ModalConfirm);
    assert!(s.config.sections[0].sensors[0].color_rules.is_none());
}

// ── value validation (invalid values block the save) ───────────

/// An invalid colour typed in the editor: the list has it at its path,
/// the save is refused (nothing written), the status names the block
/// and the modal lists the field in human words.
#[test]
fn invalid_value_blocks_the_save_and_is_listed() {
    let mut s = fixture();
    let _ = update(
        &mut s,
        Message::DefaultsStyleChanged("border_color".to_string(), "#zz0000".to_string()),
    );
    assert_eq!(
        s.field_err("panel.sensor_defaults.border_color"),
        Some("\"#zz0000\": expected hex digits after #"),
        "the reason is available to the field"
    );
    let err = s.save_reporting("piuma.json").expect_err("must refuse");
    assert!(err.contains("Save blocked"), "message: {err}");
    assert!(s.status_is_error);
    let Some(Modal::InvalidFields { errors }) = &s.modal else {
        panic!("the error list modal must open");
    };
    assert_eq!(errors.len(), 1, "one line per invalid value: {errors:?}");
    assert!(
        errors[0].contains("Sensor defaults") && errors[0].contains("Border"),
        "human path: {}",
        errors[0]
    );
    // Fix the value: the list empties and the save goes through the
    // normal confirmation flow again (no write in this test).
    let _ = update(
        &mut s,
        Message::DefaultsStyleChanged("border_color".to_string(), "#dddddd00".to_string()),
    );
    assert!(s.validation.is_empty(), "fixed value clears the list");
    assert!(s.field_err("panel.sensor_defaults.border_color").is_none());
}

/// Loaded invalid values are found without any edit, reported per
/// section with the section/sensor names and counted per section
/// (what frames the section card red).
#[test]
fn loaded_invalid_values_are_named_per_section() {
    let mut s = test_state();
    s.config.sections[0].sensors[0].name = Some("perc".to_string());
    s.config.sections[0].sensors[0].border_color = Some("nope".to_string());
    s.config.sections[1].bg_color = Some("#ff00zz".to_string());
    s.refresh_validation();
    assert_eq!(s.validation.len(), 2, "both bad values: {:?}", s.validation);
    assert_eq!(s.section_err_count(0), 1);
    assert_eq!(s.section_err_count(1), 1);
    let line = s.human_error(&s.validation[0]);
    assert!(
        line.contains("Section «Left»") && line.contains("perc") && line.contains("Border"),
        "human path: {line}"
    );
    assert!(
        s.human_error(&s.validation[1]).contains("Section «Right»"),
        "second section: {}",
        s.human_error(&s.validation[1])
    );
}

/// Every message recomputes the list, so a live edit turns the field
/// red and the next one clears it again.
#[test]
fn validation_follows_every_edit() {
    let mut s = fixture();
    assert!(s.validation.is_empty());
    let _ = update(
        &mut s,
        Message::SecDefaultsStyleChanged("bg_color".to_string(), "#12".to_string()),
    );
    assert!(s.field_err("panel.section_defaults.bg_color").is_some());
    let _ = update(
        &mut s,
        Message::SecDefaultsStyleChanged("bg_color".to_string(), "#112233".to_string()),
    );
    assert!(s.validation.is_empty());
}

/// The reason line must keep the widget type at the field's position: iced
/// replaces a subtree on a tag mismatch (`Tree::diff`), which destroys the
/// text input's focus state — the mid-typing focus-stealing bug.
#[test]
fn invalid_under_keeps_the_widget_shape() {
    let build = |reason: Option<&'static str>| {
        let field: iced::Element<'_, Message> =
            input("#rgb", "#ffffff", |v| Message::FontChanged(v)).into();
        invalid_under(field, reason)
    };
    assert_eq!(
        build(None).as_widget().tag(),
        build(Some("bad")).as_widget().tag(),
    );
}
