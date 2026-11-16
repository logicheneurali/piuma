//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.
//!
//! Configuration is loaded from a JSON file (CLI arg or default path).
//! See `src/config.rs` for the config schema.

use std::time::{Instant, SystemTime};

use iced::{
    Color, Event, Task as Command,
};

use iced_exwlshell::reexport::{
    BlurOption, KeyboardInteractivity, Layer, NewLayerShellSettings,
};
use iced_exwlshell::settings::LayerSize;
use iced_exwlshell::to_exwlshell_message;

mod app;
mod app_theme;
mod auth;
mod autostart;
mod calendar;
mod config;
mod corners;
mod dashboard;
mod dashboard_view;
mod dbusmenu;
mod effects;
mod events;
mod fill_sections;
mod geometry;
mod graphs;
mod hover;
mod hypr;
mod idle;
mod icons;
mod instance_lock;
mod layouts;
mod logging;
mod menus;
mod mpris;
mod osd;
mod output;
mod panel;
mod panel_view;
mod notifications;
mod palette;
mod pam_ffi;
mod playback;
mod power_menu;
mod process;
mod popup_anim;
mod popup_calendar;
mod popup_graph;
mod popup_notifications;
mod screensaver;
mod sensor_apply;
mod sensor_runner;
mod sensors;
mod styles;
mod text_util;
mod text_width;
mod theme_menu;
// The setup shares this module and reads more of it (names, the
// theme picker): the bar only needs to read and apply.
#[allow(dead_code)]
mod themes;
mod tooltip;
mod tray;
mod tweaks;
mod volume_ui;
mod wallpaper_ui;
mod widgets;
mod window_menu;
mod windows;
mod workspace;
#[allow(dead_code)]
mod xdg;

use corners::sync_corners;
use geometry::{
    corner_radius, panel_anchor, panel_anchor_for, panel_size, panel_thickness, placement_rect,
};
use menus::{
    fit_menu_labels, menu_arrow_h, menu_placement, menu_row_h, menu_size_shown,
    taskbar_menu_size, taskbar_menu_view, tray_menu_view, TaskbarRow, MENU_MAX_ROWS, MENU_PAD,
};
use popup_calendar::calendar_page;
use popup_notifications::{
    notif_list_row_h, popup_dismiss, scroll_rows, sync_popup,
    toggle_notif_list, NOTIF_LIST_MAX, NOTIF_LIST_ROWS,
};
use styles::restyle;
use text_util::panel_font;
use tooltip::{tooltip_close, tooltip_open};
use wallpaper_ui::{sync_palette, sync_wallpaper};
// Layout padding lives with the icon sizing helpers (icons.rs): the view
// and the row-height size formula must read the same number.

// ── globals ────────────────────────────────────────────────────────

/// Startup config, handed to the first window factory call and to any
/// later surface: written once before the threads start, read from any
/// thread afterwards (`OnceLock` keeps that sound without `unsafe`).
pub(crate) static CONFIG: std::sync::OnceLock<config::Config> = std::sync::OnceLock::new();
/// The config file watched for hot reloads (string for the log lines);
/// also written once at startup.
pub(crate) static CONFIG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub(crate) static mut PANEL_SHOULD_CLOSE: bool = false;
pub(crate) static mut SCREEN_WIDTH: u32 = 1920;
pub(crate) static mut SCREEN_HEIGHT: u32 = 1080;

// ── colours / defaults ────────────────────────────────────────────

pub(crate) use app::open_setup;
pub(crate) use events::subscription;
pub(crate) use icons::icons_theme;
pub(crate) use widgets::{bar_line_height, glass_blur, spawn, styled_box};
pub(crate) use panel::{
    App, BarTooltip, CalendarPopup, GraphPopup, NotifList, Panel, SectionData, SectionStyle,
    SensorStyle, SensorValue, TaskbarMenu, TrayMenu,
};

pub(crate) use panel_view::{view, BAR_ID};
pub(crate) use styles::{popup_card_h, popup_style, PopupStyle};
pub(crate) use sensor_apply::{
    apply_active_theme, apply_read, maybe_reload, refresh_tray_and_notif,
    spawn_read, Reload,
};

pub(crate) use output::{
    focused_output, focused_screen_size, monitor_names, popup_output_option,
};

pub(crate) use hover::{
    cursor_in_rect, hover_is_fresh, hover_sync, FindSensorRect,
};

pub(crate) const DEFAULT_FONT: &str = "Noto Sans";
pub(crate) const DEFAULT_FONT_SIZE: u32 = 14;
/// Gap between two sensor boxes inside a section: the same rhythm on
/// both axes, whether the sensors sit in a row or a stacked column.
const SENSOR_SPACING: f32 = 8.0;
/// Dwell before a sensor hover activates (style, graph/calendar
/// popup): an enter only takes effect once the pointer has rested
/// this long, so an involuntary sweep across the bar triggers nothing.
const SENSOR_HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(300);
/// A sensor pass that has not answered after this long is considered
/// stuck (the built-in commands have no timeout).
const PASS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// Retry delay after a stuck sensor pass.
const STUCK_RETRY: std::time::Duration = std::time::Duration::from_secs(30);
pub(crate) const DEFAULT_BG_COLOR: Color = Color::from_rgba(0.06, 0.06, 0.08, 0.85);
pub(crate) const DEFAULT_TEXT_COLOR: Color = Color::from_rgb(0.88, 0.88, 0.94);

/// Internal action identifiers.
pub(crate) const ACTION_CLOSE_PANEL: &str = "close_panel";

// ── model ───────────────────────────────────────────────────────────

