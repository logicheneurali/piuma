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
        wallpaper_thumbs: std::collections::HashMap::new(),
        wallpaper_thumbs_requested: false,
        hypr_wallpaper_disabled: None,
        autostart_apps: Vec::new(),
        layouts: Vec::new(),
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
/// for a config piuma would reject (the fixture holds a command sensor
/// without its required parameter).
#[test]
fn save_refuses_config_piuma_would_reject() {
    let mut state = fixture();
    state.load_from_json(LEGACY_CONFIG).unwrap();
    let err = state.save_config("never-written.json").unwrap_err();
    assert!(err.contains("Save rejected") && err.contains("missing field"),
        "error must state the rejection and its reason, got: {}", err);
    assert!(!file_path("never-written.json").exists(), "no file may be written");
}

/// A Text/Icon sensor may carry the icon alone: the empty text is a
/// valid source and the saved JSON must pass the bar's schema.
#[test]
fn text_icon_sensor_saves_with_only_an_icon() {
    let json = r##"{
  "panel": {
    "sections": [ {
      "name": "sys",
      "sensors": [ { "icon": "folder", "source": { "type": "static_text" } } ]
    } ]
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("text/icon config must load");
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    assert_eq!(out["panel"]["sections"][0]["sensors"][0]["icon"], "folder");
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the icon-only text/icon sensor");
    assert!(
        config::validate_config(&serde_json::from_str(&saved).unwrap()).is_empty(),
        "text or icon is enough"
    );
}

/// With no text, the icon must actually resolve: a typo is blocked
/// before a file is written (the bar would render nothing).
#[test]
fn text_icon_sensor_without_text_needs_a_resolvable_icon() {
    let json = r##"{
  "panel": {
    "sections": [ {
      "name": "sys",
      "sensors": [ { "icon": "piuma-definitely-missing-icon",
                     "source": { "type": "static_text" } } ]
    } ]
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("text/icon config must load");
    state.refresh_validation();
    assert!(
        state.field_err("panel.sections[0].sensors[0].icon").is_some(),
        "the missing icon is named: {:?}",
        state.validation
    );
    let err = state.save_config("never-written-icon.json").unwrap_err();
    assert!(err.contains("Save blocked"), "message: {err}");
    assert!(!file_path("never-written-icon.json").exists(), "no file may be written");
}

#[test]
fn text_icon_is_displayed_as_text_slash_icon() {
    assert_eq!(source_label("static_text"), "Text/Icon");
    assert_eq!(source_type_display("static_text"), "Text/Icon");
    assert_eq!(source_type_from_display("Text/Icon"), "static_text");
    assert_eq!(source_label("volume_meter"), "Volume bar");
    assert_eq!(source_type_display("volume_meter"), "Volume bar");
    assert_eq!(source_type_from_display("Volume bar"), "volume_meter");
    assert_eq!(source_field_display("volume_meter"), "volume_meter");
    assert_eq!(source_type_from_display("volume_meter"), "volume_meter");
    assert_eq!(source_field_display("static_text"), "Text/Icon");
    assert_eq!(source_label("workspace_dots"), "Workspace dots");
    assert_eq!(source_field_display("workspace_dots"), "workspace_dots");
    assert_eq!(source_type_from_display("workspace_dots"), "workspace_dots");
    assert_eq!(source_label("media"), "Media");
    assert_eq!(source_field_display("media"), "media");
    assert_eq!(source_type_display("perc_cpu"), "perc_cpu");
}

/// The optional value animation survives a load/save roundtrip, both
/// the panel mode and the per-sensor opt-in.
#[test]
fn value_effect_and_sensor_optin_roundtrip() {
    let json = r##"{
  "panel": {
    "value_effect": "sheen",
    "sections": [ {
      "sensors": [ { "name": "CPU", "effect": true,
                     "source": { "type": "perc_cpu" } } ]
    } ]
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("effect config must load");
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    assert_eq!(out["panel"]["value_effect"], "sheen");
    assert_eq!(
        out["panel"]["sections"][0]["sensors"][0]["effect"].as_bool(),
        Some(true)
    );
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the saved effect config");
}

/// The graphical workspace sensor's circle count survives a save and
/// piuma accepts the result.
#[test]
fn workspace_dots_roundtrips_its_count() {
    let json = r##"{
  "panel": {
    "sections": [ {
      "name": "sys",
      "sensors": [ { "source": { "type": "workspace_dots", "count": 12 } } ]
    } ]
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("workspace dots must load");
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    assert_eq!(
        out["panel"]["sections"][0]["sensors"][0]["source"]["type"],
        "workspace_dots"
    );
    assert_eq!(
        out["panel"]["sections"][0]["sensors"][0]["source"]["count"],
        12
    );
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the saved workspace dots");
}

/// The graphical volume meter is an ordinary config sensor for the
/// editor: its device survives a save and piuma accepts the result.
#[test]
fn volume_meter_roundtrips_its_device() {
    let json = r##"{
  "panel": {
    "sections": [ {
      "name": "sys",
      "sensors": [ { "source": { "type": "volume_meter", "device": "Speaker" } } ]
    } ]
  }
}"##;
    let mut state = fixture();
    state.load_from_json(json).expect("volume meter must load");
    let saved = state.serialize().expect("serialize must succeed");
    let out: JsonValue = serde_json::from_str(&saved).expect("saved JSON must parse");
    assert_eq!(
        out["panel"]["sections"][0]["sensors"][0]["source"]["type"],
        "volume_meter"
    );
    assert_eq!(
        out["panel"]["sections"][0]["sensors"][0]["source"]["device"],
        "Speaker"
    );
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the saved volume meter");
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

/// A `layout` sensor survives load→save with no source parameters:
/// layouts are named automatically now, so a legacy `"name"` in the
/// file is ignored and never written back.
#[test]
fn layout_sensor_roundtrips_without_a_name() {
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
    assert!(
        out["panel"]["sections"][0]["sensors"][0]["source"].get("name").is_none(),
        "the legacy name must not be written back: {saved}"
    );
    serde_json::from_str::<config::Config>(&saved)
        .expect("piuma must accept the saved config (pre-save validation)");
}

/// A saved layout's windows are grouped by class, with the classes and
/// their titles sorted; an entry that isn't an array shows no classes.
#[test]
fn layout_classes_group_windows_by_class() {
    let value: JsonValue = serde_json::from_str(
        r#"[{"class":"vivaldi-stable","title":"WhatsApp"},{"class":"kitty","title":"shell"},{"class":"kitty","title":"vim"}]"#,
    )
    .expect("entries must parse");
    let classes = layout_classes(&value);
    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0].class, "kitty");
    assert_eq!(classes[0].titles, vec!["shell", "vim"]);
    assert_eq!(classes[1].class, "vivaldi-stable");
    assert_eq!(classes[1].titles, vec!["WhatsApp"]);
    assert!(layout_classes(&JsonValue::Bool(true)).is_empty(), "a non-array entry shows nothing");
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
    let _ = update(&mut state, Message::TabSelected(Tab::Tweaks));
    assert_eq!(state.active_tab, Tab::Tweaks);
    assert_eq!(
        state.serialize().expect("serialize must succeed"),
        before,
        "switching tabs must not change the edited config"
    );
    assert_eq!(
        Tab::ALL.last(),
        Some(&Tab::Tweaks),
        "Tweaks is the last sidebar category"
    );
}

/// The Tweaks tab creates the block on first touch, writes concrete
/// min/max, and the range is checked by the same validation the save
/// uses (the bar parses the identical shape).
#[test]
fn dynamic_gaps_tweaks_roundtrip() {
    let mut s = test_state();
    let json: JsonValue = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert!(json["panel"].get("tweaks").is_none(), "absent until touched");

    let _ = update(&mut s, Message::DynamicGapsEnabled(true));
    assert_eq!(
        s.config.tweaks.as_ref().and_then(|t| t.dynamic_gaps.as_ref()).map(|g| (g.min, g.max)),
        Some((4, 16)),
        "first touch seeds the bar's defaults"
    );
    let _ = update(&mut s, Message::GapMinChanged(6));
    let _ = update(&mut s, Message::GapMaxChanged(12));

    let saved = s.serialize().unwrap();
    let parsed: config::Config = serde_json::from_str(&saved).unwrap();
    let gaps = parsed
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.dynamic_gaps.as_ref())
        .expect("the block must reach the bar");
    assert!(gaps.enabled);
    assert_eq!((gaps.min, gaps.max), (6, 12));
    assert!(config::validate_config(&parsed).is_empty());

    // The UI cannot create an inverted range: the maximum stops at the
    // minimum (the shared validation still guards hand-edited files).
    let _ = update(&mut s, Message::GapMaxChanged(3));
    let clamped: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let gaps = clamped
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.dynamic_gaps.as_ref())
        .expect("the block stays");
    assert_eq!((gaps.min, gaps.max), (6, 6));
    assert!(config::validate_config(&clamped).is_empty());
}

