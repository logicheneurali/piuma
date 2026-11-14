//! Boot: CLI, logging, the single instance lock, the config/theme load and
//! the iced_exwlshell daemon that spawns one bar per configured layout.

use std::collections::HashMap;
use std::path::PathBuf;

use iced::Color;
use iced_exwlshell::reexport::{
    KeyboardInteractivity, Layer, NewLayerShellSettings,
};
use iced_exwlshell::settings::LayerShellSettings;
use iced_exwlshell::daemon;
use iced_exwlshell::ExWlSettings;
use crate::tweaks;

use crate::graphs;
use crate::layouts;
use crate::hover::HoverGate;
use crate::geometry::{panel_anchor_for, panel_size, panel_thickness};
use crate::screensaver;
use crate::sensor_apply::config_file_state;
use crate::styles::resolve_styles;
use crate::output::{layout_output_option, monitor_names, screen_size_for, start_mode_for_layout};
/// Window namespace of the bar's layer surfaces.
fn namespace() -> String {
    String::from("piuma")
}

use crate::{
    app_theme, autostart, config, glass_blur, instance_lock, logging, subscription,
    themes, update, view, App, Command,
    Message, Panel, CONFIG, CONFIG_PATH, PANEL_SHOULD_CLOSE, SCREEN_HEIGHT,
    SCREEN_WIDTH,
};

/// Sibling `piuma-setup` of the running executable: both binaries are
/// installed in the same folder, so no PATH lookup is involved.
pub(crate) fn setup_binary_path(exe: &std::path::Path) -> PathBuf {
    exe.with_file_name("piuma-setup")
}

/// Starts the editor detached; a missing binary stays a log line (no
/// popup on the bar).
pub(crate) fn open_setup() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let path = setup_binary_path(&exe);
    crate::process::reaping(std::process::Command::new(&path));
}

// ── positioning helpers ─────────────────────────────────────────────


// ── style ───────────────────────────────────────────────────────────

fn style(_app: &App, _theme: &iced::Theme) -> iced::theme::Style {
    // Root stays transparent: the panel background is drawn by the inner
    // container, and a root background would force the surface to the whole
    // screen.
    iced::theme::Style {
        background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        text_color: _theme.palette().text,
    }
}

// ── main ────────────────────────────────────────────────────────────

