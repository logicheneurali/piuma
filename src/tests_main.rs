//! Unit tests for the bar: JSON round-trips, geometry, hover and popups.

use crate::{
    bar_line_height, config, cursor_in_rect, hover_is_fresh, icons_theme, palette,
    popup_card_h, popup_style, PopupStyle,
};
use crate::app::setup_binary_path;
use crate::hover::{HoverGate, HOVER_MOTION_GRACE};
use crate::output::{focused_output_in, monitor_names_json, screen_size_in, start_mode_for};
use iced_exwlshell::settings::StartMode;
use std::time::Instant;

/// The bar text keeps an even whole-pixel line box (the renderer floors
/// the origin and rounds the baseline, so a fractional box loses a
/// pixel of the vertical centring) and stays close to 1.2 × the size.
#[test]
fn bar_line_height_is_an_even_whole_pixel() {
    for size in 8..=32u32 {
        let height = bar_line_height(size);
        assert_eq!(height % 2.0, 0.0, "size {size}: {height}");
        assert!(
            (height - size as f32 * 1.2).abs() <= 1.0,
            "size {size}: {height}"
        );
    }
}

/// `panel.output` selects the monitor: the size must be that output's,
/// with the focused monitor as the fallback.
#[test]
fn output_selects_the_screen_size_and_start_mode() {
    let json = r#"[
        {"name":"HDMI-A-2","width":1920,"height":1080,"scale":1.0,"focused":false},
        {"name":"eDP-1","width":2560,"height":1600,"scale":2.0,"focused":true}
    ]"#;
    assert_eq!(screen_size_in(json, Some("HDMI-A-2")), Some((1920, 1080)));
    assert_eq!(
        screen_size_in(json, Some("eDP-1")),
        Some((1280, 800)),
        "physical pixels are divided by the output scale"
    );
    assert_eq!(screen_size_in(json, Some("auto")), Some((1280, 800)));
    assert_eq!(screen_size_in(json, None), Some((1280, 800)));
    assert_eq!(
        screen_size_in(json, Some("missing")),
        Some((1280, 800)),
        "a missing output falls back to the focused one"
    );

    let mut panel = config::default_config().panel;
    assert!(matches!(start_mode_for(&panel), StartMode::Active));
    panel.output = Some("auto".to_string());
    assert!(matches!(start_mode_for(&panel), StartMode::Active));
    panel.output = Some("HDMI-A-2".to_string());
    assert!(matches!(
        start_mode_for(&panel),
        StartMode::TargetScreen(name) if name == "HDMI-A-2"
    ));
    panel.output = Some("all".to_string());
    assert!(matches!(start_mode_for(&panel), StartMode::AllScreens));
}

/// The right-click entry point starts the installer from the same
/// folder as the running bar, whatever that folder is.
#[test]
fn setup_binary_is_the_sibling_of_the_bar() {
    assert_eq!(
        setup_binary_path(std::path::Path::new("/opt/piuma/bin/piuma")),
        std::path::PathBuf::from("/opt/piuma/bin/piuma-setup")
    );
    assert_eq!(
        setup_binary_path(std::path::Path::new("target/release/piuma")),
        std::path::PathBuf::from("target/release/piuma-setup")
    );
}

/// The hover dwell gate: an enter arms, its confirmation activates
/// exactly once — a leave on that sensor or a newer enter cancels
/// it (the stale token then never fires), while another sensor's
/// leave leaves the arm standing.
#[test]
fn hover_gate_confirms_only_unsuperseded_enters() {
    let mut gate = HoverGate::default();
    let a = gate.enter(0, 1);
    assert!(gate.confirm(9, 9, a.2).is_none(), "a foreign token never confirms");
    assert!(gate.confirm(a.0, a.1, a.2).is_some(), "the armed enter confirms");
    assert!(gate.confirm(a.0, a.1, a.2).is_none(), "exactly once");

    let b = gate.enter(0, 1);
    gate.leave(0, 1);
    assert!(gate.confirm(b.0, b.1, b.2).is_none(), "a cancelled enter never activates");

    let c = gate.enter(0, 1);
    gate.leave(1, 0);
    assert!(gate.confirm(c.0, c.1, c.2).is_some(), "another sensor's leave keeps the arm");

    let d1 = gate.enter(0, 1);
    let d2 = gate.enter(0, 1);
    assert!(gate.confirm(d1.0, d1.1, d1.2).is_none(), "the stale arm is superseded");
    assert!(gate.confirm(d2.0, d2.1, d2.2).is_some(), "the newest arm wins");
}

/// A hover only counts when the pointer moved with it: iced fires an
/// enter when a widget's bounds change under a still pointer too, and
/// that must not open a popup (the CPU pill growing under the cursor
/// opened its graph).
#[test]
fn hover_needs_motion_next_to_the_arm() {
    let now = Instant::now();
    assert!(hover_is_fresh(now, Some(now)), "the entering motion itself");
    assert!(
        hover_is_fresh(now, Some(now + HOVER_MOTION_GRACE / 2)),
        "the motion message may arrive right after the enter"
    );
    assert!(
        !hover_is_fresh(now, Some(now - std::time::Duration::from_secs(2))),
        "a bounds change under a still pointer: no popup"
    );
    assert!(!hover_is_fresh(now, None), "no motion at all: no popup");
}