/// The screensaver block: first touch seeds the bar's defaults (off, 10
/// minutes), the delay is clamped to 1..=120, and the saved shape parses
/// back into the bar's own config.
#[test]
fn screensaver_tweaks_roundtrip() {
    let mut s = test_state();
    let json: JsonValue = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert!(json["panel"].get("tweaks").is_none(), "absent until touched");

    let _ = update(&mut s, Message::ScreensaverEnabled(true));
    let saved = s.serialize().unwrap();
    let parsed: config::Config = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        parsed.panel.tweaks.as_ref().and_then(|t| t.screensaver.as_ref()).map(|v| (v.enabled, v.minutes)),
        Some((true, 10)),
        "first touch seeds the bar's defaults"
    );

    let _ = update(&mut s, Message::ScreensaverMinutesChanged(45));
    let _ = update(&mut s, Message::ScreensaverMinutesChanged(999));
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let saver = parsed
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.screensaver.as_ref())
        .expect("the block must reach the bar");
    assert!(saver.enabled);
    assert_eq!(saver.minutes, 120, "clamped to the bar's range");
    assert!(config::validate_config(&parsed).is_empty());
}

/// Glass and auto-hide: first touch seeds the bar's defaults, the
/// opacity round-trips as 0.05..1.0, and both parse back into the bar.
#[test]
fn glass_and_auto_hide_roundtrip() {
    let mut s = test_state();
    let _ = update(&mut s, Message::GlassEnabled(true));
    let _ = update(&mut s, Message::GlassOpacityChanged(60));
    let _ = update(&mut s, Message::AutoHideEnabled(true));
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let tweaks = parsed.panel.tweaks.as_ref().expect("tweaks");
    let glass = tweaks.glass.as_ref().expect("glass");
    assert!(glass.enabled);
    assert!((glass.opacity - 0.60).abs() < 1e-6);
    assert!(tweaks.auto_hide.as_ref().expect("auto_hide").enabled);
    assert!(config::validate_config(&parsed).is_empty());

    let _ = update(&mut s, Message::GlassOpacityChanged(1000));
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert_eq!(parsed.panel.tweaks.unwrap().glass.unwrap().opacity, 1.0);
}