pub(crate) fn run() -> Result<(), iced_exwlshell::Error> {
    // glibc raises its dynamic mmap threshold after the first multi-MB
    // free, so later image decodes (notification icons, wallpaper) come
    // from the heap and stay there: measured ~0.7 MB of RSS per icon,
    // never returned (88 icons ≈ +70 MB). A fixed trim threshold hands
    // the top of the heap back on free (+14 MB in the same test).
    #[cfg(target_env = "gnu")]
    unsafe {
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
    }

    // Parse CLI args for config path.
    let config_path = parse_config_arg();

    // Initialise logging (RUST_LOG level, UTC timestamps on stderr).
    logging::init();

    eprintln!("[piuma] Starting …");

    // The panel thread's death must end the process: the toolkit panics
    // instead of returning when the compositor cannot be reached
    // ("Cannot create context … NoCompositor"), and a main thread
    // polling its close flag forever is exactly what used to leave a
    // ghost bar behind. Other threads (sensors, D-Bus) only have their
    // panic logged: losing one of them does not invalidate the bar.
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic_hook(info);
        if std::thread::current().name() == Some("piuma-panel") {
            eprintln!("[piuma] panel thread died — signalling close");
            unsafe { PANEL_SHOULD_CLOSE = true; }
        }
    }));

    // One bar per session: replace a previous instance, if any. The
    // binding keeps the flock (and with it the lock file's pid) alive
    // until main() returns, which is exactly the process lifetime.
    let _instance_lock = instance_lock::acquire_instance_lock();

    // Load configuration once at startup; remember the file it came from so
    // it can be watched for hot reload.
    let (mut config, resolved_config_path) = config::load_config(config_path.clone());

    eprintln!(
        "[piuma] Config loaded: {} section(s) from {}",
        config.panel.sections.len(),
        resolved_config_path.display()
    );

    // Roadmap N2: launch the freedesktop autostart applications with the
    // bar (background thread; dormant unless the config enables it).
    // The config's per-app selection is pruned against the live
    // directories first — a removed .desktop must not outlive its file —
    // and the config is rewritten (atomically) before anything launches.
    if config.panel.autostart.unwrap_or(false) {
        let current = config.panel.autostart_disabled.clone().unwrap_or_default();
        let pruned = autostart::prune_disabled(&autostart::existing_files(), &current);
        if pruned != current {
            let mut updated = config.clone();
            updated.panel.autostart_disabled = (!pruned.is_empty()).then_some(pruned.clone());
            if let Err(err) = config::save_config(&resolved_config_path, &updated) {
                eprintln!(
                    "[piuma] autostart: cannot prune {}: {err}",
                    resolved_config_path.display()
                );
            }
        }
        autostart::spawn(pruned);
    }

    // The active theme lives in its own file (`~/.config/piuma/theme.json`)
    // and is applied over the config in memory only: the base copy stays
    // untouched for "custom", and `piuma.json` is never rewritten.
    let config_base = config.clone();
    let active_theme = themes::active();
    if let Some(slug) = active_theme {
        themes::apply(&mut config, slug);
        eprintln!("[piuma] Theme: {slug}");
    }

    // Roadmap N3: render the GTK theme bridge from the bar's own style
    // (writes/removes only piuma's own file; off = untouched).
    // The palette arrives with PaletteReady below, which resyncs.
    app_theme::sync(&resolved_config_path, &config.panel, None);
    // Session exit (logind/systemd send SIGTERM) restores the user files,
    // so the palette does not leak into a Plasma/GNOME session.
    app_theme::install_exit_cleanup(resolved_config_path.clone(), config.panel.clone());

    // One window per configured layout; the first one is the daemon's
    // initial surface, the others are opened from the boot task below.
    let layouts = config::panel_layouts(&config);
    let screens: Vec<(u32, u32)> = layouts
        .iter()
        .map(|layout| screen_size_for(layout.output.as_deref()))
        .collect();
    unsafe {
        SCREEN_WIDTH = screens[0].0;
        SCREEN_HEIGHT = screens[0].1;
    }
    for (index, layout) in layouts.iter().enumerate() {
        if let Some(name) = layout
            .output
            .as_deref()
            .filter(|o| !o.is_empty() && *o != "auto" && *o != "all")
        {
            if !monitor_names().iter().any(|output| output == name) {
                eprintln!(
                    "[piuma] panel {}: output {name:?} not found — falling back to the focused monitor",
                    index + 1
                );
            }
        }
    }

    let _ = CONFIG.set(config.clone());
    let _ = CONFIG_PATH.set(resolved_config_path.to_string_lossy().into_owned());

    let panel_config = CONFIG.get().cloned().unwrap_or_else(config::default_config);

    // Run the iced app on a dedicated thread so the close action can be
    // observed from the main thread without blocking the event loop.
    // The runtime reports the surfaces it creates on this channel: the
    // bar listens for the session-lock windows it makes for a lock
    // request (see `screensaver::on_shell`).
    let (shell_tx, shell_rx) = iced_exwlshell::shell::channel();
    let boot_base = config_base.clone();
    let handle = std::thread::Builder::new()
        .name("piuma-panel".to_string())
        .spawn(move || {
            let boot_layouts = layouts.clone();
            let boot_screens = screens.clone();
            let boot_base = boot_base.clone();
            let boot_shell = shell_rx.clone();
            let outcome = daemon(
                move || {
                    let cfg = CONFIG
                        .get()
                        .cloned()
                        .unwrap_or_else(config::default_config);
                    let layouts = boot_layouts.clone();
                    let screens = boot_screens.clone();
                    // Initial geometry (size, anchor, exclusive zone) must
                    // match what wl_settings applies at startup, so reloads
                    // only emit LayoutChange/ExclusiveZoneChange on change.
                    let size =
                        panel_size(&cfg, screens[0].0, layouts[0].anchor.as_deref());
                    let anchor = panel_anchor_for(&cfg, layouts[0].anchor.as_deref());
                    let zone = panel_thickness(&cfg) as i32;
                    let graphs = graphs::History::new(&cfg.panel.sections);
                    // Colour chains resolve once, off the tick (see
                    // `restyle`): the palette starts absent, so the
                    // configured defaults lead until one arrives.
                    let styles = resolve_styles(&cfg, None);
                    let state = Panel {
                        config: cfg.clone(),
                        config_base: boot_base.clone(),
                        active_theme,
                        theme_state: themes::state(),
                        sections: Vec::new(),
                        styles,
                        read_pending: false,
                        read_started: None,
                        read_pass: 0,
                        read_backoff: None,
                        cfg_gen: 0,
                        popup_resync: false,
                        hovered: None,
                        hover_gate: HoverGate::default(),
                        last_motion: None,
                        pending_hover: None,
                        first_anchor: layouts[0].anchor.clone(),
                        last_reload: config_file_state(),
                        last_size: size,
                        last_anchor: anchor,
                        last_exclusive_zone: zone,
                        last_cursor: None,
                        popup: None,
                        closing: Vec::new(),
                        popup_cards: 0,
                        popup_hover: None,
                        notif_scroll: HashMap::new(),
                        value_effects: HashMap::new(),
                        volume_drag: None,
                        tray_menu: None,
                        tray_menu_hover: None,
                        tray_menu_gen: 0,
                        notif_list: None,
                        taskbar_menu: None,
                        taskbar_menu_hover: None,
                        window_menu: None,
                        taskbar_hover: None,
                        bar_tooltip: None,
                        graphs,
                        graph_popup: None,
                        calendar: None,
                        wallpaper: Vec::new(),
                        wallpaper_hidden: Vec::new(),
                        corners: Vec::new(),
                        screensaver: Vec::new(),
                        screensaver_on: false,
                        screensaver_start: None,
                        screensaver_wait: false,
                        screensaver_retry: None,
                        screensaver_phases: Vec::new(),
                        screensaver_last: None,
                        screensaver_password: String::new(),
                        screensaver_error: None,
                        screensaver_pending: false,
                        screensaver_lock: screensaver::LockMode::Off,
                        shell: boot_shell.clone(),
                        screensaver_prompt_open: false,
                        caffeine: false,
                        bar_windows: Vec::new(),
                        last_glass: cfg.glass().is_some(),
                        power_menu: None,
                        theme_menu: None,
                        dashboard: None,
                        dash_probe: None,
                        dash_profile: None,
                        dash_volume: None,
                        dash_brightness: None,
                        dash_volume_drag: false,
                        dash_brightness_drag: false,
                        cover: None,
                        cover_url: None,
                        osd: None,
                        collapsed_sections: std::collections::HashSet::new(),
                        palette: None,
                        palette_seed: None,
                        palette_key: None,
                        hypr: None,
                        layout_state: layouts::LayoutState::default(),
                    };
                    let mut app = App {
                        state,
                        layouts,
                        screens,
                        windows: HashMap::new(),
                    };
                    // The daemon owns the first window; open one more
                    // layershell per extra panel and remember the mapping.
                    let mut task: Command<Message> = Command::none();
                    for index in 1..app.layouts.len() {
                        let layout = app.layouts[index].clone();
                        let (width, _) = app.screens[index];
                        let (id, open) = Message::layershell_open(NewLayerShellSettings {
                            size: panel_size(&cfg, width, layout.anchor.as_deref()),
                            layer: Layer::Top,
                            anchor: panel_anchor_for(&cfg, layout.anchor.as_deref()),
                            exclusive_zone: Some(panel_thickness(&cfg) as i32),
                            margin: None,
                            keyboard_interactivity: KeyboardInteractivity::None,
                            output_option: layout_output_option(&layout),
                            events_transparent: false,
                            namespace: Some(format!("piuma-{}", index + 1)),
                            blur_option: glass_blur(&cfg),
                        });
                        app.windows.insert(id, index);
                        app.state.bar_windows.push((id, layout.anchor.clone()));
                        task = Command::batch([task, open]);
                    }
                    (app, task)
                },
                namespace,
                update,
                view,
            )
            .subscription(subscription)
            .style(style)
            .wl_settings(ExWlSettings {
                shell_broadcast: shell_tx,
                layer_settings: LayerShellSettings {
                    size: panel_size(&panel_config, screens[0].0, layouts[0].anchor.as_deref()),
                    // Reserve exactly the configured thickness: a constant
                    // here would leave dead space whenever thickness != 32.
                    exclusive_zone: panel_thickness(&panel_config) as i32,
                    anchor: panel_anchor_for(&panel_config, layouts[0].anchor.as_deref()),
                    layer: Layer::Top,
                    start_mode: start_mode_for_layout(&layouts[0]),
                    blur_option: glass_blur(&panel_config),
                    ..Default::default()
                },
                ..Default::default()
            })
            .run();
            // The loop also ends when the compositor goes away: the dead
            // connection returns run() while main() keeps polling the close
            // flag forever — that is how a hyprland respawn used to leave a
            // ghost bar sleeping behind. Signal the same close the close
            // action does, so the process really exits.
            match outcome {
                Ok(()) => eprintln!("[piuma] event loop ended — signalling close"),
                Err(e) => eprintln!("[piuma] event loop ended ({e:?}) — signalling close"),
            }
            unsafe { PANEL_SHOULD_CLOSE = true; }
        })
        .expect("Failed to spawn piuma panel thread");

    // Poll for the close flag; exit the process when set.
    while !unsafe { PANEL_SHOULD_CLOSE } {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    eprintln!("[piuma] close_panel requested — exiting …");
    // Restore the gaps the bar patched at runtime before leaving.
    tweaks::restore_on_exit();

    // Detach the iced thread so the process exit terminates it.
    std::mem::forget(handle);
    Ok(())
}

/// Parse `-c <path>` or `--config <path>` from CLI args.
fn parse_config_arg() -> Option<String> {
    let args: Vec<String> = std::env::args().map(|s| s.to_string()).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if (arg == "-c" || arg == "--config")
            && i + 1 < args.len() {
                return Some(args[i + 1].clone());
            }
        i += 1;
    }
    None
}