#[to_exwlshell_message]
#[derive(Debug, Clone)]
pub(crate) enum Message {
    /// Marks a second edge: the tick handler reloads, polls and advances.
    Tick,
    /// One animation frame while a value effect is alive: only prunes
    /// expired effects; the view computes their progress.
    EffectFrame,
    /// One frame of a popup's slide (see `popup_anim`).
    PopupFrame,
    /// A popup's close slide finished: destroy its surface and drop the
    /// ghost.
    PopupClosed(iced::window::Id),
    /// The tick's data pass finished on a worker thread: the config
    /// generation it read against (a reply from an older config is
    /// dropped, see `spawn_read`) and every sensor's reading.
    SensorsReady(u64, u64, sensor_runner::Reads),
    /// The wallpaper palette finished extracting: the tones (or `None`
    /// on a failed decode) and the file key they came from.
    PaletteReady(Option<palette::Palette>, (String, SystemTime)),
    /// A cursor/input event with the id of the window it happened on:
    /// coordinates are per-window and only the bar's own belong into
    /// `last_cursor`.
    IcedEvent(iced::window::Id, Event),
    /// A sensor entered/exited the hover state: (section_index, sensor_index, hovered).
    SensorHovered(usize, usize, bool),
    /// An armed sensor enter's dwell elapsed (carrying its sequence
    /// token): the hover activates only if no leave or newer enter has
    /// superseded it (see `HoverGate`).
    SensorHoverConfirmed(usize, usize, u64),
    /// Answer of the sensor-box query `FindSensorRect`: (section,
    /// sensor, bar-relative rect). The hover activates only when the
    /// cursor is inside it (iced can fire an enter whose bounds changed
    /// under a still pointer).
    SensorRectFound(usize, usize, (i32, i32, i32, i32)),
    /// The idle notification fired (`true`) or the seat woke up
    /// (`false`): the screensaver starts or closes (see `screensaver`).
    Idle(bool),
    /// Answer of the gate check that starts the screensaver: `true` =
    /// blocked (a fullscreen window or media playing).
    ScreensaverBlocked(bool),
    /// The setup's button (SIGUSR1): show the overlay now.
    ScreensaverManual,
    /// A surface event from the Wayland runtime: the session-lock windows
    /// it creates and the compositor's answer to the lock request.
    Shell(iced_exwlshell::shell::ShellEvent),
    /// One animation frame of the overlay.
    ScreensaverFrame,
    /// Enter pressed or the Unlock button: check the password.
    ScreensaverSubmit,
    /// PAM answer for the submitted password.
    ScreensaverAuthResult(Result<(), String>),
    /// One row of the power menu.
    PowerAction(power_menu::Action),
    /// The theme picked in the theme menu; `None` = custom colours.
    ThemeSelected(Option<String>),
    /// The dashboard's scrim or ✕ was clicked.
    DashboardClose,
    /// One dashboard control (stateful tile or transport button).
    DashboardAction(dashboard::Action),
    /// The volume slider moved; the mixer is written on release.
    DashboardVolume(f32),
    DashboardVolumeRelease,
    /// The brightness slider moved; written on release.
    DashboardBrightness(f32),
    DashboardBrightnessRelease,
    /// The cover decode finished: the art URL and its handle (`None` on
    /// an unreadable file).
    DashboardCover(String, Option<iced::widget::image::Handle>),
    /// The dot handle of a collapsible section: collapse or expand it.
    SectionToggled(usize),
    /// A sensor was clicked: (section_index, sensor_index, action).
    SensorPressed(usize, usize, Option<String>),
    /// A `command:` sensor action came back from its startup grace: the
    /// reason when it failed early, `None` when it ran or kept running.
    ActionFinished(Option<String>),
    /// Right-click on the bar outside any widget that owns that button
    /// (tray/taskbar keep theirs): open the sibling `piuma-setup`.
    OpenSetup,
    /// A workspace button was clicked: focus that workspace (roadmap2 #4).
    WorkspaceClicked(String),
    /// Wheel scrolled over a sensor: (section_index, sensor_index, up);
    /// `None` for scroll events that carry no usable delta.
    SensorScrolled(usize, usize, Option<bool>),
    /// The graphical volume meter moved (click/drag): update the local
    /// percentage now; the mixer waits for the release.
    VolumeChanged(usize, usize, f32),
    /// The graphical volume meter was released: write its displayed
    /// percentage to the sink.
    VolumeCommitted(usize, usize),
    /// Wheel over the notification list popup: signed pixel delta-y
    /// (positive = towards the newest rows).
    NotifListScrolled(f32),
    /// Top/bottom arrow of the notification list: one row up/down.
    NotifListArrow(bool),
    /// Row clicked in the notification list popup: run that entry's
    /// spec `default` action and close the popup.
    NotifListClicked(u32),
    /// The list header's do-not-disturb switch.
    NotifDndToggled,
    /// The list header's Clear: drop the whole notification history.
    NotifListClear,
    /// Taskbar icon pressed (left): focus that window (with its
    /// workspace).
    TaskbarLeft(String),
    /// Taskbar icon pressed (middle): request that window's graceful
    /// close.
    TaskbarMiddle(String),
    /// Taskbar icon pressed (right): open the move-to-workspace menu
    /// for that window, under the cursor.
    TaskbarRight(String),
    /// Workspace chosen in the taskbar menu: move the window there
    /// silently and close the menu.
    TaskbarMenuWs(u32),
    /// Window action chosen in the taskbar menu: run it on the menu's
    /// window and close the menu.
    TaskbarMenuAction(window_menu::Action),
    /// Row under the cursor in the taskbar menu (`None` outside every
    /// row).
    TaskbarMenuHover(Option<TaskbarRow>),
    /// Menu row the cursor left: clears only if it is still the hovered
    /// one — widget order can deliver this exit after the next row's
    /// enter (iced visits the tree in order), and a stale exit must not
    /// drop the new hover.
    TaskbarMenuLeft(TaskbarRow),
    /// The floating menu's window changed (focus, geometry, flags):
    /// `None` = nothing focused. Comes from the event-socket thread.
    WindowMenuSync(Option<window_menu::Target>),
    /// One action pressed on the floating menu.
    WindowMenuAction(window_menu::Action),
    /// Floating-menu button under the cursor (`None` = outside).
    WindowMenuHover(Option<window_menu::Action>),
    /// Floating-menu button the cursor left (stale-exit guard).
    WindowMenuLeft(window_menu::Action),
    /// One glide frame of the floating menu.
    WindowMenuFrame,
    /// The window menu's idle threshold crossed (`true`) or input resumed
    /// (`false`): fade it out or back in.
    WindowMenuIdle(bool),
    /// Taskbar icon under the cursor: the hover fill plus its title
    /// tooltip (empty title → no tooltip window).
    TaskbarIconEnter {
        address: String,
        title: String,
    },
    /// Taskbar icon the cursor left: clears that icon's hover (stale
    /// exit guard, tree order) and closes its tooltip (same rule).
    TaskbarIconLeave {
        address: String,
    },
    /// Show a tooltip window for a bar source (`id`: tray `bus:path`)
    /// just off the bar's edge — never inside the bar surface, which is
    /// only `thickness` tall.
    TooltipShow {
        id: String,
        text: String,
    },
    /// The source left: close only if it still owns the tooltip (a
    /// stale exit must not drop the next one).
    TooltipHide {
        id: String,
    },
    /// Left the taskbar menu's surface: close it.
    TaskbarMenuClosed,
    /// Tray state changed (item added/removed/updated): rebuild now.
    TrayItemsChanged,
    /// A notification arrived, was replaced, expired or closed: rebuild.
    NotificationsChanged,
    /// Tray icon pressed: (bus, path, button); `tray.rs` computes the
    /// screen coordinates when the action is spawned.
    TrayIconPressed(String, String, tray::TrayButton),
    /// Wheel over a tray icon: (bus, path, scroll units + orientation);
    /// `None` for deltas that carry no usable delta.
    TrayIconScrolled(String, String, Option<(i32, bool)>),
    /// The DBusMenu of a pressed tray icon arrived: open the menu
    /// window at the output-relative point `at` — unless `gen` is stale
    /// (a newer press superseded this fetch) or the item is gone.
    TrayMenuLoaded {
        bus: String,
        path: String,
        menu_path: String,
        at: (i32, i32),
        gen: u64,
        entries: Result<Vec<dbusmenu::MenuEntry>, String>,
    },
    /// A menu row was clicked: report it to the app and close the menu.
    TrayMenuClicked(i32),
    /// Wheel over the menu: pixel delta, whole rows move `offset`.
    TrayMenuScrolled(f32),
    /// Top/bottom arrow clicked: move the offset by one row.
    TrayMenuArrow(bool),
    /// Cursor entered/left a clickable menu row (hover styling).
    TrayMenuEnter(i32),
    TrayMenuLeave(i32),
    /// The cursor left the menu surface: dismiss it.
    TrayMenuClosed,
    /// The cursor left the graph popup: dismiss it unless the cursor
    /// went back to its own sensor (the popup follows the hover).
    GraphPopupExited,
    /// The calendar popup's arrows page the shown month by one.
    CalendarPrev,
    CalendarNext,
    /// The cursor left the calendar popup's surface: the tick's grace
    /// decides the close (three seconds on neither sensor nor popup).
    CalendarExited,
    /// Click on a popup card: run its spec `default` action, dismiss it.
    PopupCardClicked(u32),
    /// Click on a popup card's ✕: dismiss it only (spec reason 2).
    PopupCardDismissed(u32),
    /// Cursor entered a popup card (hover styling, both card targets).
    PopupCardEnter(u32),
    /// Cursor left the popup card under it.
    PopupCardLeave,
    /// Notification body scrolled: (card id, absolute offset, max offset).
    NotifCardScrolled(u32, f32, f32),
    /// Notification body arrow: (card id, up/down).
    NotifCardArrow(u32, bool),
}

/// Top-level update: every panel window shares one `Panel` state (one set
/// of readings, tray items and notifications), so the messages need no
/// panel index; the layouts only change rendering and window placement.
pub(crate) fn update(app: &mut App, message: Message) -> Command<Message> {
    update_panel(&mut app.state, message)
}