/// The footer's reload hint watches the root `panels` value: a changed
/// panel set needs the bar restarted, everything else hot-reloads.
#[test]
fn panel_changes_are_detected_for_the_reload_hint() {
    use crate::state::panels_of;
    let one = r#"{"panel":{},"panels":[{"anchor":"top"}]}"#;
    let two = r#"{"panel":{},"panels":[{"anchor":"top"},{"anchor":"bottom"}]}"#;
    assert_eq!(panels_of(one), panels_of(one));
    assert_ne!(panels_of(one), panels_of(two));
    assert_eq!(panels_of("{}"), None);
    assert_ne!(panels_of("{}"), panels_of(one));
}

/// The floating window menu round-trips into the bar's schema; an
/// unknown picker label falls back to the default instead of writing a
/// value the bar would refuse.
#[test]
fn window_menu_tweaks_roundtrip() {
    let mut s = test_state();
    let json: JsonValue = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert!(json["panel"].get("tweaks").is_none(), "absent until touched");

    let _ = update(&mut s, Message::WindowMenuEnabled(true));
    let _ = update(
        &mut s,
        Message::WindowMenuPositionChanged("Bottom right".to_string()),
    );
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let menu = parsed
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.window_menu.as_ref())
        .expect("the block must reach the bar");
    assert!(menu.enabled);
    assert_eq!(menu.position, config::WindowMenuPosition::BottomRight);
    assert!(config::validate_config(&parsed).is_empty());

    let _ = update(
        &mut s,
        Message::WindowMenuPositionChanged("middle".to_string()),
    );
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert_eq!(
        parsed.panel.tweaks.unwrap().window_menu.unwrap().position,
        config::WindowMenuPosition::TopCenter,
        "an unknown label keeps the default"
    );
}