/// The box check that drops an enter handed to a sensor the cursor is
/// nowhere near: inside counts, the borders compare half-open, and a
/// cursor never seen yet counts as outside.
#[test]
fn hover_requires_the_cursor_inside_the_sensor_box() {
    let rect = (60, 4, 110, 24);
    assert!(cursor_in_rect(Some((70.0, 14.0)), rect), "inside");
    assert!(cursor_in_rect(Some((60.0, 4.0)), rect), "top-left corner");
    assert!(!cursor_in_rect(Some((110.0, 14.0)), rect), "right edge excluded");
    assert!(!cursor_in_rect(Some((70.0, 24.0)), rect), "bottom edge excluded");
    assert!(
        !cursor_in_rect(Some((1268.0, 24.0)), rect),
        "the live misfire: far to the right of Layout"
    );
    assert!(!cursor_in_rect(None, rect), "no cursor seen: no hover");
}

/// The default popup style must keep the F3 geometry exactly (84px
/// cards): the surface size is computed from it, and any drift would
/// leave a dead (click-blocking) strip below the stack.
#[test]
fn popup_card_h_defaults_to_84_and_scales_with_style() {
    assert_eq!(popup_card_h(&PopupStyle::default()), 120);
    let styled = PopupStyle {
        font_size: 16,
        padding: 8,
        ..PopupStyle::default()
    };
    assert_eq!(popup_card_h(&styled), 144);
}


/// Roadmap N1 gate: no declared icon = no theme, even with
/// `panel.icon_theme` set — the icon machinery stays dormant. One
/// icon activates it, config value over the GTK fallback (that path
/// spawns `gsettings`, so it is deliberately not exercised here).
#[test]
fn icon_gate_runs_only_with_a_sensor_icon() {
    let mut cfg = crate::config::default_config();
    assert!(icons_theme(&cfg).is_none());
    cfg.panel.icon_theme = Some("Papirus".to_string());
    assert!(icons_theme(&cfg).is_none()); // still dormant
    cfg.panel.sections[0].sensors[0].icon = Some("cpu".to_string());
    assert_eq!(icons_theme(&cfg), Some("Papirus"));
}












/// Roadmap2 #1: the monitor list is the `name` field of every entry
/// of `hyprctl monitors -j`; junk or empty input yields no outputs
/// (the sync then just retries next tick).
#[test]
fn monitor_names_from_hyprctl_json() {
    let json = r#"[{"id":0,"name":"HDMI-A-2","x":0},{"id":1,"name":"eDP-1","x":1920}]"#;
    assert_eq!(monitor_names_json(json), vec!["HDMI-A-2", "eDP-1"]);
    assert!(monitor_names_json("not json at all").is_empty());
    assert!(monitor_names_json("[]").is_empty());
    assert!(monitor_names_json(r#"[{"id":0}]"#).is_empty());
}

/// The popup output follows the focused monitor (the one the bar
/// hover happened on), not whichever output the compositor calls
/// "last": with two monitors the graph popup used to open on the
/// other one.
#[test]
fn focused_output_takes_the_focused_monitor() {
    let json = r#"[{"id":0,"name":"HDMI-A-1","x":0,"y":0,"focused":false},
                   {"id":1,"name":"eDP-2","x":1920,"y":0,"focused":true}]"#;
    assert_eq!(
        focused_output_in(json),
        Some(("eDP-2".to_string(), 1920, 0))
    );
    assert!(focused_output_in("not json at all").is_none());
    assert!(focused_output_in("[]").is_none());
    assert!(focused_output_in(r#"[{"id":0,"name":"eDP-2"}]"#).is_none());
}


/// An active wallpaper palette replaces the notification card's
/// configured COLOURS (bg/border/text) with its tones, while the
/// numerics stay configured.
#[test]
fn popup_style_follows_the_wallpaper_palette() {
    let config: config::Config = serde_json::from_str(
        r##"{"panel":{"sections":[],"notifications":{"font_size":16,"bg_color":"#101010","border_color":"#ff5555","text_color":"#ffe066"}}}"##,
    )
    .unwrap();
    let plain = popup_style(&config, None);
    assert_eq!(plain.bg, config::parse_color("#101010").unwrap());
    let pal = palette::Palette {
        bg: "#000033".to_string(),
        card: "#000061".to_string(),
        card_hover: "#002d99".to_string(),
        text: "#d2d2e0".to_string(),
        text_hover: "#f4f4f6".to_string(),
    };
    let styled = popup_style(&config, Some(&pal));
    assert_eq!(styled.bg, config::parse_color("#000061").unwrap());
    assert_eq!(styled.border, config::parse_color("#002d99").unwrap());
    assert_eq!(styled.text, config::parse_color("#d2d2e0").unwrap());
    assert_eq!(styled.font_size, 16, "numerics stay configured");
}

/// The popup's tone keeps the configured colour's transparency: a
/// `#10101080` card renders the palette's hue at the same alpha.
#[test]
fn popup_style_keeps_the_configured_alpha() {
    let config: config::Config = serde_json::from_str(
        r##"{"panel":{"sections":[],"notifications":{"bg_color":"#10101080","text_color":"#ffe066"}}}"##,
    )
    .unwrap();
    let pal = palette::Palette {
        bg: "#000033".to_string(),
        card: "#000061".to_string(),
        card_hover: "#002d99".to_string(),
        text: "#d2d2e0".to_string(),
        text_hover: "#f4f4f6".to_string(),
    };
    let styled = popup_style(&config, Some(&pal));
    assert_eq!(styled.bg, config::parse_color("#00006180").unwrap());
    // An opaque configured colour stays opaque under the new hue.
    assert_eq!(styled.text, config::parse_color("#d2d2e0").unwrap());
}