fn update_panel(panel: &mut Panel, message: Message) -> Command<Message> {
    match message {
        Message::OpenSetup => {
            open_setup();
            Command::none()
        }
        Message::EffectFrame => {
            let now = Instant::now();
            panel
                .value_effects
                .retain(|_, start| now.saturating_duration_since(*start) < effects::DURATION);
            Command::none()
        }
        Message::Tick => {
            let now = Instant::now();
            // Hot reload: pick up config file changes (content and, when
            // layout/align differ from what is applied, surface geometry).
            let mut geometry_changed = false;
            let mut zone_changed = false;
            // Any reload re-anchors/resizes the popup too: the popup style
            // (font/padding) lives in the config without affecting the bar's
            // own geometry, so its reposition rides on this flag.
            let mut config_reloaded = false;
            // Theme changed (`theme.json` written by the setup or the
            // `theme` sensor): re-resolve the colours from the saved base
            // config and keep the GTK bridge in step. The file is only
            // read here — the bar never writes it.
            let theme_state = themes::state();
            if theme_state != panel.theme_state {
                panel.theme_state = theme_state;
                let slug = themes::active();
                if slug != panel.active_theme {
                    panel.active_theme = slug;
                    apply_active_theme(panel);
                    if let Some(path) = CONFIG_PATH.get() {
                        app_theme::sync(
                            std::path::Path::new(path),
                            &panel.config.panel,
                            panel.palette.as_ref(),
                        );
                    }
                }
            }
            let mut reload_toast = None;
            match maybe_reload(panel) {
                Some(Reload::Applied(reloaded, path, errors)) => {
                    config_reloaded = true;
                    panel.config_base = *reloaded;
                    // An in-flight data pass was flattened against the
                    // previous sensor list: its reply gets dropped.
                    panel.cfg_gen += 1;
                    // The colour chains follow the new config (see
                    // `restyle`); the readings and the geometry below
                    // don't need them, so one resolve covers the tick.
                    apply_active_theme(panel);
                    // The graph history follows the new sensor set: one
                    // ring per `draw_graph` column of the new config.
                    panel.graphs = graphs::History::new(&panel.config.panel.sections);
                    // Roadmap N3: keep the generated GTK bridge in step with
                    // hot reloads (rewritten only when its content changes).
                    app_theme::sync(
                        std::path::Path::new(&path),
                        &panel.config.panel,
                        panel.palette.as_ref(),
                    );
                    let size = panel_size(
                        &panel.config,
                        unsafe { SCREEN_WIDTH },
                        panel.first_anchor.as_deref(),
                    );
                    let anchor = panel_anchor_for(&panel.config, panel.first_anchor.as_deref());
                    let zone = panel_thickness(&panel.config) as i32;
                    if size != panel.last_size || anchor != panel.last_anchor {
                        panel.last_size = size;
                        panel.last_anchor = anchor;
                        geometry_changed = true;
                    }
                    if zone != panel.last_exclusive_zone {
                        panel.last_exclusive_zone = zone;
                        zone_changed = true;
                    }
                    reload_toast = Some(if errors.is_empty() {
                        "Config reloaded".to_string()
                    } else {
                        format!(
                            "Config: {} ignored value{} — {}",
                            errors.len(),
                            if errors.len() == 1 { "" } else { "s" },
                            errors[0].path
                        )
                    });
                }
                Some(Reload::Failed(reason)) => {
                    reload_toast = Some(format!("Invalid config: {reason}"));
                }
                None => {}
            }

            // Geometry or config moved: the reply that carries the
            // fresh sections re-anchors the popup on them
            // (`apply_read` consumes this). The data pass itself runs
            // on a worker — the UI thread never forks a sensor.
            panel.popup_resync |= geometry_changed || config_reloaded;
            let mut cmds: Vec<Command<Message>> = Vec::new();
            if let Some(text) = reload_toast {
                cmds.push(osd::toast(panel, &text));
            }
            // With multiple layer surfaces the runtime routes actions by
            // surface id: target the bar explicitly. Its id is captured on
            // the first render, which always precedes this Tick (the guard
            // only covers a theoretical startup race).
            if let Some(bar_id) = BAR_ID.get().copied() {
                if geometry_changed {
                    // Resize/re-anchor the layer surface at runtime; the view
                    // already uses the new config for content and alignment.
                    cmds.push(Command::done(Message::LayoutChange {
                        id: bar_id,
                        anchor: panel.last_anchor,
                        size: panel.last_size,
                    }));
                }
                if zone_changed {
                    // Keep hyprland's reserved space in step with the thickness.
                    cmds.push(Command::done(Message::ExclusiveZoneChange {
                        id: bar_id,
                        zone_size: panel.last_exclusive_zone,
                    }));
                }
                // Glass: the surfaces were created with the right blur at
                // startup; a reload that flips the tweak updates them.
                let glass = panel.config.glass().is_some();
                if glass != panel.last_glass {
                    panel.last_glass = glass;
                    let option = if glass {
                        BlurOption::FullRegion
                    } else {
                        BlurOption::None
                    };
                    let mut ids = vec![bar_id];
                    ids.extend(panel.bar_windows.iter().map(|(id, _)| *id));
                    for id in ids {
                        cmds.push(Command::done(Message::BlurOptionChange { id, option: option.clone() }));
                    }
                }
            }
            // An open graph popup dies with a reload too: its columns
            // were rebuilt underneath it (and its sensor may be gone).
            // The calendar popup goes with it: its month came from the
            // previous config's sensor.
            if config_reloaded {
                panel.volume_drag = None;
                panel.value_effects.clear();
                if let Some(gp) = panel.graph_popup.take() {
                    cmds.push(popup_anim::close(
                        panel,
                        gp.id,
                        popup_anim::Ghost::Graph(gp),
                    ));
                }
                if let Some(cp) = panel.calendar.take() {
                    cmds.push(popup_anim::close(
                        panel,
                        cp.id,
                        popup_anim::Ghost::Calendar(cp),
                    ));
                }
            }
            // The popup follows the active set (and re-anchors with the
            // bar on a geometry change) — one place drives its window.
            if let Some(cmd) = sync_popup(panel, geometry_changed || config_reloaded) {
                cmds.push(cmd);
            }
            // The wallpaper surface follows the config on the same
            // cadence: the first tick materializes it after startup,
            // later ticks track enable/path changes (a cheap compare).
            cmds.push(sync_wallpaper(panel));
            // The corner masks ride the same tick (roadmap angoli):
            // enable/radius/output changes reopen the squares cheaply.
            cmds.push(sync_corners(panel));
            // The idle screensaver follows its tweak and the monitor
            // list; while its overlay covers the outputs the readings
            // are not drawn anywhere, so the pass is skipped.
            cmds.push(screensaver::sync(panel));
            // The floating window menu follows its tweak the same way:
            // created on enable, closed on disable, re-placed on edits.
            cmds.push(window_menu::sync(panel));
            // A fullscreen window covers the bar and its tooltip with
            // it, and the resting pointer never sends the source's exit.
            cmds.push(tooltip::sync(panel));
            // The volume/brightness OSD and the power menu close
            // themselves after their linger.
            cmds.push(osd::sync(panel));
            cmds.push(power_menu::sync(panel));
            cmds.push(theme_menu::sync(panel));
            // The derived palette follows the same cadence: extracted
            // off-thread only when the wallpaper file changes, dropped
            // when the option turns off.
            cmds.push(sync_palette(panel));
            // A pass that has not answered by now is stuck in a built-in
            // command: abandon it (its late reply is dropped by token) and
            // retry after a pause.
            if panel.read_pending
                && panel
                    .read_started
                    .is_some_and(|start| now.duration_since(start) >= PASS_DEADLINE)
            {
                let waited = panel
                    .read_started
                    .map(|start| start.elapsed().as_secs())
                    .unwrap_or_default();
                eprintln!(
                    "[piuma] sensor pass stuck for {waited}s — retrying in {}s",
                    STUCK_RETRY.as_secs()
                );
                panel.read_pending = false;
                panel.read_started = None;
                panel.read_pass += 1;
                panel.read_backoff = Some(now + STUCK_RETRY);
            }
            // The data pass (compositor snapshot + every reading) runs
            // on a worker; its reply rebuilds the sections. A pass
            // already in flight covers this second — one at a time
            // keeps replies ordered.
            let retry_ready = panel.read_backoff.is_none_or(|at| now >= at);
            if !panel.read_pending && !panel.screensaver_on && retry_ready {
                cmds.push(spawn_read(panel));
            }
            Command::batch(cmds)
        }
        Message::SensorsReady(cfg_gen, pass, data) => {
            if pass != panel.read_pass {
                // A pass abandoned by the watchdog answered late: its
                // readings are stale and a newer pass owns the state.
                return Command::none();
            }
            panel.read_pending = false;
            panel.read_started = None;
            if cfg_gen != panel.cfg_gen {
                // The config moved while this read ran: its readings
                // no longer line up with the sensor list. Drop them
                // and read again at once.
                return spawn_read(panel);
            }
            panel.read_backoff = None;
            Command::batch(apply_read(panel, data))
        }
        Message::PaletteReady(pal, key) => {
            // The next view applies the tones — or resolves the user's
            // own colours again when the extraction failed (`None`).
            panel.palette_seed = pal;
            panel.palette = palette::active(panel.palette_seed.as_ref(), &panel.config.panel);
            panel.palette_key = Some(key);
            // The palette feeds the same colour chains as the config
            // (`restyle`): every chain re-resolves once, not per tick.
            restyle(panel);
            // The GTK bridge renders the same tones: re-render it too
            // (the write is a no-op when the content is unchanged).
            if let Some(path) = CONFIG_PATH.get() {
                app_theme::sync(
                    std::path::Path::new(path),
                    &panel.config.panel,
                    panel.palette.as_ref(),
                );
            }
            Command::none()
        }
        Message::IcedEvent(id, e) => {
            // While locked, input never unlocks: only the password does.
            // The prompt surface asks for keyboard focus when it opens.
            if power_menu::is_window(panel, id)
                && matches!(e, Event::Mouse(iced::mouse::Event::CursorLeft))
            {
                return power_menu::close(panel);
            }
            if power_menu::settled(panel)
                && BAR_ID.get().copied() == Some(id)
                && matches!(e, Event::Mouse(iced::mouse::Event::ButtonPressed { .. }))
            {
                return power_menu::close(panel);
            }
            if theme_menu::is_window(panel, id)
                && matches!(e, Event::Mouse(iced::mouse::Event::CursorLeft))
            {
                return theme_menu::close(panel);
            }
            if theme_menu::settled(panel)
                && BAR_ID.get().copied() == Some(id)
                && matches!(e, Event::Mouse(iced::mouse::Event::ButtonPressed { .. }))
            {
                return theme_menu::close(panel);
            }
            if screensaver::is_window(panel, id) {
                // The card appears on the first real input: until then
                // the lock is just the grid. Key presses then type into
                // the password (the card has no focusable widget).
                if !panel.screensaver_prompt_open && screensaver::reveal_input(&e) {
                    panel.screensaver_prompt_open = true;
                }
                if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    text,
                    ..
                }) = &e
                {
                    return screensaver::type_key(panel, key, text.as_deref());
                }
                return Command::none();
            }
            // Mouse enter/leave/press are handled per-sensor by MouseArea,
            // which emits Message::SensorHovered / SensorPressed. The cursor
            // position itself is remembered for tray activation hints — but
            // only from the bar's window: the layer popups (graph, tray menu)
            // report their own origin, and their coordinates leaked into
            // `last_cursor`, reopening the graph popup in a wrong place.
            match e {
                Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    if let Some(gp) = panel.graph_popup.as_mut() {
                        if gp.id == id {
                            // Position-checked both ways, like the
                            // calendar's: inside keeps the popup alive,
                            // a point outside clears the flag — the
                            // widget's exit alone can be missed and a
                            // one-way flag pinned the popup open for good.
                            let w = (gp.rect.2 - gp.rect.0) as f32;
                            let h = (gp.rect.3 - gp.rect.1) as f32;
                            gp.pointer_on_popup =
                                (0.0..w).contains(&position.x) && (0.0..h).contains(&position.y);
                        }
                    }
                    if let Some(cp) = panel.calendar.as_mut() {
                        if cp.id == id {
                            // Position-checked both ways: inside keeps the
                            // popup alive, a point outside clears the flag —
                            // the widget's exit alone can be missed (see the
                            // field's note) and a one-way flag pinned the
                            // popup open for good.
                            let w = (cp.rect.2 - cp.rect.0) as f32;
                            let h = (cp.rect.3 - cp.rect.1) as f32;
                            cp.pointer_on_popup = (0.0..w).contains(&position.x)
                                && (0.0..h).contains(&position.y);
                        }
                    }
                    if BAR_ID.get().copied() == Some(id) {
                        panel.last_cursor = Some((position.x, position.y));
                        panel.last_motion = Some(Instant::now());
                        // The hover can process before this event (a sensor
                        // hovered on the very first motion): retry the popup
                        // opens now that a position exists.
                        if let Some((section, sensor)) = panel.hovered {
                            return hover_sync(panel, section, sensor, true);
                        }
                    }
                }
                // The pointer left this window: for both popup surfaces
                // the compositor's own leave is authoritative — clear
                // the flag even where the widget's `on_exit` never fired.
                Event::Mouse(iced::mouse::Event::CursorLeft) => {
                    if let Some(gp) = panel.graph_popup.as_mut() {
                        if gp.id == id {
                            gp.pointer_on_popup = false;
                        }
                    }
                    if let Some(cp) = panel.calendar.as_mut() {
                        if cp.id == id {
                            cp.pointer_on_popup = false;
                        }
                    }
                }
                _ => {}
            }
            Command::none()
        }
        Message::TrayItemsChanged => {
            // An item appeared/vanished/was updated: rebuild the sections at
            // once instead of waiting for the next tick (same full path).
            // A menu open for a vanished item closes with it — its rows
            // would only emit failing events now.
            let mut cmds: Vec<Command<Message>> = Vec::new();
            if let Some((_, menu)) = panel.tray_menu.as_ref() {
                let alive = tray::items()
                    .iter()
                    .any(|i| i.bus == menu.bus && i.path == menu.path);
                if !alive {
                    let (id, menu) = panel.tray_menu.take().unwrap();
                    panel.tray_menu_hover = None;
                    cmds.push(popup_anim::close(
                        panel,
                        id,
                        popup_anim::Ghost::Tray(menu),
                    ));
                }
            }
            // Only the tray sensors change with the items — a full
            // tick would re-run (and re-fork) every other sensor for
            // an icon that merely reported a new state.
            refresh_tray_and_notif(panel);
            Command::batch(cmds)
        }
        Message::NotificationsChanged => {
            // A notification arrived, was replaced, expired or closed:
            // only the bell's counter moves (see `refresh_tray_and_notif`).
            // While the list is open the deliveries are already in front
            // of the user, so they count as read.
            if panel.notif_list.is_some() {
                notifications::mark_read();
            }
            refresh_tray_and_notif(panel);
            Command::none()
        }
        Message::PopupCardClicked(id) => {
            // Whole-card click = the spec's default action, then dismiss.
            notifications::invoke_id(id);
            popup_dismiss(panel, id)
        }
        Message::PopupCardDismissed(id) => popup_dismiss(panel, id),
        Message::PopupCardEnter(id) => {
            panel.popup_hover = Some(id);
            Command::none()
        }
        Message::PopupCardLeave => {
            panel.popup_hover = None;
            Command::none()
        }
        Message::NotifCardScrolled(id, offset, max) => {
            panel.notif_scroll.insert(id, (offset, max));
            Command::none()
        }
        Message::NotifCardArrow(id, up) => {
            let step = popup_style(&panel.config, panel.palette.as_ref()).font_size as f32 * 3.0;
            iced_runtime::widget::operation::scroll_by::<Message>(
                iced::widget::Id::from(format!("notif-card-{id}")),
                iced::widget::scrollable::AbsoluteOffset {
                    x: 0.0,
                    y: if up { -step } else { step },
                },
            )
        }
        Message::TrayIconPressed(bus, path, button) => {
            // Foreground thread only queues the D-Bus work (tray.rs /
            // dbusmenu spawn their own threads), so the UI never blocks
            // on the item.
            let target = tray::click_target(&bus, &path, button);
            let mut cmds: Vec<Command<Message>> = Vec::new();
            panel.tray_menu_gen += 1;
            let gen = panel.tray_menu_gen;
            // Any open menu closes on this press; pressing the SAME icon
            // in menu mode just toggles it (reopen-on-the-next-press).
            if let Some((id, menu)) = panel.tray_menu.take() {
                panel.tray_menu_hover = None;
                let same = menu.bus == bus
                    && menu.path == path
                    && target == tray::ClickTarget::Menu;
                cmds.push(popup_anim::close(
                    panel,
                    id,
                    popup_anim::Ghost::Tray(menu),
                ));
                if same {
                    return Command::batch(cmds);
                }
            }
            match target {
                tray::ClickTarget::Action => {
                    if let Some((x, y)) = tray::hint_point(panel) {
                        tray::spawn_action(&bus, &path, button, x, y);
                    }
                }
                tray::ClickTarget::Menu => {
                    // Menu path or cursor missing → the spec call is the
                    // only fallback left (its failure gets logged).
                    match (tray::menu_path(&bus, &path), tray::cursor_output_point(panel)) {
                        (Some(menu_path), Some(at)) => {
                            let (b, p, m) = (bus.clone(), path.clone(), menu_path.clone());
                            cmds.push(Command::perform(
                                dbusmenu::fetch(bus.clone(), menu_path),
                                move |entries| Message::TrayMenuLoaded {
                                    bus: b.clone(),
                                    path: p.clone(),
                                    menu_path: m.clone(),
                                    at,
                                    gen,
                                    entries,
                                },
                            ));
                        }
                        _ => {
                            if let Some((x, y)) = tray::hint_point(panel) {
                                tray::spawn_action(&bus, &path, button, x, y);
                            }
                        }
                    }
                }
            }
            if cmds.is_empty() {
                Command::none()
            } else {
                Command::batch(cmds)
            }
        }
        Message::TrayMenuLoaded {
            bus,
            path,
            menu_path,
            at,
            gen,
            entries,
        } => {
            // Superseded by a newer press while the layout was in flight.
            if gen != panel.tray_menu_gen {
                return Command::none();
            }
            let mut entries = match entries {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("[piuma] tray: menu fetch failed: {e}");
                    return Command::none();
                }
            };
            if entries.is_empty() {
                eprintln!("[piuma] tray: {bus} returned an empty menu");
                return Command::none();
            }
            // The item may have died while its layout was in flight: an
            // open menu for a gone item would only emit failing events.
            if !tray::items().iter().any(|i| i.bus == bus && i.path == path) {
                return Command::none();
            }
            let font = panel_font(&panel.config);
            let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
            let (sw, sh) = crate::focused_screen_size();
            // Cap the visible rows by both the private maximum and the
            // output height, so an over-long menu (a 116-row update menu)
            // scrolls instead of opening with its first rows off-screen.
            // A scrollable menu reserves one arrow band above and one
            // below; the arrow itself is drawn only in that direction.
            let row_h = menu_row_h(font_size).max(1);
            let arrow_h = menu_arrow_h(font_size);
            let screen_rows = ((sh.max(0) as u32).saturating_sub(2 * MENU_PAD) / row_h)
                .max(1) as usize;
            let scrollable = entries.len() > MENU_MAX_ROWS || entries.len() > screen_rows;
            let visible = if scrollable {
                let rows_with_arrows = ((sh.max(0) as u32)
                    .saturating_sub(2 * MENU_PAD + 2 * arrow_h)
                    / row_h)
                    .max(1) as usize;
                entries.len().min(MENU_MAX_ROWS).min(rows_with_arrows)
            } else {
                entries.len()
            };
            let (w, h) = menu_size_shown(&entries, &font, font_size, visible, scrollable);
            // The surface width is final now: overlong labels are cut to
            // their row budget (single line + ellipsis in the view).
            fit_menu_labels(&mut entries, w, &font, font_size);
            let (anchor, margin) = menu_placement(at.0, at.1, w, h, sw, sh);
            let opening = popup_anim::opening_margin(anchor, margin);
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: LayerSize::px(w, h),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: None,
                margin: Some(opening),
                keyboard_interactivity: KeyboardInteractivity::None,
                // The bar's own output: `LastOutput` landed the popup
                // on a second monitor once one joined the session.
                output_option: popup_output_option(),
                namespace: Some("piuma-tray-menu".to_string()),
                ..Default::default()
            });
            popup_anim::start(id, anchor, margin);
            panel.tray_menu = Some((
                id,
                TrayMenu {
                    bus,
                    path,
                    menu_path,
                    entries,
                    offset: 0,
                    visible,
                },
            ));
            open
        }
        Message::TrayMenuClicked(entry_id) => {
            if let Some((id, menu)) = panel.tray_menu.take() {
                panel.tray_menu_hover = None;
                dbusmenu::spawn_event(&menu.bus, &menu.menu_path, entry_id);
                popup_anim::close(panel, id, popup_anim::Ghost::Tray(menu))
            } else {
                Command::none()
            }
        }
        Message::TrayMenuScrolled(delta_y) => {
            // Wheel over the menu: whole rows, clamped to what is left.
            // The hover id belongs to the old slice, so clear it.
            if let Some((_, menu)) = panel.tray_menu.as_mut() {
                let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
                let row_h = menu_row_h(font_size).max(1) as f32;
                let rows = (-delta_y / row_h).round() as i32;
                menu.offset = scroll_rows(menu.offset, rows, menu.entries.len(), menu.visible);
                panel.tray_menu_hover = None;
            }
            Command::none()
        }
        Message::TrayMenuArrow(up) => {
            if let Some((_, menu)) = panel.tray_menu.as_mut() {
                menu.offset = scroll_rows(
                    menu.offset,
                    if up { -1 } else { 1 },
                    menu.entries.len(),
                    menu.visible,
                );
                panel.tray_menu_hover = None;
            }
            Command::none()
        }
        Message::TrayMenuEnter(entry_id) => {
            panel.tray_menu_hover = Some(entry_id);
            Command::none()
        }
        Message::TrayMenuLeave(entry_id) => {
            // Only the row still under the cursor may clear: the list
            // dispatches top-down, so entering the row above is applied
            // before the row below leaves — an unguarded clear dropped
            // the hover whenever the cursor moved upward.
            if panel.tray_menu_hover == Some(entry_id) {
                panel.tray_menu_hover = None;
            }
            Command::none()
        }
        Message::TrayMenuClosed => {
            // The surface only exists with its state (and a press
            // already bumps the generation before this can fire).
            if let Some((id, menu)) = panel.tray_menu.take() {
                panel.tray_menu_hover = None;
                popup_anim::close(panel, id, popup_anim::Ghost::Tray(menu))
            } else {
                Command::none()
            }
        }
        Message::TrayIconScrolled(bus, path, delta) => {
            if let Some((units, horizontal)) = delta {
                tray::spawn_scroll(&bus, &path, units, horizontal);
            }
            Command::none()
        }
        Message::ActionFinished(Some(text)) => osd::toast(panel, &text),
        Message::ActionFinished(None) => Command::none(),
        Message::SensorPressed(section_index, sensor_index, action) => {
            // The media sensor's click is its transport control; a
            // configured shell action has no useful meaning here.
            let is_media = panel
                .config
                .panel
                .sections
                .get(section_index)
                .and_then(|sec| sec.sensors.get(sensor_index))
                .is_some_and(|s| matches!(s.source, config::SensorSource::Media {}));
            if is_media {
                mpris::play_pause();
                return Command::none();
            }
            // The runtime-state sensors' click is their control: the
            // power profile cycles, Bluetooth toggles, the keyboard
            // layout advances. Like the media sensor, a configured shell
            // action has no useful meaning here.
            let control = panel
                .config
                .panel
                .sections
                .get(section_index)
                .and_then(|sec| sec.sensors.get(sensor_index))
                .map(|s| &s.source);
            match control {
                Some(config::SensorSource::PowerProfile {}) => {
                    std::thread::spawn(sensors::cycle_power_profile);
                    return Command::none();
                }
                Some(config::SensorSource::Bluetooth {}) => {
                    std::thread::spawn(sensors::toggle_bluetooth);
                    return Command::none();
                }
                Some(config::SensorSource::Keyboard {}) => {
                    std::thread::spawn(sensors::keyboard_next_layout);
                    return Command::none();
                }
                Some(config::SensorSource::Lock {}) => {
                    return screensaver::show(panel);
                }
                Some(config::SensorSource::Caffeine {}) => {
                    panel.caffeine = !panel.caffeine;
                    return Command::none();
                }
                Some(config::SensorSource::Power {}) => {
                    return crate::power_menu::open(panel);
                }
                Some(config::SensorSource::Theme {}) => {
                    return crate::theme_menu::open(panel);
                }
                Some(config::SensorSource::Dashboard {}) => {
                    return crate::dashboard::open(panel);
                }
                _ => {}
            }
            let action = action.filter(|a| !a.trim().is_empty());
            match action.as_deref() {
                Some(ACTION_CLOSE_PANEL) => {
                    // Panel teardown handled in main via a flag.
                    unsafe { PANEL_SHOULD_CLOSE = true; }
                }
                Some(command) => {
                    // Shell action: run off the UI thread, off the async
                    // executor (a GUI app would occupy it for minutes),
                    // and watch its first moment for a failure.
                    let label = panel
                        .config
                        .panel
                        .sections
                        .get(section_index)
                        .and_then(|sec| sec.sensors.get(sensor_index))
                        .map(|sensor| sensor.name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "Action".to_string());
                    let mut shell = std::process::Command::new("sh");
                    shell.args(["-c", command]);
                    return Command::perform(
                        smol::unblock(move || process::run_reporting(shell, label)),
                        Message::ActionFinished,
                    );
                }
                None => {
                    // No configured action: on a notifications sensor the
                    // click toggles the history list popup — the entry's
                    // spec `default` action moved to the row click in it.
                    let is_notifications = panel
                        .config
                        .panel
                        .sections
                        .get(section_index)
                        .and_then(|sec| sec.sensors.get(sensor_index))
                        .is_some_and(|s| matches!(s.source, config::SensorSource::Notifications {}));
                    if is_notifications {
                        return toggle_notif_list(panel);
                    }
                    // A layout sensor saves the current workspace's
                    // window arrangement in piuma-layouts.json on click
                    // (the bar's own bookkeeping — no shell action; the
                    // automatic name comes from the window set itself).
                    // The windows come from the last pass's snapshot.
                    let is_layout = panel
                        .config
                        .panel
                        .sections
                        .get(section_index)
                        .and_then(|sec| sec.sensors.get(sensor_index))
                        .is_some_and(|s| matches!(s.source, config::SensorSource::Layout {}));
                    if is_layout {
                        let rows = panel
                            .hypr
                            .as_ref()
                            .and_then(|h| h.clients.as_deref())
                            .unwrap_or(&[]);
                        let ws = panel
                            .hypr
                            .as_ref()
                            .and_then(|h| h.active_workspace.as_deref());
                        if let Err(e) = layouts::save(&mut panel.layout_state, rows, ws) {
                            eprintln!("[piuma] layout save failed: {e}");
                        }
                        return Command::none();
                    }
                }
            }
            Command::none()
        }
        Message::WorkspaceClicked(name) => {
            // Focus the clicked workspace in the background, like every
            // sensor action (the Lua dispatch itself lives in workspace.rs).
            workspace::focus(&name);
            Command::none()
        }
        Message::SensorScrolled(section_index, sensor_index, up) => {
            // Wheel: the notifications sensor walks its history, volume
            // sensors adjust the level; both reflect the change in place
            // (non-blocking, like the click actions above).
            if let Some(up) = up {
                let source = panel
                    .config
                    .panel
                    .sections
                    .get(section_index)
                    .and_then(|sec| sec.sensors.get(sensor_index))
                    .map(|s| &s.source);
                if matches!(source, Some(config::SensorSource::Media {})) {
                    mpris::step(up);
                    return Command::none();
                }
                if matches!(source, Some(config::SensorSource::PowerProfile {})) {
                    // Wheel cycles the profile like a click.
                    std::thread::spawn(sensors::cycle_power_profile);
                    return Command::none();
                }
                let volume_device = match source {
                    Some(config::SensorSource::PercVolume { device })
                    | Some(config::SensorSource::VolumeMeter { device }) => Some(device.clone()),
                    _ => None,
                };
                if let Some(device) = volume_device {
                    // Reflect the change on the bar right away (same math as
                    // the backend adjust); the next tick confirms it.
                    let shown = panel
                        .sections
                        .get_mut(section_index)
                        .and_then(|sec| sec.sensors.get_mut(sensor_index))
                        .and_then(|sensor_value| {
                            let next = sensors::next_volume(&sensor_value.value, up)?;
                            sensor_value.value = format!("{next:.0}%");
                            Some(sensor_value.value.clone())
                        });
                    std::thread::spawn(move || sensors::adjust_volume(&device, up));
                    // The OSD here, not from the next read: the bar already
                    // shows the new value, so the read comparison sees no
                    // change and the wheel used to miss its own popup.
                    if let Some(value) = shown {
                        let fraction = sensors::leading_number(&value)
                            .map(|v| (v / 100.0).clamp(0.0, 1.0))
                            .unwrap_or(0.0);
                        return osd::show(panel, "Volume", &value, fraction);
                    }
                }
            }
            Command::none()
        }
        Message::VolumeChanged(section_index, sensor_index, percent) => {
            let percent = percent.clamp(0.0, 100.0);
            panel.volume_drag = Some((section_index, sensor_index, percent));
            if let Some(sensor_value) = panel
                .sections
                .get_mut(section_index)
                .and_then(|sec| sec.sensors.get_mut(sensor_index))
            {
                sensor_value.value = format!("{percent:.0}%");
            }
            // Same as the wheel: the bar pre-updates its value, so the OSD
            // cannot wait for the next read to find a difference.
            osd::show(
                panel,
                "Volume",
                &format!("{percent:.0}%"),
                percent / 100.0,
            )
        }
        Message::VolumeCommitted(section_index, sensor_index) => {
            let device = panel
                .config
                .panel
                .sections
                .get(section_index)
                .and_then(|section| section.sensors.get(sensor_index))
                .and_then(|sensor| match &sensor.source {
                    config::SensorSource::VolumeMeter { device } => Some(device.clone()),
                    _ => None,
                });
            let percent = panel
                .volume_drag
                .filter(|(section, sensor, _)| {
                    *section == section_index && *sensor == sensor_index
                })
                .map(|(_, _, percent)| percent)
                .or_else(|| {
                    panel
                        .sections
                        .get(section_index)
                        .and_then(|section| section.sensors.get(sensor_index))
                        .and_then(|sensor| sensors::leading_number(&sensor.value))
                });
            panel.volume_drag = None;
            if let (Some(device), Some(percent)) = (device, percent) {
                std::thread::spawn(move || sensors::set_volume(&device, percent));
            }
            Command::none()
        }
        Message::NotifListScrolled(delta_y) => {
            // Wheel over the list: whole rows per notch (the surface
            // height is exact, so the math matches the layout). The row
            // height follows the notification style's font size.
            let font_size = popup_style(&panel.config, panel.palette.as_ref()).font_size;
            if let Some((_, state)) = panel.notif_list.as_mut() {
                let row_h = notif_list_row_h(font_size) as f32;
                let rows = (-delta_y / row_h).round() as i32;
                let len = notifications::history_rows(NOTIF_LIST_MAX).len();
                state.offset = scroll_rows(state.offset, rows, len, NOTIF_LIST_ROWS);
            }
            Command::none()
        }
        Message::NotifListArrow(up) => {
            if let Some((_, state)) = panel.notif_list.as_mut() {
                let len = notifications::history_rows(NOTIF_LIST_MAX).len();
                state.offset = scroll_rows(
                    state.offset,
                    if up { -1 } else { 1 },
                    len,
                    NOTIF_LIST_ROWS,
                );
            }
            Command::none()
        }
        Message::NotifDndToggled => {
            notifications::set_dnd(!notifications::dnd());
            // Turning it on closes the cards popup; the list stays open
            // to show the new state.
            sync_popup(panel, false).unwrap_or_else(Command::none)
        }
        Message::NotifListClear => {
            let ids = notifications::clear_history();
            for id in ids {
                notifications::spawn_closed_signal(id);
            }
            popup_notifications::refresh_notification_sensor(panel);
            let mut cmds: Vec<Command<Message>> = Vec::new();
            if let Some(cmd) = sync_popup(panel, false) {
                cmds.push(cmd);
            }
            // Nothing left to browse: the list closes with its rows.
            if let Some((id, state)) = panel.notif_list.take() {
                cmds.push(popup_anim::close(
                    panel,
                    id,
                    popup_anim::Ghost::NotifList(state),
                ));
            }
            Command::batch(cmds)
        }
        Message::NotifListClicked(id) => {
            // A row runs its notification's spec `default` action (the
            // sensor's old click) and the popup closes with it.
            if let Some((win, state)) = panel.notif_list.take() {
                notifications::invoke_id(id);
                return popup_anim::close(panel, win, popup_anim::Ghost::NotifList(state));
            }
            Command::none()
        }
        Message::TaskbarLeft(address) => {
            // Focus is the pick: it brings the window with its
            // workspace (the click's own address, focus-follows-mouse
            // irrelevant).
            windows::focus(&address);
            Command::none()
        }
        Message::TaskbarMiddle(address) => {
            windows::close_window(&address);
            Command::none()
        }
        Message::TaskbarRight(address) => {
            // Any open menu closes first (a press elsewhere = move it);
            // the new one opens under the cursor.
            let mut cmds: Vec<Command<Message>> = Vec::new();
            if let Some((id, _)) = panel.taskbar_menu.take() {
                panel.taskbar_menu_hover = None;
                cmds.push(popup_anim::close(
                    panel,
                    id,
                    popup_anim::Ghost::Taskbar,
                ));
            }
            let Some(at) = tray::cursor_output_point(panel) else {
                return if cmds.is_empty() {
                    Command::none()
                } else {
                    Command::batch(cmds)
                };
            };
            let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
            let font = panel_font(&panel.config);
            let (w, h) = taskbar_menu_size(&font, font_size);
            let (sw, sh) = crate::focused_screen_size();
            let (anchor, margin) = menu_placement(at.0, at.1, w, h, sw, sh);
            let opening = popup_anim::opening_margin(anchor, margin);
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: LayerSize::px(w, h),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: None,
                margin: Some(opening),
                keyboard_interactivity: KeyboardInteractivity::None,
                output_option: popup_output_option(),
                namespace: Some("piuma-taskbar-menu".to_string()),
                ..Default::default()
            });
            popup_anim::start(id, anchor, margin);
            let (floating, pinned) = panel
                .hypr
                .as_ref()
                .and_then(|snapshot| snapshot.clients.as_ref())
                .and_then(|rows| rows.iter().find(|row| row.address == address))
                .map(|row| (row.floating, row.pinned))
                .unwrap_or((false, false));
            panel.taskbar_menu = Some((
                id,
                TaskbarMenu {
                    address,
                    floating,
                    pinned,
                },
            ));
            cmds.push(open);
            Command::batch(cmds)
        }
        Message::TaskbarMenuWs(workspace) => {
            // The menu carries the target: move it silently (the bar's
            // workspace stays) and the menu goes with the action.
            if let Some((id, menu)) = panel.taskbar_menu.take() {
                panel.taskbar_menu_hover = None;
                windows::move_to_workspace(&menu.address, workspace);
                return popup_anim::close(panel, id, popup_anim::Ghost::Taskbar);
            }
            Command::none()
        }
        Message::TaskbarMenuAction(action) => {
            // Same contract as the workspace rows: the menu carries the
            // target and goes with the action.
            if let Some((id, menu)) = panel.taskbar_menu.take() {
                panel.taskbar_menu_hover = None;
                window_menu::run(action, &menu.address, menu.floating, menu.pinned);
                return popup_anim::close(panel, id, popup_anim::Ghost::Taskbar);
            }
            Command::none()
        }
        Message::WindowMenuSync(target) => window_menu::apply(panel, target),
        Message::WindowMenuAction(action) => {
            // A fully faded menu is invisible: its surface stays mounted
            // for the fade-in, the buttons must not act from under the
            // pointer until it is visible again.
            if panel.window_menu.as_ref().is_some_and(|menu| menu.alpha() <= 0.01) {
                return Command::none();
            }
            if let Some(target) = panel.window_menu.as_ref().and_then(|menu| menu.target.as_ref()) {
                window_menu::run(action, &target.address, target.floating, target.pinned);
            }
            Command::none()
        }
        Message::WindowMenuHover(button) => {
            if let Some(menu) = panel.window_menu.as_mut() {
                menu.hover = button;
            }
            Command::none()
        }
        Message::WindowMenuLeft(button) => {
            // Stale-exit guard, same rule as the taskbar rows.
            if let Some(menu) = panel.window_menu.as_mut() {
                if menu.hover == Some(button) {
                    menu.hover = None;
                }
            }
            Command::none()
        }
        Message::WindowMenuFrame => window_menu::frame(panel),
        Message::PopupFrame => popup_anim::frame(),
        Message::PopupClosed(id) => {
            panel.closing.retain(|(window, _)| *window != id);
            iced::window::close(id)
        }
        Message::WindowMenuIdle(idled) => window_menu::idle(panel, idled),
        Message::TaskbarMenuHover(row) => {
            panel.taskbar_menu_hover = row;
            Command::none()
        }
        Message::TaskbarMenuLeft(workspace) => {
            // An exit that names another row arrived after the new
            // row's enter (tree order): only the row still under the
            // cursor may clear the hover.
            if panel.taskbar_menu_hover == Some(workspace) {
                panel.taskbar_menu_hover = None;
            }
            Command::none()
        }
        Message::TaskbarIconEnter { address, title } => {
            panel.taskbar_hover = Some(address.clone());
            tooltip_open(panel, address, title)
        }
        Message::TaskbarIconLeave { address } => {
            // Same stale-exit rule as `TaskbarMenuLeft`: only the icon
            // still under the cursor keeps (or clears) the hover.
            if panel.taskbar_hover.as_deref() == Some(address.as_str()) {
                panel.taskbar_hover = None;
            }
            tooltip_close(panel, address)
        }
        Message::TooltipShow { id, text } => tooltip_open(panel, id, text),
        Message::TooltipHide { id } => tooltip_close(panel, id),
        Message::TaskbarMenuClosed => {
            if let Some((id, _)) = panel.taskbar_menu.take() {
                panel.taskbar_menu_hover = None;
                return popup_anim::close(panel, id, popup_anim::Ghost::Taskbar);
            }
            Command::none()
        }
        Message::SensorHovered(section_index, sensor_index, hovered) => {
            // Any new enter/leave supersedes a hover still waiting for
            // its sensor box.
            panel.pending_hover = None;
            if hovered {
                // Only armed: the hover (style and popups) activates
                // when the dwell confirms it, so a pointer sweeping
                // past a sensor triggers nothing.
                let token = panel.hover_gate.enter(section_index, sensor_index);
                Command::perform(
                    async move { tray::delay(SENSOR_HOVER_DELAY).await },
                    move |_| Message::SensorHoverConfirmed(token.0, token.1, token.2),
                )
            } else {
                // Leaving is immediate: it cancels a pending enter and
                // closes an active hover alike.
                panel.hover_gate.leave(section_index, sensor_index);
                if panel.hovered == Some((section_index, sensor_index)) {
                    panel.hovered = None;
                }
                hover_sync(panel, section_index, sensor_index, false)
            }
        }
        Message::SensorHoverConfirmed(section_index, sensor_index, seq) => {
            let Some(armed) = panel.hover_gate.confirm(section_index, sensor_index, seq) else {
                return Command::none(); // superseded: the pointer moved on
            };
            // A pill that grew or shifted under a still pointer fires an
            // enter too (iced compares bounds as well as the cursor): no
            // motion next to the arm means nobody hovered it.
            if !hover_is_fresh(armed, panel.last_motion) {
                return Command::none();
            }
            // The pointer must really be on that sensor: iced also fires
            // an enter when a widget's bounds change under a still cursor,
            // and a shift can hand the enter to a sensor far from it (seen
            // live: the hovered list sensor handed it to `Layout`). The
            // box comes from the widget tree, so the check rides a query
            // (`SensorRectFound` sets the hover).
            panel.pending_hover = Some((section_index, sensor_index));
            iced_runtime::task::widget(FindSensorRect::new(section_index, sensor_index))
        }
        Message::SensorRectFound(section_index, sensor_index, rect) => {
            // A newer enter/leave already replaced this query's hover.
            if panel.pending_hover != Some((section_index, sensor_index)) {
                return Command::none();
            }
            panel.pending_hover = None;
            // The box is the real one from the widget tree: a cursor
            // outside it means iced handed the enter to the wrong sensor.
            if !cursor_in_rect(panel.last_cursor, rect) {
                return Command::none();
            }
            panel.hovered = Some((section_index, sensor_index));
            hover_sync(panel, section_index, sensor_index, true)
        }
        Message::Idle(idle) => {
            if !idle {
                // Input while locked does not unlock: the password does.
                if panel.screensaver_on {
                    return Command::none();
                }
                panel.screensaver_wait = false;
                panel.screensaver_retry = None;
                return Command::none();
            }
            if panel.config.screensaver().is_none() || panel.screensaver_on || panel.caffeine {
                return Command::none();
            }
            // The overlay must not cover a fullscreen app or a playing
            // video (even windowed): ask the compositor and MPRIS before
            // opening anything.
            panel.screensaver_wait = true;
            panel.screensaver_retry = None;
            Command::perform(
                smol::unblock(|| {
                    hypr::active_is_fullscreen() == Some(true) || playback::blocks_screensaver()
                }),
                Message::ScreensaverBlocked,
            )
        }
        Message::ScreensaverBlocked(blocked) => {
            if !panel.screensaver_wait || panel.screensaver_on {
                return Command::none(); // superseded by a resume or a manual start
            }
            if blocked || panel.caffeine {
                // A fullscreen app or a playing video: no overlay;
                // `sync` checks again in a minute.
                return Command::none();
            }
            screensaver::show(panel)
        }
        Message::ScreensaverManual => screensaver::show(panel),
        Message::Shell(event) => screensaver::on_shell(panel, event),
        Message::SectionToggled(index) => {
            if panel.collapsed_sections.remove(&index) {
                return Command::none();
            }
            panel.collapsed_sections.insert(index);
            // A collapsed section has no sensors left on screen: its
            // popups go with it, and a stale hover is cleared.
            let mut cmds = Vec::new();
            if let Some(gp) = panel.graph_popup.take_if(|gp| gp.section == index) {
                cmds.push(popup_anim::close(
                    panel,
                    gp.id,
                    popup_anim::Ghost::Graph(gp),
                ));
            }
            if let Some(cp) = panel.calendar.take_if(|cp| cp.section == index) {
                cmds.push(popup_anim::close(
                    panel,
                    cp.id,
                    popup_anim::Ghost::Calendar(cp),
                ));
            }
            if panel.hovered.is_some_and(|(section, _)| section == index) {
                panel.hovered = None;
            }
            Command::batch(cmds)
        }
        Message::DashboardClose => dashboard::close(panel),
        Message::DashboardCover(url, handle) => {
            // A decode that raced another art change is stale.
            if panel.cover_url.as_deref() == Some(url.as_str()) {
                panel.cover = handle;
            }
            Command::none()
        }
        Message::DashboardAction(action) => match action {
            dashboard::Action::PowerProfile => {
                // Show the next label at once; the probe confirms it.
                let current = panel
                    .dash_probe
                    .as_ref()
                    .map(|p| p.power_profile.as_str())
                    .unwrap_or("");
                panel.dash_profile = Some(dashboard::next_profile(current).to_string());
                std::thread::spawn(sensors::cycle_power_profile);
                Command::none()
            }
            dashboard::Action::Caffeine => {
                panel.caffeine = !panel.caffeine;
                Command::none()
            }
            dashboard::Action::Bluetooth => {
                std::thread::spawn(sensors::toggle_bluetooth);
                Command::none()
            }
            dashboard::Action::Lock => {
                let closed = dashboard::close(panel);
                Command::batch([closed, screensaver::show(panel)])
            }
            dashboard::Action::PlayPause => {
                mpris::play_pause();
                Command::none()
            }
            dashboard::Action::Previous => {
                mpris::step(true);
                Command::none()
            }
            dashboard::Action::Next => {
                mpris::step(false);
                Command::none()
            }
        },
        Message::DashboardVolume(value) => {
            panel.dash_volume = Some(value);
            panel.dash_volume_drag = true;
            Command::none()
        }
        Message::DashboardVolumeRelease => {
            panel.dash_volume_drag = false;
            if let Some(value) = panel.dash_volume {
                std::thread::spawn(move || sensors::set_volume("", value));
            }
            Command::none()
        }
        Message::DashboardBrightness(value) => {
            panel.dash_brightness = Some(value);
            panel.dash_brightness_drag = true;
            Command::none()
        }
        Message::DashboardBrightnessRelease => {
            panel.dash_brightness_drag = false;
            if let Some(value) = panel.dash_brightness {
                std::thread::spawn(move || sensors::set_backlight(value as f64));
            }
            Command::none()
        },
        Message::ThemeSelected(slug) => {
            let close = theme_menu::close(panel);
            // An unknown slug is custom, exactly like a hand-edited
            // `theme.json`: never apply it as a broken theme.
            let chosen = slug.as_deref().and_then(|s| themes::find(s)).map(|t| t.slug);
            if let Err(err) = themes::write_active(chosen) {
                eprintln!("[piuma] theme: cannot save the active theme: {err}");
                return close;
            }
            panel.theme_state = themes::state();
            panel.active_theme = chosen;
            apply_active_theme(panel);
            if let Some(path) = CONFIG_PATH.get() {
                app_theme::sync(
                    std::path::Path::new(path),
                    &panel.config.panel,
                    panel.palette.as_ref(),
                );
            }
            close
        }
        Message::PowerAction(action) => {
            let close = power_menu::close(panel);
            let command = match action {
                power_menu::Action::Lock => screensaver::show(panel),
                power_menu::Action::Logout => spawn("hyprctl", &["dispatch", "exit"]),
                power_menu::Action::Suspend => spawn("systemctl", &["suspend"]),
                power_menu::Action::Reboot => spawn("systemctl", &["reboot"]),
                power_menu::Action::Shutdown => spawn("systemctl", &["poweroff"]),
            };
            Command::batch([close, command])
        }
        Message::ScreensaverSubmit => {
            if !panel.screensaver_on || panel.screensaver_pending {
                return Command::none();
            }
            let password = std::mem::take(&mut panel.screensaver_password);
            if password.is_empty() {
                return Command::none();
            }
            panel.screensaver_pending = true;
            panel.screensaver_error = None;
            Command::perform(
                smol::unblock(move || auth::verify(&password)),
                Message::ScreensaverAuthResult,
            )
        }
        Message::ScreensaverAuthResult(result) => {
            panel.screensaver_pending = false;
            if !panel.screensaver_on {
                return Command::none(); // unlocked by a previous attempt
            }
            match result {
                Ok(()) => {
                    panel.screensaver_password.clear();
                    screensaver::close(panel)
                }
                Err(_) => {
                    panel.screensaver_password.clear();
                    panel.screensaver_error = Some("Wrong password".to_string());
                    Command::none()
                }
            }
        }
        Message::ScreensaverFrame => {
            screensaver::advance(panel);
            Command::none()
        }
        Message::GraphPopupExited => {
            // The pointer left the surface: this exit is the fast
            // close, while the flag reset leaves the tick's reap ready
            // to backstop the times it never fires.
            if let Some(gp) = panel.graph_popup.as_mut() {
                gp.pointer_on_popup = false;
            }
            // The cursor is back on the popup's sensor: it stays (the
            // popup re-anchors when the sensor is entered again).
            let on_sensor = panel
                .graph_popup
                .as_ref()
                .is_some_and(|gp| panel.hovered == Some((gp.section, gp.sensor)));
            if on_sensor {
                eprintln!("[piuma] graph: keep (pointer left, cursor on the sensor)");
                return Command::none();
            }
            if let Some(gp) = panel.graph_popup.take() {
                eprintln!("[piuma] graph: close (pointer left the popup)");
                popup_anim::close(panel, gp.id, popup_anim::Ghost::Graph(gp))
            } else {
                Command::none()
            }
        }
        Message::CalendarPrev => calendar_page(panel, -1),
        Message::CalendarNext => calendar_page(panel, 1),
        Message::CalendarExited => {
            // No close here: the popup survives the three grace seconds
            // in case the cursor comes back to it or to its sensor (the
            // tick's reap decides, see `reap_abandoned_calendar`).
            if let Some(cp) = panel.calendar.as_mut() {
                cp.pointer_on_popup = false;
            }
            Command::none()
        }
        _ => Command::none(),
    }
}

/// The binary entry point: the daemon lives in [`app::run`].
fn main() -> Result<(), iced_exwlshell::Error> {
    app::run()
}

#[cfg(test)]
mod tests_main;