/// Every shadow field round-trips into the bar's schema, and the shared
/// validation refuses the values Hyprland would.
#[test]
fn shadows_tweaks_roundtrip() {
    let mut s = test_state();
    let json: JsonValue = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert!(json["panel"].get("tweaks").is_none(), "absent until touched");

    let _ = update(&mut s, Message::ShadowsEnabled(true));
    let _ = update(&mut s, Message::ShadowRangeChanged(24));
    let _ = update(&mut s, Message::ShadowPowerChanged(3));
    let _ = update(&mut s, Message::ShadowSharpChanged(true));
    let _ = update(&mut s, Message::ShadowScaleChanged(15));
    let _ = update(&mut s, Message::ShadowOffsetChanged(0, 4));
    let _ = update(&mut s, Message::ShadowColorChanged("#10203080".to_string()));

    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let sh = parsed
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.shadows.as_ref())
        .expect("the block must reach the bar");
    assert!(sh.enabled && sh.sharp);
    assert_eq!((sh.range, sh.render_power), (24, 3));
    assert_eq!(sh.color, "#10203080");
    assert_eq!(sh.offset, [4, 0]);
    assert!((sh.scale - 1.5).abs() < 1e-6, "tenths become the float");
    assert!(config::validate_config(&parsed).is_empty());

    // The sliders clamp; a hand-edited file is what the validation guards.
    let _ = update(&mut s, Message::ShadowRangeChanged(400));
    let _ = update(&mut s, Message::ShadowPowerChanged(9));
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let sh = parsed
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.shadows.as_ref())
        .unwrap();
    assert_eq!((sh.range, sh.render_power), (100, 4));
}

/// The border fields round-trip into the bar's schema; the switch first
/// seeds them from Hyprland, which is checked here through the stored
/// values it must not touch when no hyprctl answers.
#[test]
fn border_tweaks_roundtrip() {
    let mut s = test_state();
    let json: JsonValue = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert!(json["panel"].get("tweaks").is_none(), "absent until touched");

    let _ = update(&mut s, Message::BordersEnabled(true));
    let _ = update(&mut s, Message::BorderSizeChanged(3));
    let _ = update(&mut s, Message::BorderActiveChanged("#ff0000ff".to_string()));
    let _ = update(&mut s, Message::BorderInactiveChanged("#00ff0080".to_string()));

    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    let b = parsed
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.borders.as_ref())
        .expect("the block must reach the bar");
    assert!(b.enabled);
    assert_eq!(b.size, 3);
    assert_eq!(b.active, "#ff0000ff");
    assert_eq!(b.inactive, "#00ff0080");
    assert!(config::validate_config(&parsed).is_empty());

    // The slider clamps; hand-edited files are the validation's job.
    let _ = update(&mut s, Message::BorderSizeChanged(99));
    let parsed: config::Config = serde_json::from_str(&s.serialize().unwrap()).unwrap();
    assert_eq!(parsed.panel.tweaks.unwrap().borders.unwrap().size, 20);
}

/// The font picker lists each family once, alphabetically without case.
#[test]
fn font_families_are_sorted_and_unique() {
    let families = crate::controls::sorted_families(
        ["Noto Sans", "dejaVu Sans", "Noto Sans", "Fira Sans"]
            .into_iter()
            .map(str::to_string),
    );
    assert_eq!(families, vec!["dejaVu Sans", "Fira Sans", "Noto Sans"]);
}

/// The two gap sliders never cross: a minimum above the maximum is
/// clamped to it, and a maximum below the minimum is clamped up.
#[test]
fn dynamic_gap_sliders_never_cross() {
    let mut s = test_state();
    let _ = update(&mut s, Message::DynamicGapsEnabled(true));
    let gaps = |s: &PiumaSetup| {
        s.config
            .tweaks
            .as_ref()
            .and_then(|t| t.dynamic_gaps.as_ref())
            .map(|g| (g.min, g.max))
            .unwrap()
    };
    assert_eq!(gaps(&s), (4, 16), "defaults");
    let _ = update(&mut s, Message::GapMinChanged(40));
    assert_eq!(gaps(&s), (16, 16), "the minimum stops at the maximum");
    let _ = update(&mut s, Message::GapMaxChanged(2));
    assert_eq!(gaps(&s), (16, 16), "the maximum stops at the minimum");
    let _ = update(&mut s, Message::GapMaxChanged(30));
    assert_eq!(gaps(&s), (16, 30));
    let _ = update(&mut s, Message::GapMinChanged(20));
    assert_eq!(gaps(&s), (20, 30));
}

/// The runtime-state sensors and a command's interval survive the
/// editor's save path and validate against the bar's schema.
#[test]
fn runtime_sensors_and_command_interval_roundtrip() {
    let mut s = test_state();
    let sensor = |name: &str, source_type: &str| Sensor {
        name: Some(name.to_string()),
        source_type: source_type.to_string(),
        ..Sensor::default()
    };
    let mut command = sensor("CMD", "command");
    command.source_config = SourceConfig {
        command: Some("date +%s".to_string()),
        interval: Some(30),
        interval_text: "30".to_string(),
        ..SourceConfig::default()
    };
    let mut list = sensor("", "list");
    list.source_config = SourceConfig {
        command: Some("ls ~/Downloads".to_string()),
        count: Some(5),
        ..SourceConfig::default()
    };
    s.config.sections[0].sensors = vec![
        sensor("NET", "net_info"),
        sensor("PWR", "power_profile"),
        sensor("BT", "bluetooth"),
        sensor("KBD", "keyboard"),
        command,
        list,
    ];

    let saved = s.serialize().unwrap();
    let parsed: config::Config = serde_json::from_str(&saved).unwrap();
    assert!(
        config::validate_config(&parsed).is_empty(),
        "the editor must save a config the bar accepts"
    );
    let sources: Vec<&config::SensorSource> = parsed.panel.sections[0]
        .sensors
        .iter()
        .map(|sensor| &sensor.source)
        .collect();
    assert!(matches!(sources[0], config::SensorSource::NetInfo {}));
    assert!(matches!(sources[1], config::SensorSource::PowerProfile {}));
    assert!(matches!(sources[2], config::SensorSource::Bluetooth {}));
    assert!(matches!(sources[3], config::SensorSource::Keyboard {}));
    assert!(matches!(
        sources[4],
        config::SensorSource::Command { interval: Some(30), .. }
    ));
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
        .load_from_json(r##"{"panel":{"wallpaper":{"enabled":true,"mode":"color","path":"/tmp/a.png","color":"#112233"}}}"##)
        .expect("wallpaper block must parse");
    let wp = state.config.wallpaper.as_ref().expect("block present");
    assert!(wp.enabled);
    assert_eq!(wp.mode, "color");
    assert_eq!(wp.path, "/tmp/a.png");
    assert_eq!(wp.color, "#112233");
    let out: JsonValue = serde_json::from_str(&state.serialize().expect("serialize")).expect("json");
    assert_eq!(out["panel"]["wallpaper"]["enabled"], true);
    assert_eq!(out["panel"]["wallpaper"]["mode"], "color");
    assert_eq!(out["panel"]["wallpaper"]["path"], "/tmp/a.png");
    assert_eq!(out["panel"]["wallpaper"]["color"], "#112233");

    // A partial block falls back per field…
    let mut state = fixture();
    state
        .load_from_json(r#"{"panel":{"wallpaper":{}}}"#)
        .expect("partial block must parse");
    let wp = state.config.wallpaper.as_ref().expect("block present");
    assert!(!wp.enabled);
    assert_eq!(wp.mode, "image");
    assert!(wp.path.is_empty());
    assert!(wp.color.is_empty());

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

/// The visual picker's samples: every parameterless/typical type has a
/// representative reading and a short name to draw.
#[test]
fn source_previews_carry_a_sample_reading() {
    for kind in SOURCE_TYPES {
        let value = crate::preview::sample_value(kind);
        let name = crate::preview::sample_name(kind);
        // Purely graphical sensors legitimately draw no text; every
        // textual one must not fall back to the generic "value".
        if !matches!(
            canonical_type(kind),
            "workspace_dots" | "tray" | "taskbar" | "volume_meter" | "static_text"
        ) {
            assert_ne!(value, "value", "no sample for {kind}");
        }
        if !matches!(canonical_type(kind), "static_text") {
            assert!(!name.is_empty(), "no preview name for {kind}");
        }
    }
    assert_eq!(crate::preview::sample_value("perc_cpu"), "12%");
    assert_eq!(crate::preview::sample_value("media"), "⏵  Song — Artist");
}

/// WCAG contrast: identical colours are 1:1, black on white ~21:1, and
/// the preview warns only below its 3:1 threshold.
#[test]
fn contrast_ratio_is_the_wcag_value() {
    let black = iced::Color::BLACK;
    let white = iced::Color::WHITE;
    let ratio = crate::preview::contrast_ratio(black, white);
    assert!((ratio - 21.0).abs() < 0.1, "got {ratio}");
    assert!((crate::preview::contrast_ratio(black, black) - 1.0).abs() < f32::EPSILON);

    let mut s = test_state();
    s.config.sections[0].sensors[0].text_color = Some("#000000".to_string());
    s.config.sections[0].sensors[0].bg_color = Some("#000000".to_string());
    assert!(
        crate::preview::contrast_hint(
            &s.config,
            Some(&s.config.sections[0]),
            &s.config.sections[0].sensors[0],
        )
        .is_some()
    );
}

/// Placement grids offer exactly the values the bar accepts.
#[test]
fn placement_grids_cover_the_config_values() {
    let anchors: Vec<&str> = anchor_cells()
        .iter()
        .filter_map(|(_, value)| *value)
        .collect();
    assert_eq!(anchors.len(), 8);
    for anchor in &anchors {
        let cfg: crate::config::Config = serde_json::from_str(&format!(
            r#"{{"panel":{{"anchor":"{anchor}","sections":[]}}}}"#
        ))
        .expect("anchor must parse");
        assert!(
            crate::config::validate_config(&cfg).is_empty(),
            "{anchor} rejected by the bar"
        );
    }
    let notif: Vec<&str> = notif_cells().iter().map(|(_, value)| *value).collect();
    assert_eq!(notif.len(), 6);
    assert!(notif.contains(&"top-center") && notif.contains(&"bottom-right"));
}

/// Parameter pick lists keep the sentinel first and never lose a custom
/// value the detection did not find.
#[test]
fn parameter_options_keep_the_current_value() {
    let options = options_with_current(vec!["wlan0".to_string()], "eth0");
    assert_eq!(options[0], DEFAULT_OPTION);
    assert_eq!(options[1], "eth0");
    assert!(options.contains(&"wlan0".to_string()));
    let again = options_with_current(vec!["wlan0".to_string()], "wlan0");
    assert_eq!(again.iter().filter(|o| *o == "wlan0").count(), 1);
    assert!(!clock_formats().is_empty() && !calendar_formats().is_empty());
}

/// The source field's card reopens the picker and retyping applies on
/// the click, without touching the sensor's name or style.
#[test]
fn sensor_type_picker_retypes_an_existing_sensor() {
    let mut s = test_state();
    s.config.sections[0].sensors[0].name = Some("Keep".to_string());
    s.config.sections[0].sensors[0].text_color = Some("#abcdef".to_string());
    let _ = update(&mut s, Message::SensorTypePickerOpened(0, 0));
    assert!(matches!(
        s.modal,
        Some(Modal::AddSensor { sensor_index: Some(0), .. })
    ));
    let _ = update(&mut s, Message::ModalTypeChanged("media".to_string()));
    let sensor = &s.config.sections[0].sensors[0];
    assert_eq!(sensor.source_type, "media");
    assert_eq!(sensor.name.as_deref(), Some("Keep"));
    assert_eq!(sensor.text_color.as_deref(), Some("#abcdef"));
    assert!(s.modal.is_none());
}

/// A mini-bar click opens the Sections tab on that sensor's card.
#[test]
fn mini_bar_click_opens_the_sensor_card() {
    let mut s = test_state();
    s.active_tab = Tab::Panel;
    s.sections_collapsed.insert(1);
    let _ = update(&mut s, Message::SensorSelected(1, 0));
    assert_eq!(s.active_tab, Tab::Sections);
    assert!(s.sensors_open.contains(&(1, 0)));
    assert!(!s.sections_collapsed.contains(&1));
    // Out-of-range clicks are ignored.
    let _ = update(&mut s, Message::SensorSelected(9, 9));
    assert_eq!(s.active_tab, Tab::Sections);
}

/// The × on an override clears the key (the bar reads its inherited
/// value again) instead of storing the "disabled" marker.
#[test]
fn override_reset_clears_the_key() {
    let mut s = test_state();
    let sensor = &mut s.config.sections[0].sensors[0];
    sensor.border_width = Some(4);
    sensor.padding = Some(6);
    sensor.text_color = Some("#ffffff".to_string());
    let _ = update(&mut s, Message::SensorBorderWidthChanged(0, 0, -1));
    let _ = update(&mut s, Message::SensorPaddingChanged(0, 0, -1));
    let _ = update(
        &mut s,
        Message::SensorTextColorChanged(0, 0, String::new()),
    );
    let sensor = &s.config.sections[0].sensors[0];
    assert!(sensor.border_width.is_none());
    assert!(sensor.padding.is_none());
    assert!(sensor.text_color.is_none());
}

/// Wallpaper previews decode small, skip unreadable files and land in
/// the cache so the next run lists them without decoding again.
#[test]
fn wallpaper_thumbnails_skip_broken_files_and_cache() {
    let dir = std::env::temp_dir().join(format!("piuma-thumb-test-{}", std::process::id()));
    let cache = dir.join("cache");
    fs::create_dir_all(&dir).expect("fixture dir");
    let good = dir.join("good.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 10, 10, 255]))
        .save(&good)
        .expect("png fixture");
    let bad = dir.join("broken.png");
    fs::write(&bad, b"not an image").expect("broken fixture");
    let thumb = crate::view_wallpaper::thumbnail_cached(Some(&cache), &good)
        .expect("the valid image must decode");
    assert_eq!(thumb.0, good.to_string_lossy());
    assert!(
        crate::view_wallpaper::thumbnail_cached(Some(&cache), &bad).is_none(),
        "a broken image is skipped, the grid still lists it by name"
    );
    assert!(
        fs::read_dir(&cache).map_or(0, |entries| entries.count()) >= 1,
        "the decoded preview must be cached on disk"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The mini-bar jump converts the card's window position back to a
/// content offset (translation included) with a small top margin.
#[test]
fn scroll_target_converts_window_coordinates() {
    assert_eq!(scroll_target(100.0, 0.0, 500.0), 392.0);
    assert_eq!(scroll_target(100.0, 200.0, 500.0), 592.0);
    assert_eq!(scroll_target(100.0, 0.0, 50.0), 0.0);
}

/// The light/dark variant roundtrips through the edited file and the
/// panel switch maps dark back to the absent key.
#[test]
fn theme_mode_roundtrips_and_switches() {
    let mut s = fixture();
    s.load_from_json(r#"{"panel":{"theme_mode":"light","sections":[]}}"#)
        .expect("theme_mode must load");
    assert_eq!(s.config.theme_mode.as_deref(), Some("light"));
    let out = s.serialize().unwrap();
    assert!(out.contains("\"theme_mode\": \"light\""), "got: {out}");
    let _ = update(&mut s, Message::ThemeModeChanged("dark".to_string()));
    assert!(s.config.theme_mode.is_none(), "dark keeps the file minimal");
    let _ = update(&mut s, Message::ThemeModeChanged("light".to_string()));
    assert_eq!(s.config.theme_mode.as_deref(), Some("light"));
}

/// The output setting roundtrips and `auto` drops the key again.
#[test]
fn output_roundtrips_and_auto_clears() {
    let mut s = fixture();
    s.load_from_json(r#"{"panel":{"output":"HDMI-A-2","sections":[]}}"#)
        .expect("output must load");
    assert_eq!(s.config.output.as_deref(), Some("HDMI-A-2"));
    let out = s.serialize().unwrap();
    assert!(out.contains("\"output\": \"HDMI-A-2\""), "got: {out}");
    let _ = update(&mut s, Message::OutputChanged("auto".to_string()));
    assert!(s.config.output.is_none(), "auto is the absent key");
    let _ = update(&mut s, Message::OutputChanged("eDP-1".to_string()));
    assert_eq!(s.config.output.as_deref(), Some("eDP-1"));
}

/// Multiple panels: the first layout fills the root panel fields, extras
/// stay separate, and save writes the whole array again.
#[test]
fn panels_roundtrip_and_edit() {
    let mut s = fixture();
    s.load_from_json(
        r#"{"panel":{"sections":[
            {"name":"Workspace","sensors":[]},
            {"name":"Stats","sensors":[]}
        ]},"panels":[
            {"output":"HDMI-A-2","anchor":"top","sections":["Workspace"]},
            {"output":"eDP-1","anchor":"bottom","align":"fill","sections":["Stats"]}
        ]}"#,
    )
    .expect("panels must load");
    assert_eq!(s.config.output.as_deref(), Some("HDMI-A-2"));
    assert_eq!(s.config.visible_sections, Some(vec!["Workspace".to_string()]));
    assert_eq!(s.config.panels.len(), 1);
    assert_eq!(s.config.panels[0].output.as_deref(), Some("eDP-1"));
    assert_eq!(s.config.panels[0].sections_text, "Stats");

    let out = s.serialize().unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let layouts = json["panels"].as_array().expect("panels array");
    assert_eq!(layouts.len(), 2, "{out}");
    assert_eq!(layouts[1]["anchor"], "bottom");

    let _ = update(&mut s, Message::PanelAdded);
    assert!(
        s.field_err("panels[2].sections").is_some(),
        "an empty secondary panel is flagged"
    );
    let _ = update(&mut s, Message::PanelSectionsChanged(1, "Workspace, Stats".to_string()));
    assert_eq!(s.config.panels.len(), 2);
    assert!(
        s.field_err("panels[2].sections").is_none(),
        "sections clear the error"
    );
    assert_eq!(
        s.config.panels[1].sections,
        Some(vec!["Workspace".to_string(), "Stats".to_string()])
    );
    let _ = update(&mut s, Message::PanelRemoved(1));
    assert_eq!(s.config.panels.len(), 1);
    let _ = update(&mut s, Message::RootSectionsChanged("Stats".to_string()));
    assert_eq!(s.config.visible_sections, Some(vec!["Stats".to_string()]));
}
