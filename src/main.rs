//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.
//!
//! Configuration is loaded from a JSON file (CLI arg or default path).
//! See `src/config.rs` for the config schema.

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use iced::widget::{container, text, Column, Row, MouseArea, Space, Stack};
use iced::{
    Alignment, Background, Color, Element, Event, Length, Subscription, Task as Command,
};
use iced::event;
use iced::time;
use iced::widget::container::Style as ContainerStyle;
use iced::advanced::widget::Operation;

use iced_exwlshell::ExWlSettings;
use iced_exwlshell::daemon;
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::{LayerShellSettings, LayerSize, StartMode};
use iced_exwlshell::to_layer_message;
use iced_runtime::core::widget::operation::Outcome;

mod app_theme;
mod autostart;
mod calendar;
mod config;
mod dbusmenu;
mod fill_sections;
mod geometry;
mod graphs;
mod hypr;
mod icons;
mod instance_lock;
mod menus;
mod notifications;
mod palette;
mod popup_calendar;
mod popup_graph;
mod popup_notifications;
mod sensor_runner;
mod sensors;
mod styles;
mod text_util;
mod text_width;
mod tooltip;
mod tray;
mod wallpaper_ui;
mod windows;
mod workspace;

use fill_sections::FillSections;
use geometry::{corner_radius, panel_anchor, panel_size, panel_thickness, placement_rect};
use menus::{
    fit_menu_labels, menu_placement, menu_size, taskbar_menu_size, taskbar_menu_view,
    tray_menu_view,
};
use popup_calendar::{
    calendar_hover_sync, calendar_page, calendar_popup_view, reap_abandoned_calendar,
};
use popup_graph::{
    graph_hover_sync, graph_link_view, graph_popup_view, reap_abandoned_popup, sync_graph_link,
};
use popup_notifications::{
    notif_list_row_h, notif_list_view, popup_dismiss, popup_view, scroll_rows, sync_popup,
    toggle_notif_list, NOTIF_LIST_MAX, NOTIF_LIST_ROWS,
};
use styles::{panel_background, resolve_styles, restyle};
use text_util::{panel_font, value_font};
use tooltip::{tooltip_close, tooltip_open, tooltip_window_view};
use wallpaper_ui::{sync_palette, sync_wallpaper, wallpaper_view};
// Layout padding lives with the icon sizing helpers (icons.rs): the view
// and the row-height size formula must read the same number.
use icons::SECTION_CONTENT_PAD;

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

pub(crate) const DEFAULT_FONT: &str = "Noto Sans";
pub(crate) const DEFAULT_FONT_SIZE: u32 = 14;
/// Gap between two sensor boxes inside a section: the same rhythm on
/// both axes, whether the sensors sit in a row or a stacked column.
const SENSOR_SPACING: f32 = 8.0;
pub(crate) const DEFAULT_BG_COLOR: Color = Color::from_rgba(0.06, 0.06, 0.08, 0.85);
pub(crate) const DEFAULT_TEXT_COLOR: Color = Color::from_rgb(0.88, 0.88, 0.94);

/// Internal action identifiers.
pub(crate) const ACTION_CLOSE_PANEL: &str = "close_panel";

// ── popup card geometry (F3) ────────────────────────────────────────
// The popup surface is sized to the stack exactly (cards × height +
// gaps): the region below the stack simply is not surface, so clicks
// there reach the window underneath with no input-region handling.
// The card height is style-derived (`popup_card_h`); with the default
// style (12px font, 12px padding) it is exactly 84px.

// ── sensor graph popup geometry (roadmap2 #11) ──────────────────────
// One popup at a time, on the sensor under the cursor: fixed size, the
// surface ends where the box ends (no input-region games — the layout
// below knows nothing of it, the popup is its own window).

// ── calendar popup geometry (the `calendar` sensor) ────────────────

// ── tray menu geometry ──────────────────────────────────────────────
// One row per entry, sized exactly (`menu_row_h` × count + 2× pad, see
// `menu_size`): like the popup, the surface ends where the list ends,
// so a stray click below it reaches the window underneath.


// ── model ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct SensorValue {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) value: String,
    pub(crate) text_color: Option<iced::Color>,
    /// Threshold colour rules (config `color_rules`) parsed to colours
    /// and sorted ascending by `min`: the view tints the value with
    /// the highest rule the displayed reading reaches.
    pub(crate) color_rules: Vec<(f32, Color)>,
    pub(crate) bg_color: Option<Color>,
    pub(crate) text_color_hover: Option<iced::Color>,
    pub(crate) bg_color_hover: Option<Color>,
    pub(crate) border_color: Option<Color>,
    pub(crate) border_width: u16,
    pub(crate) border_radius: u16,
    /// Border drawing style: `full`, `corners`, `underline`, `upperline`,
    /// `left` or `right`.
    pub(crate) border_type: String,
    pub(crate) padding: u16,
    pub(crate) action: Option<String>,
    /// Tray icons when the source is a `tray` sensor (`None` otherwise);
    /// the icons themselves live in `tray.rs`.
    pub(crate) tray: Option<Vec<tray::TrayIcon>>,
    /// Workspace buttons when the source is a `workspace` sensor (`None`
    /// otherwise), built by `workspace::buttons()` (roadmap2 #4).
    pub(crate) workspaces: Option<Vec<workspace::WorkspaceButton>>,
    /// Taskbar slots when the source is a `taskbar` sensor (`None`
    /// otherwise), one per open window, built per tick by
    /// `windows::task_icons()`.
    pub(crate) taskbar: Option<Vec<windows::TaskIcon>>,
    /// Whether this sensor shows the notification feed: its text
    /// refreshes from the shared state alone when a notification
    /// changes (`refresh_tray_and_notif`), without re-reading every
    /// other sensor.
    pub(crate) is_notifications: bool,
    /// Pinned pixel width of the value text (roadmap M5): net rate sensors
    /// keep a stable box while the value moves inside its unit, and the pin
    /// follows the value across ×1024 unit crossings (same tick), so it is
    /// never narrower than the content.
    pub(crate) fixed_width: Option<u16>,
    /// Resolved theme file for the sensor's `icon` name (roadmap N1);
    /// `None` = no icon configured or the theme has no such icon.
    pub(crate) icon: Option<PathBuf>,
    /// Edge (px) of `icon`: the sensor row's real height
    /// (`icons::row_icon_target`), so the icon fills its box like the
    /// tray's do. The resolve caches by this size and the widget draws
    /// at it — both must stay the same number.
    pub(crate) icon_size: u16,
}

/// Rendered section data.
pub(crate) struct SectionData {
    pub(crate) name: String,
    pub(crate) layout: String,       // "horizontal" or "vertical"
    pub(crate) bg_color: Color,
    pub(crate) text_color: Color,
    pub(crate) title_color: Color,
    /// Resolved border (section > section_defaults): drawn by `styled_box`
    /// with the same rule as the sensors (color + positive width).
    pub(crate) border_color: Option<Color>,
    pub(crate) border_width: u16,
    pub(crate) border_radius: u16,
    pub(crate) border_type: String,
    /// Which sides `[top, right, bottom, left]` sit flush on a screen edge
    /// with `section_defaults.respect_screen_edges` active (roadmap M3):
    /// those sides render square instead of rounded.
    pub(crate) touch: [bool; 4],
    pub(crate) sensors: Vec<SensorValue>,
}

/// A sensor's resolved style: every colour and shape field the view
/// draws plus its labels — the whole precedence chain (sensor >
/// wallpaper palette > defaults) resolved once per config load or
/// palette change instead of once per sensor per second.
#[derive(Clone)]
pub(crate) struct SensorStyle {
    /// Display name — empty for the sensors that never show it
    /// (`static_text`, tray, workspace buttons).
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) action: Option<String>,
    /// `true` for the notification feed sensor: its text refreshes
    /// from the shared state alone (`refresh_tray_and_notif`).
    pub(crate) is_notifications: bool,
    pub(crate) padding: u16,
    pub(crate) text_color: Color,
    pub(crate) bg_color: Option<Color>,
    pub(crate) text_color_hover: Option<Color>,
    pub(crate) bg_color_hover: Option<Color>,
    pub(crate) border_color: Option<Color>,
    pub(crate) border_width: u16,
    pub(crate) border_radius: u16,
    /// `full`, `corners`, `underline`, `upperline`, `left` or `right`.
    pub(crate) border_type: String,
    /// Threshold colours sorted ascending: the view picks the highest
    /// rule the reading reaches.
    pub(crate) color_rules: Vec<(f32, Color)>,
    /// Theme file for the sensor's icon (`None` = none configured).
    pub(crate) icon: Option<PathBuf>,
    pub(crate) icon_size: u16,
}

/// A section's resolved style (same chain as [`SensorStyle`]): the
/// box fields of `SectionData` plus the per-sensor styles — all of it
/// config-driven, so the tick only pairs readings with it.
#[derive(Clone)]
pub(crate) struct SectionStyle {
    pub(crate) name: String,
    pub(crate) layout: String,
    pub(crate) bg_color: Color,
    pub(crate) text_color: Color,
    pub(crate) title_color: Color,
    pub(crate) border_color: Option<Color>,
    pub(crate) border_width: u16,
    pub(crate) border_radius: u16,
    pub(crate) border_type: String,
    pub(crate) touch: [bool; 4],
    pub(crate) sensors: Vec<SensorStyle>,
}

pub(crate) struct Panel {
    pub(crate) config: config::Config,
    pub(crate) sections: Vec<SectionData>,
    /// Every section's and sensor's resolved style (`resolve_styles`),
    /// rebuilt on config reload and on wallpaper-palette change — the
    /// tick pairs readings with it instead of re-resolving colours.
    pub(crate) styles: Vec<SectionStyle>,
    /// True while the tick's data pass runs on a worker thread: one
    /// read at a time, so replies can never arrive out of order.
    pub(crate) read_pending: bool,
    /// Bumped on every config reload: a reply built against an older
    /// config no longer lines up with the sensor list and is dropped.
    pub(crate) cfg_gen: u64,
    /// Set when geometry or config moved in the tick that spawned the
    /// current read; consumed with the fresh sections, so the popup
    /// re-anchors on them (`apply_read` → `sync_popup`).
    pub(crate) popup_resync: bool,
    /// Index into `sections[s]` and `sections[s].sensors[sensor]` under the cursor.
    pub(crate) hovered: Option<(usize, usize)>,
    /// State (mtime, len) of the last config read attempt: unchanged files
    /// are skipped on the next tick instead of being re-read and parsed.
    pub(crate) last_reload: Option<(SystemTime, u64)>,
    /// Geometry currently applied to the layer surface; compared after a
    /// reload to decide whether a runtime `LayoutChange` is needed.
    pub(crate) last_size: LayerSize,
    pub(crate) last_anchor: Anchor,
    /// Exclusive zone currently applied (equals the panel thickness, the
    /// space hyprland subtracts for windows); re-sent when it changes.
    pub(crate) last_exclusive_zone: i32,
    /// Last known cursor position, window-relative: feeds the screen-coordinate
    /// hints of tray activation (see `tray::hint_point`).
    pub(crate) last_cursor: Option<(f32, f32)>,
    /// Popup stack window (F3): its id while open, `None` while hidden.
    pub(crate) popup: Option<iced::window::Id>,
    /// Card count the popup surface is currently sized for.
    pub(crate) popup_cards: usize,
    /// Card under the cursor (`hover_*` style fields), `None` outside.
    pub(crate) popup_hover: Option<u32>,
    /// Open tray DBusMenu window: id + the menu it renders, `None`
    /// while hidden (see `dbusmenu`).
    pub(crate) tray_menu: Option<(iced::window::Id, TrayMenu)>,
    /// Menu row under the cursor (`TrayMenuEnter`/`TrayMenuLeave`),
    /// `None` outside every row.
    pub(crate) tray_menu_hover: Option<i32>,
    /// Bumped by every tray press: a `TrayMenuLoaded` carrying an older
    /// generation belongs to a superseded press and must not open.
    pub(crate) tray_menu_gen: u64,
    /// The notification history list popup (the notifications sensor's
    /// click): window id + the wheel scroll state.
    pub(crate) notif_list: Option<(iced::window::Id, NotifList)>,
    /// The taskbar's right-click menu: window id + the window it acts
    /// on.
    pub(crate) taskbar_menu: Option<(iced::window::Id, TaskbarMenu)>,
    /// Workspace row under the cursor in the taskbar menu
    /// (`TaskbarMenuHover`), `None` outside every row.
    pub(crate) taskbar_menu_hover: Option<u32>,
    /// Address of the taskbar icon under the cursor (`None` = none):
    /// the row's hover fill.
    pub(crate) taskbar_hover: Option<String>,
    /// The tooltip window (taskbar or tray hover): window id + source
    /// and fitted text.
    pub(crate) bar_tooltip: Option<(iced::window::Id, BarTooltip)>,
    /// Trend history for the `draw_graph` sensors' hover popup: one
    /// dense ring of samples per config, rebuilt on reload (see
    /// `graphs`).
    pub(crate) graphs: graphs::History,
    /// Open graph popup: its surface id, the sensor it draws and the
    /// output-space rect the hover guards compare the cursor with,
    /// `None` while hidden.
    pub(crate) graph_popup: Option<GraphPopup>,
    /// The connector surface joining the graph popup to its sensor (see
    /// `sync_graph_link`): its id and the id of the popup it draws —
    /// `None` while hidden.
    pub(crate) graph_link: Option<(iced::window::Id, iced::window::Id)>,
    /// Open calendar popup of a hovered `calendar` sensor: surface id,
    /// the sensor it belongs to, the shown month and its pointer/grace
    /// state, `None` while hidden.
    pub(crate) calendar: Option<CalendarPopup>,
    /// Wallpaper windows (roadmap2 #1): one background surface per
    /// active output, keyed by the compositor's output name; empty
    /// while the wallpaper is off.
    pub(crate) wallpaper: Vec<(String, iced::window::Id)>,
    /// Colours derived from the wallpaper (`panel.wallpaper_palette`):
    /// applied over the configured defaults while present, dropped when
    /// the option or the wallpaper turns off. Runtime-only — never
    /// written back to the config file.
    pub(crate) palette: Option<palette::Palette>,
    /// The wallpaper file (path + mtime) the current palette was
    /// extracted from: the extraction re-runs only when this changes.
    pub(crate) palette_key: Option<(String, SystemTime)>,
}

/// One open tray menu: which item it belongs to (the item's bus + path
/// tell us when it dies), the menu object path events go to, and the
/// flattened rows the view renders.
#[derive(Clone)]
pub(crate) struct TrayMenu {
    pub(crate) bus: String,
    pub(crate) path: String,
    pub(crate) menu_path: String,
    pub(crate) entries: Vec<dbusmenu::MenuEntry>,
}

/// The open notification list popup (the notifications sensor's click):
/// its window and the wheel scroll offset (whole rows from the newest).
#[derive(Clone, Debug)]
pub(crate) struct NotifList {
    pub(crate) offset: u32,
}

/// The taskbar's right-click menu: which window the workspace rows
/// move (the address, never an index — a tick may rebuild the icons
/// while the menu is open).
#[derive(Clone, Debug)]
pub(crate) struct TaskbarMenu {
    pub(crate) address: String,
}

/// The open bar tooltip window: its source (so a stale leave knows
/// whether it still owns the surface) and the fitted text shown.
#[derive(Clone, Debug)]
pub(crate) struct BarTooltip {
    pub(crate) id: String,
    pub(crate) text: String,
}

/// The open sensor-graph popup: which sensor's trend it draws and the
/// output-space rect `(x0, y0, x1, y1)` it occupies (placement of the
/// moment, for the hover guards). `pointer_on_popup`/`missed` feed the
/// tick's abandonment reap (see `popup_missed_tick`).
#[derive(Clone)]
pub(crate) struct GraphPopup {
    pub(crate) id: iced::window::Id,
    pub(crate) section: usize,
    pub(crate) sensor: usize,
    pub(crate) rect: (i32, i32, i32, i32),
    /// Which output edge the popup anchors to: an anchored surface sits
    /// past the bar's reserved strip, so the real surface rect derives
    /// from `rect` + the anchor (see `popup_surface_rect`).
    pub(crate) anchor: Anchor,
    /// The sensor box's on-screen rect in output space, as answered by
    /// `FindSensorRect`: the connector window is pinned to it.
    pub(crate) sensor_rect: Option<(i32, i32, i32, i32)>,
    /// The cursor is on the popup's surface — set and cleared by the
    /// surface's own cursor events (position-checked both ways, plus
    /// the window's `CursorLeft`) and by the widget's exit. Never
    /// one-way: a single "seen" setter with only the widget's exit as
    /// a reset pins the popup open forever when that exit never fires
    /// (the very bug `31771e1` fixed on the calendar popup).
    pub(crate) pointer_on_popup: bool,
    /// Grace ticks spent without sensor and without pointer.
    pub(crate) missed: u8,
}

/// The calendar popup: which sensor's hover opened it, the month
/// paging offset and the localized data fetched at open — plus the
/// pointer/grace bookkeeping that decides when it disappears.
#[derive(Clone)]
pub(crate) struct CalendarPopup {
    pub(crate) id: iced::window::Id,
    pub(crate) section: usize,
    pub(crate) sensor: usize,
    /// Whole months between today's month and the shown one: the
    /// arrows page it, a close drops it (a reopen starts at today).
    pub(crate) offset: i32,
    /// Today (local): the highlight and the month the offset starts at.
    pub(crate) today: (u16, u8, u8),
    /// Monday-first weekday labels, fetched once at open.
    pub(crate) weekdays: [String; 7],
    /// The header's "month year", re-fetched on every page change.
    pub(crate) month_label: String,
    /// The cursor is on the popup's surface — set and cleared by the
    /// surface's own cursor events (position-checked both ways, plus
    /// the window's `CursorLeft`) and by the open-time test against
    /// `rect`. Never one-way: a single "seen" setter with only the
    /// widget's exit as a reset pins the popup open forever when that
    /// exit never fires (its enter can be missed: the surface maps
    /// under an already-moving cursor before the widget state exists).
    pub(crate) pointer_on_popup: bool,
    /// The surface's real output-space rect (the placement shifted by
    /// the anchors, `shifted_surface_rect`): the pointer checks compare
    /// against where the surface really is — a top bar pushes it down
    /// by the exclusive zone, so the intended rect contains the cursor
    /// while the surface does not.
    pub(crate) rect: (i32, i32, i32, i32),
    /// Grace ticks spent without sensor and without pointer.
    pub(crate) missed: u8,
}

// ── messages ────────────────────────────────────────────────────────

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub(crate) enum Message {
    /// Marks a second edge: the tick handler reloads, polls and advances.
    Tick,
    /// The tick's data pass finished on a worker thread: the config
    /// generation it read against (a reply from an older config is
    /// dropped, see `spawn_read`) and every sensor's reading.
    SensorsReady(u64, sensor_runner::Reads),
    /// The wallpaper palette finished extracting: the tones (or `None`
    /// on a failed decode) and the file key they came from.
    PaletteReady(Option<palette::Palette>, (String, SystemTime)),
    /// A cursor/input event with the id of the window it happened on:
    /// coordinates are per-window and only the bar's own belong into
    /// `last_cursor`.
    IcedEvent(iced::window::Id, Event),
    /// A sensor entered/exited the hover state: (section_index, sensor_index, hovered).
    SensorHovered(usize, usize, bool),
    /// A sensor was clicked: (section_index, sensor_index, action).
    SensorPressed(usize, usize, Option<String>),
    /// A workspace button was clicked: focus that workspace (roadmap2 #4).
    WorkspaceClicked(String),
    /// Wheel scrolled over a sensor: (section_index, sensor_index, up);
    /// `None` for scroll events that carry no usable delta.
    SensorScrolled(usize, usize, Option<bool>),
    /// Wheel over the notification list popup: signed pixel delta-y
    /// (positive = towards the newest rows).
    NotifListScrolled(f32),
    /// Row clicked in the notification list popup: run that entry's
    /// spec `default` action and close the popup.
    NotifListClicked(u32),
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
    /// Workspace row under the cursor in the taskbar menu (`None`
    /// outside every row).
    TaskbarMenuHover(Option<u32>),
    /// Menu row the cursor left: clears only if it is still the hovered
    /// one — widget order can deliver this exit after the next row's
    /// enter (iced visits the tree in order), and a stale exit must not
    /// drop the new hover.
    TaskbarMenuLeft(u32),
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
    /// Answer of the sensor-box rect query `FindSensorRect`:
    /// (section, sensor, window-relative rect). The connector window
    /// gets pinned to it (`sync_graph_link`).
    SensorRectFound(usize, usize, (i32, i32, i32, i32)),
    /// Click on a popup card: run its spec `default` action, dismiss it.
    PopupCardClicked(u32),
    /// Click on a popup card's ✕: dismiss it only (spec reason 2).
    PopupCardDismissed(u32),
    /// Cursor entered a popup card (hover styling, both card targets).
    PopupCardEnter(u32),
    /// Cursor left the popup card under it.
    PopupCardLeave,
}

// ── namespace ───────────────────────────────────────────────────────

fn namespace() -> String {
    String::from("piuma")
}

// ── sensor rect query ──────────────────────────────────────────────

/// Widget id of a sensor box in the bar view: the graph popup queries
/// the sensor's on-screen rect through it (see `FindSensorRect`).
fn sensor_box_id(section: usize, sensor: usize) -> iced::widget::Id {
    format!("piuma-sensor-{section}-{sensor}").into()
}

/// Runtime query of a sensor box's rect: the operation walks the
/// widget trees (on every window — one answer is enough, only the bar
/// carries the tagged container) and reports the bounds of the
/// container tagged with the sensor's widget id.
pub(crate) struct FindSensorRect {
    pub(crate) section: usize,
    pub(crate) sensor: usize,
    pub(crate) target: iced::widget::Id,
    pub(crate) found: Option<(i32, i32, i32, i32)>,
}

impl FindSensorRect {
    fn new(section: usize, sensor: usize) -> Self {
        Self {
            section,
            sensor,
            target: sensor_box_id(section, sensor),
            found: None,
        }
    }
}

impl Operation<Message> for FindSensorRect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&iced::widget::Id>, bounds: iced::Rectangle) {
        if self.found.is_none() && id == Some(&self.target) {
            self.found = Some((
                bounds.x as i32,
                bounds.y as i32,
                (bounds.x + bounds.width) as i32,
                (bounds.y + bounds.height) as i32,
            ));
        }
    }

    fn finish(&self) -> Outcome<Message> {
        match self.found {
            Some(rect) => Outcome::Some(Message::SensorRectFound(self.section, self.sensor, rect)),
            None => Outcome::None,
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────





/// A solid bar widget used to compose partial sensor borders.
pub(crate) fn border_bar(color: Color, width: Length, height: Length) -> Element<'static, Message> {
    container(Space::new().width(width).height(height))
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(color)),
            ..ContainerStyle::default()
        })
        .into()
}

/// Build a styled box: background plus its border, drawn according to
/// `border_type`. Used for the sensor boxes, the section containers and the
/// tray sensor's tooltip (which must look like the sensor it belongs to).
///
/// * `full` (or any unknown value) -> native container border, which also
///   honors `border_radius`;
/// * `underline` / `upperline` / `left` / `right` -> a single colored bar on
///   that side, overlaid through a `Stack` so the box keeps exactly the
///   size it would have without a border;
/// * `corners` -> four L-shaped brackets (each arm covers one quarter of
///   its side, so every side keeps a visible middle gap).
///
/// The border is drawn only when both a color and a positive width are set,
/// which is the rule the native border always applied.
pub(crate) fn styled_box<'a>(
    content: Element<'a, Message>,
    bg_color: Option<Color>,
    border_color: Option<Color>,
    border_width: u16,
    border_radius: u16,
    border_type: &str,
    // Sides `[top, right, bottom, left]` flush with a screen edge: they
    // render square instead of rounded (roadmap M3; `[false; 4]` elsewhere).
    touch: [bool; 4],
    padding: u16,
    width: Length,
    height: Length,
) -> Element<'a, Message> {
    // Only these types bypass the native uniform border.
    let partial = matches!(
        border_type,
        "underline" | "upperline" | "left" | "right" | "corners"
    );

    let bw_px = Length::Fixed(border_width as f32);

    // Positions a border bar anywhere inside the box bounds.
    let place = |bar: Element<'a, Message>, x: Alignment, y: Alignment| -> Element<'a, Message> {
        container(bar)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(x)
            .align_y(y)
            .into()
    };

    // `width`/`height` carry the panel's thickness axis (Fill when the boxes
    // stretch, Shrink otherwise); the content stays content-sized and is
    // centred inside (a no-op on the Shrink axes).
    let base = container(content)
        .padding(padding)
        .width(width)
        .height(height)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme: &iced::Theme| {
            let mut style = ContainerStyle::default();
            if let Some(bg) = bg_color {
                style.background = Some(Background::Color(bg));
            }
            if !partial {
                if let Some(c) = border_color {
                    style.border.color = c;
                    // The native border carves the innermost `border_width`
                    // px of the quad: with an α=0 colour that ring renders as
                    // a hole (premultiplied to nothing) showing the panel
                    // behind — a transparent border draws no border at all.
                    style.border.width = if c.a == 0.0 {
                        0.0
                    } else {
                        border_width as f32
                    };
                    // Roadmap M3: the native border (line *and* background
                    // shape) rounds only the corners away from screen edges;
                    // touching ones render square. Radius is set here — the
                    // renderer composes background and border once, so a
                    // translucent colour shows no seams and nothing is ever
                    // drawn over the sensors.
                    style.border.radius = corner_radius(touch, border_radius as f32);
                }
            }
            style
        });

    // Full border: the per-corner radius above is all M3 needs — no overlay
    // strips (they used to sit over the content and double-blend the bg).
    if !partial {
        return base.into();
    }
    // Partial borders need a color and a positive width.
    let (Some(color), bw) = (border_color, border_width) else {
        return base.into();
    };
    if bw == 0 {
        return base.into();
    }

    let mut layers: Vec<Element<'a, Message>> = vec![base.into()];
    match border_type {
        "underline" => layers.push(place(
            border_bar(color, Length::Fill, bw_px),
            Alignment::Center,
            Alignment::End,
        )),
        "upperline" => layers.push(place(
            border_bar(color, Length::Fill, bw_px),
            Alignment::Center,
            Alignment::Start,
        )),
        "left" => layers.push(place(
            border_bar(color, bw_px, Length::Fill),
            Alignment::Start,
            Alignment::Center,
        )),
        "right" => layers.push(place(
            border_bar(color, bw_px, Length::Fill),
            Alignment::End,
            Alignment::Center,
        )),
        "corners" => {
            // Each corner is an L of two bars, each arm covering one quarter
            // of its side (FillPortion 1 vs spacer 3), so the middle of every
            // side keeps a gap whatever the sensor size is.
            let arm_h = |bar_first: bool, y: Alignment| -> Element<'a, Message> {
                let bar = border_bar(color, Length::FillPortion(1), bw_px);
                let spacer = Space::new().width(Length::FillPortion(3)).height(bw_px);
                let row = Row::new().width(Length::Fill).spacing(0);
                let row = if bar_first { row.push(bar).push(spacer) } else { row.push(spacer).push(bar) };
                container(row).width(Length::Fill).height(Length::Fill).align_y(y).into()
            };
            let arm_v = |bar_first: bool, x: Alignment| -> Element<'a, Message> {
                let bar = border_bar(color, bw_px, Length::FillPortion(1));
                let spacer = Space::new().width(bw_px).height(Length::FillPortion(3));
                let col = Column::new().height(Length::Fill).spacing(0);
                let col = if bar_first { col.push(bar).push(spacer) } else { col.push(spacer).push(bar) };
                container(col).width(Length::Fill).height(Length::Fill).align_x(x).into()
            };
            layers.push(arm_h(true, Alignment::Start));  // top edge, left quarter
            layers.push(arm_v(true, Alignment::Start));  // left edge, top quarter
            layers.push(arm_h(false, Alignment::Start)); // top edge, right quarter
            layers.push(arm_v(true, Alignment::End));    // right edge, top quarter
            layers.push(arm_h(true, Alignment::End));    // bottom edge, left quarter
            layers.push(arm_v(false, Alignment::Start)); // left edge, bottom quarter
            layers.push(arm_h(false, Alignment::End));   // bottom edge, right quarter
            layers.push(arm_v(false, Alignment::End));   // right edge, bottom quarter
        }
        _ => {}
    }

    Stack::with_children(layers)
        .width(width)
        .height(height)
        .into()
}

/// State of the watched config file: `(modified time, length)`.
/// `None` when no path is set or the file is missing/unreadable.
fn config_file_state() -> Option<(SystemTime, u64)> {
    let path = CONFIG_PATH.get()?.clone();
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Hot reload of the config file: returns `(parsed config, config path)`
/// only when the file changed on disk since the last attempt (mtime +
/// length check, so an unchanged file is not re-read every tick). The
/// attempt is recorded regardless of the outcome; a parse failure (e.g.
/// a half-written file) logs once per change and keeps the previous
/// config, retrying when the file changes again. The path rides along so
/// callers do not touch `CONFIG_PATH` again (one static ref per reload).
fn maybe_reload(panel: &mut Panel) -> Option<(config::Config, String)> {
    let state = config_file_state()?;
    if panel.last_reload == Some(state) {
        return None;
    }
    let path = CONFIG_PATH.get()?.clone();
    panel.last_reload = Some(state);

    let contents = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<config::Config>(&contents) {
        Ok(cfg) => {
            eprintln!("[piuma] config reloaded: {}", path);
            Some((cfg, path))
        }
        Err(e) => {
            eprintln!("[piuma] config changed but failed to parse ({}), keeping previous", e);
            None
        }
    }
}

// ── update ──────────────────────────────────────────────────────────

/// Applies a message to the panel. `timed`'s `Instant` parameter was dropped
/// in the migration to the `daemon` build pattern (multi-surface), and the
/// `Tick` payload with it: the tick only marks the second edge.
/// Spawn the tick's data pass: the shared compositor snapshot plus
/// every sensor's reading runs on a worker thread, and its reply
/// pairs them with the resolved styles (`SensorsReady` →
/// `apply_read`). One pass at a time (`read_pending`): replies stay
/// ordered and a slow sensor delays only the next rebuild, never the
/// UI thread. The config generation rides along so a reply built
/// against a reloaded config is dropped instead of misaligned.
fn spawn_read(panel: &mut Panel) -> Command<Message> {
    panel.read_pending = true;
    let cfg_gen = panel.cfg_gen;
    let sources: Vec<config::SensorSource> = panel
        .config
        .panel
        .sections
        .iter()
        .flat_map(|sec| &sec.sensors)
        .map(|s| s.source.clone())
        .collect();
    Command::perform(
        smol::unblock(move || sensor_runner::read_all(&sources)),
        move |reads| Message::SensorsReady(cfg_gen, reads),
    )
}

/// Pair one data pass with the resolved styles and run the downstream
/// work the tick used to do inline: graph rings, icon fits, popup
/// reaping and the popup/link sync against fresh sections. Returns
/// the window commands the caller batches.
fn apply_read(panel: &mut Panel, data: sensor_runner::Reads) -> Vec<Command<Message>> {
    let sensor_runner::Reads { hypr, values } = data;
    // One reading per configured sensor, consumed in config order.
    let mut reads = values.into_iter();
    // Samples of this pass for the graph history (one read per
    // sensor, collected here so the sections and the history never
    // borrow the panel at once).
    let mut graph_samples: Vec<((usize, usize), f64)> = Vec::new();
    let mut sections: Vec<SectionData> = Vec::new();
    // The styles are config-driven and already resolved (`restyle`,
    // on load/reload/palette change): this pass only pairs its
    // readings with them — no colour parsing, no shape strings per
    // sensor per second. The zip stops at the shorter side, so a
    // stale style list can never panic.
    for (section_idx, (section_cfg, section_style)) in panel
        .config
        .panel
        .sections
        .iter()
        .zip(&panel.styles)
        .enumerate()
    {
        let sensors: Vec<SensorValue> = section_cfg
            .sensors
            .iter()
            .zip(&section_style.sensors)
            .enumerate()
            .map(|(sensor_idx, (s, st))| {
                // In config order — the same flatten `spawn_read`
                // cloned the sources from; `missing` covers a reply
                // shorter than the sensor list.
                let read = reads.next().unwrap_or_else(sensor_runner::Read::missing);
                // Graph engine: record this pass's sample for the
                // sensors that opted in (`draw_graph` on a
                // graphable source); the columns ignore the rest.
                if s.draw_graph {
                    if let Some((sample, _)) = read.graph {
                        graph_samples.push(((section_idx, sensor_idx), sample));
                    }
                }
                let value = read.text;
                // Tray sensors render icons from the shared tray state
                // (tray.rs) instead of text.
                let tray_icons =
                    matches!(s.source, config::SensorSource::Tray {}).then(tray::items);
                // Workspace sensors build their button list from
                // the shared snapshot (roadmap2 #4): one entry per
                // available workspace, the active one flagged.
                let workspace_buttons = matches!(s.source, config::SensorSource::Workspace {})
                    .then(|| workspace::buttons(hypr.as_ref()));
                // Taskbar sensors rebuild their icon slots from the
                // shared snapshot per pass; the icons resolve
                // through the memoized theme lookups.
                let taskbar_icons = matches!(s.source, config::SensorSource::Taskbar {})
                    .then(|| windows::task_icons(hypr.as_ref()));
                // Only the net rate sensors pin their value box
                // (roadmap M5), and the pin is the displayed value's
                // OWN tier: exact width (no headroom for units this
                // state never shows), constant while the value moves
                // inside the tier (figure spaces), moving with it on
                // a ×1024 crossing — same pass, same string, so the
                // box can never undershoot its content. Computed
                // before the literal because `value` is moved into
                // it; the measurement is cached per
                // font/size/patterns, so the per-pass rebuild only
                // re-measures when the tier label changes.
                let fixed_width = matches!(
                    &s.source,
                    config::SensorSource::UsedNetup { .. }
                        | config::SensorSource::UsedNetdown { .. }
                )
                .then(|| {
                    let font = value_font(&panel.config);
                    let size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
                    text_width::widest(&font, size, &sensors::net_rate_samples(&value))
                });
                SensorValue {
                    name: st.name.clone(),
                    description: st.description.clone(),
                    value,
                    text_color: Some(st.text_color),
                    color_rules: st.color_rules.clone(),
                    bg_color: st.bg_color,
                    text_color_hover: st.text_color_hover,
                    bg_color_hover: st.bg_color_hover,
                    border_color: st.border_color,
                    border_width: st.border_width,
                    border_radius: st.border_radius,
                    border_type: st.border_type.clone(),
                    padding: st.padding,
                    action: st.action.clone(),
                    tray: tray_icons,
                    workspaces: workspace_buttons,
                    taskbar: taskbar_icons,
                    is_notifications: st.is_notifications,
                    icon: st.icon.clone(),
                    icon_size: st.icon_size,
                    fixed_width,
                }
            })
            .collect();

        sections.push(SectionData {
            name: section_style.name.clone(),
            layout: section_style.layout.clone(),
            bg_color: section_style.bg_color,
            text_color: section_style.text_color,
            title_color: section_style.title_color,
            border_color: section_style.border_color,
            border_width: section_style.border_width,
            border_radius: section_style.border_radius,
            border_type: section_style.border_type.clone(),
            touch: section_style.touch,
            sensors,
        });
    }
    panel.sections = sections;
    // One ring advance per pass: columns repeat their last value
    // when a reading was missing (time stays uniform).
    panel.graphs.record(&graph_samples);
    // Icons size to the space their sensor box really has; both the
    // view and the icon resolution read tray::icon_target.
    update_tray_icon_target(panel);
    update_taskbar_icon_target(panel);

    let mut cmds: Vec<Command<Message>> = Vec::new();
    // A popup that lost both its sensor and its pointer gets
    // reaped once its grace runs out: the sensor's leave can act
    // on stale coordinates and, after the cursor rests on another
    // application's window, no event ever arrives again.
    if let Some(gp) = reap_abandoned_popup(panel) {
        cmds.push(iced::window::close(gp.id));
    }
    // The calendar popup's three-second grace runs on the same
    // pass: neither sensor nor surface under the cursor spends it.
    if let Some(cp) = reap_abandoned_calendar(panel) {
        cmds.push(iced::window::close(cp.id));
    }
    // The popup follows the active set (and re-anchors with the bar
    // on a geometry change) — one place drives its window.
    let resync = std::mem::take(&mut panel.popup_resync);
    if let Some(cmd) = sync_popup(panel, resync) {
        cmds.push(cmd);
    }
    // The connector surface rides with the popup: it opens once the
    // sensor rect is known and dies with the popup — one place
    // drives its window too.
    if let Some(cmd) = sync_graph_link(panel) {
        cmds.push(cmd);
    }
    cmds
}

fn update(panel: &mut Panel, message: Message) -> Command<Message> {
    match message {
        Message::Tick => {
            // Hot reload: pick up config file changes (content and, when
            // layout/align differ from what is applied, surface geometry).
            let mut geometry_changed = false;
            let mut zone_changed = false;
            // Any reload re-anchors/resizes the popup too: the popup style
            // (font/padding) lives in the config without affecting the bar's
            // own geometry, so its reposition rides on this flag.
            let mut config_reloaded = false;
            if let Some((reloaded, path)) = maybe_reload(panel) {
                config_reloaded = true;
                panel.config = reloaded;
                // An in-flight data pass was flattened against the
                // previous sensor list: its reply gets dropped.
                panel.cfg_gen += 1;
                // The colour chains follow the new config (see
                // `restyle`); the readings and the geometry below
                // don't need them, so one resolve covers the tick.
                restyle(panel);
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
                let size = panel_size(&panel.config, unsafe { SCREEN_WIDTH }, unsafe { SCREEN_HEIGHT });
                let anchor = panel_anchor(&panel.config);
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
            }

            // Geometry or config moved: the reply that carries the
            // fresh sections re-anchors the popup on them
            // (`apply_read` consumes this). The data pass itself runs
            // on a worker — the UI thread never forks a sensor.
            panel.popup_resync |= geometry_changed || config_reloaded;
            let mut cmds: Vec<Command<Message>> = Vec::new();
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
            }
            // An open graph popup dies with a reload too: its columns
            // were rebuilt underneath it (and its sensor may be gone).
            // The calendar popup goes with it: its month came from the
            // previous config's sensor.
            if config_reloaded {
                if let Some(gp) = panel.graph_popup.take() {
                    cmds.push(iced::window::close(gp.id));
                }
                if let Some(cp) = panel.calendar.take() {
                    cmds.push(iced::window::close(cp.id));
                }
            }
            // The popup follows the active set (and re-anchors with the
            // bar on a geometry change) — one place drives its window.
            if let Some(cmd) = sync_popup(panel, geometry_changed || config_reloaded) {
                cmds.push(cmd);
            }
            // The connector surface rides with the popup: it opens once
            // the sensor rect is known and dies with the popup — one
            // place drives its window too.
            if let Some(cmd) = sync_graph_link(panel) {
                cmds.push(cmd);
            }
            // The wallpaper surface follows the config on the same
            // cadence: the first tick materializes it after startup,
            // later ticks track enable/path changes (a cheap compare).
            cmds.push(sync_wallpaper(panel));
            // The derived palette follows the same cadence: extracted
            // off-thread only when the wallpaper file changes, dropped
            // when the option turns off.
            cmds.push(sync_palette(panel));
            // The data pass (compositor snapshot + every reading) runs
            // on a worker; its reply rebuilds the sections. A pass
            // already in flight covers this second — one at a time
            // keeps replies ordered.
            if !panel.read_pending {
                cmds.push(spawn_read(panel));
            }
            Command::batch(cmds)
        }
        Message::SensorsReady(cfg_gen, data) => {
            panel.read_pending = false;
            if cfg_gen != panel.cfg_gen {
                // The config moved while this read ran: its readings
                // no longer line up with the sensor list. Drop them
                // and read again at once.
                return spawn_read(panel);
            }
            Command::batch(apply_read(panel, data))
        }
        Message::PaletteReady(pal, key) => {
            // The next view applies the tones — or resolves the user's
            // own colours again when the extraction failed (`None`).
            panel.palette = pal;
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
                    let (id, _) = panel.tray_menu.take().unwrap();
                    panel.tray_menu_hover = None;
                    cmds.push(iced::window::close(id));
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
            // only the feed sensor's text moves (see
            // `refresh_tray_and_notif`); the popup and the list read
            // the shared state on their own.
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
                cmds.push(iced::window::close(id));
                if menu.bus == bus
                    && menu.path == path
                    && target == tray::ClickTarget::Menu
                {
                    return if cmds.is_empty() {
                        Command::none()
                    } else {
                        Command::batch(cmds)
                    };
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
            let (w, h) = menu_size(&entries, &font, font_size);
            // The surface width is final now: overlong labels are cut to
            // their row budget (single line + ellipsis in the view).
            fit_menu_labels(&mut entries, w, &font, font_size);
            let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
            let (anchor, margin) = menu_placement(at.0, at.1, w, h, sw, sh);
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: LayerSize::px(w, h),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: None,
                margin: Some(margin),
                keyboard_interactivity: KeyboardInteractivity::None,
                // The bar's own output: `LastOutput` landed the popup
                // on a second monitor once one joined the session.
                output_option: popup_output_option(),
                namespace: Some("piuma-tray-menu".to_string()),
                ..Default::default()
            });
            panel.tray_menu = Some((
                id,
                TrayMenu {
                    bus,
                    path,
                    menu_path,
                    entries,
                },
            ));
            open
        }
        Message::TrayMenuClicked(entry_id) => {
            if let Some((id, menu)) = panel.tray_menu.take() {
                panel.tray_menu_hover = None;
                dbusmenu::spawn_event(&menu.bus, &menu.menu_path, entry_id);
                iced::window::close(id)
            } else {
                Command::none()
            }
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
            if let Some((id, _)) = panel.tray_menu.take() {
                panel.tray_menu_hover = None;
                iced::window::close(id)
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
        Message::SensorPressed(section_index, sensor_index, action) => {
            let action = action.filter(|a| !a.trim().is_empty());
            match action.as_deref() {
                Some(ACTION_CLOSE_PANEL) => {
                    // Panel teardown handled in main via a flag.
                    unsafe { PANEL_SHOULD_CLOSE = true; }
                }
                Some(command) => {
                    // Shell action: run in background, non-blocking.
                    let cmd = command.to_string();
                    std::thread::spawn(move || {
                        let _ = std::process::Command::new("sh")
                            .args(["-c", &cmd])
                            .spawn();
                    });
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
                if matches!(source, Some(config::SensorSource::Notifications {})) {
                    notifications::scroll(up);
                    // Rewrite the value at once; the next tick confirms it.
                    if let Some(sensor_value) = panel
                        .sections
                        .get_mut(section_index)
                        .and_then(|sec| sec.sensors.get_mut(sensor_index))
                    {
                        sensor_value.value = notifications::display_value();
                    }
                    return Command::none();
                }
                if let Some(config::SensorSource::PercVolume { device }) = source {
                    let device = device.clone();
                    // Reflect the change on the bar right away (same math as
                    // the backend adjust); the next tick confirms it.
                    if let Some(sensor_value) = panel
                        .sections
                        .get_mut(section_index)
                        .and_then(|sec| sec.sensors.get_mut(sensor_index))
                    {
                        if let Some(next) = sensors::next_volume(&sensor_value.value, up) {
                            sensor_value.value = format!("{next:.0}%");
                        }
                    }
                    std::thread::spawn(move || sensors::adjust_volume(&device, up));
                }
            }
            Command::none()
        }
        Message::NotifListScrolled(delta_y) => {
            // Wheel over the list: whole rows per notch (the surface
            // height is exact, so the math matches the layout).
            if let Some((_, state)) = panel.notif_list.as_mut() {
                let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
                let row_h = notif_list_row_h(font_size) as f32;
                let rows = (-delta_y / row_h).round() as i32;
                let len = notifications::history_rows(NOTIF_LIST_MAX).len();
                state.offset = scroll_rows(state.offset, rows, len, NOTIF_LIST_ROWS);
            }
            Command::none()
        }
        Message::NotifListClicked(id) => {
            // A row runs its notification's spec `default` action (the
            // sensor's old click) and the popup closes with it.
            if let Some((win, _)) = panel.notif_list.take() {
                notifications::invoke_id(id);
                return iced::window::close(win);
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
                cmds.push(iced::window::close(id));
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
            let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
            let (anchor, margin) = menu_placement(at.0, at.1, w, h, sw, sh);
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: LayerSize::px(w, h),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: None,
                margin: Some(margin),
                keyboard_interactivity: KeyboardInteractivity::None,
                output_option: popup_output_option(),
                namespace: Some("piuma-taskbar-menu".to_string()),
                ..Default::default()
            });
            panel.taskbar_menu = Some((id, TaskbarMenu { address }));
            cmds.push(open);
            Command::batch(cmds)
        }
        Message::TaskbarMenuWs(workspace) => {
            // The menu carries the target: move it silently (the bar's
            // workspace stays) and the menu goes with the action.
            if let Some((id, menu)) = panel.taskbar_menu.take() {
                panel.taskbar_menu_hover = None;
                windows::move_to_workspace(&menu.address, workspace);
                return iced::window::close(id);
            }
            Command::none()
        }
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
                return iced::window::close(id);
            }
            Command::none()
        }
        Message::SensorHovered(section_index, sensor_index, hovered) => {
            if hovered {
                panel.hovered = Some((section_index, sensor_index));
            } else if panel.hovered == Some((section_index, sensor_index)) {
                panel.hovered = None;
            }
            hover_sync(panel, section_index, sensor_index, hovered)
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
                iced::window::close(gp.id)
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
        Message::SensorRectFound(section, sensor, rect) => {
            // The queried bounds are in the bar window's coordinates:
            // lift them to output space (the bar sits inside the output
            // at its anchor) — the popup's own rects live there too.
            let (ox, oy) = tray::bar_origin(panel);
            if let Some(gp) = panel.graph_popup.as_mut() {
                if (gp.section, gp.sensor) == (section, sensor) {
                    gp.sensor_rect = Some((rect.0 + ox, rect.1 + oy, rect.2 + ox, rect.3 + oy));
                }
            }
            // The answer lands right after the popup opened: open the
            // connector at once (the tick owns the closing side).
            sync_graph_link(panel).unwrap_or_else(Command::none)
        }
        _ => Command::none(),
    }
}

/// The theme to resolve sensor icons under, or `None` while no sensor of
/// the config declares one (roadmap N1): the gate keeps the whole icon
/// machinery — theme parsing, file lookups, cache — dormant otherwise, and
/// it fires before the first `resolve` even happens. `panel.icon_theme`
/// wins; without it the tray's GTK rule applies.
pub(crate) fn icons_theme(config: &config::Config) -> Option<&str> {
    let declares_icon = config
        .panel
        .sections
        .iter()
        .flat_map(|section| section.sensors.iter())
        .any(|sensor| sensor.icon.is_some());
    if !declares_icon {
        return None;
    }
    // A closure, not the fn item: `unwrap_or_else` would otherwise unify
    // `T` with the item's `&'static str` and demand `config: 'static`.
    Some(config.panel.icon_theme.as_deref().unwrap_or_else(|| icons::icon_theme()))
}

// ── subscription (timer + iced events) ──────────────────────────────

fn subscription(panel: &Panel) -> Subscription<Message> {
    // The tray's D-Bus side runs only while a tray sensor is configured;
    // dropping it releases the watcher/host bus names automatically.
    let tray_sub = panel
        .config
        .panel
        .sections
        .iter()
        .flat_map(|section| section.sensors.iter())
        .any(|s| matches!(s.source, config::SensorSource::Tray {}))
        .then(tray::subscription);
    // The notifications D-Bus server runs only while the block is present
    // (its subscription claims the bus name at startup, before any client
    // can activate the name's activator); dropping it releases the name.
    // `configure` keeps the live settings in step on every (re)build.
    let notif_sub = panel
        .config
        .panel
        .notifications
        .as_ref()
        .map(|cfg| {
            notifications::configure(cfg);
            notifications::subscription()
        });
    Subscription::batch(
        [
            time::every(std::time::Duration::from_secs(1)).map(|_| Message::Tick),
            event::listen_with(|event, _status, id| Some(Message::IcedEvent(id, event))),
        ]
        .into_iter()
        .chain(tray_sub)
        .chain(notif_sub),
    )
}

// ── view ────────────────────────────────────────────────────────────

/// Id of the bar's layer surface, captured on the first render of the
/// bar itself (the popup renders its own tree and never reaches this
/// line, so the first bar `view` call sets it regardless of the order
/// in which the surfaces come up). `LayoutChange` and
/// `ExclusiveZoneChange` carry it to keep targeting the bar once other
/// surfaces are registered — the multi-surface runtime keys actions by id.
pub(crate) static BAR_ID: std::sync::OnceLock<iced::window::Id> = std::sync::OnceLock::new();

/// Configured family names kept alive for [`Font::Name`]'s `'static`
/// bound: a name is leaked the first time it is used and the same
/// pointer is reused after, so reloads — even alternating between two
/// fonts — never leak a fresh copy per change.
pub(crate) static FONT_NAMES: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());




fn view<'a>(panel: &'a Panel, window: iced::window::Id) -> Element<'a, Message> {
    // Side windows render their own trees; only bar calls feed BAR_ID.
    if panel.wallpaper.iter().any(|(_, id)| *id == window) {
        return wallpaper_view(panel);
    }
    if let Some((id, _)) = panel.tray_menu.as_ref() {
        if *id == window {
            return tray_menu_view(panel);
        }
    }
    if let Some((id, _)) = panel.notif_list.as_ref() {
        if *id == window {
            return notif_list_view(panel);
        }
    }
    if let Some((id, _)) = panel.taskbar_menu.as_ref() {
        if *id == window {
            return taskbar_menu_view(panel);
        }
    }
    if let Some((id, _)) = panel.bar_tooltip.as_ref() {
        if *id == window {
            return tooltip_window_view(panel);
        }
    }
    if let Some(gp) = panel.graph_popup.as_ref() {
        if gp.id == window {
            return graph_popup_view(panel, gp);
        }
    }
    if let Some(cp) = panel.calendar.as_ref() {
        if cp.id == window {
            return calendar_popup_view(panel, cp);
        }
    }
    if panel.graph_link.is_some_and(|(id, _)| id == window) {
        return graph_link_view(panel);
    }
    if panel.popup == Some(window) {
        return popup_view(panel);
    }
    let _ = BAR_ID.set(window);
    let font_obj = panel_font(&panel.config);
    // Value texts can ride a separate (typically monospaced) family.
    let value_font_obj = value_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);

    // Panel orientation: the `thickness` axis (the "cross" axis of the panel)
    // is the height for a horizontal panel and the width for a vertical one.
    let vertical = panel
        .config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal") == "vertical";

    // Build sections.
    let mut section_elements: Vec<Element<'a, Message>> = Vec::new();

    for (section_idx, section) in panel.sections.iter().enumerate() {
        let section_bg = section.bg_color;
        let section_text = section.text_color;
        let has_title = !section.name.is_empty();
        let title_color = section.title_color;

        // A vertical panel stacks every section's sensors down its height,
        // whatever the section's own layout says (a row would squeeze them
        // across the thickness): `stack_sensors` is that effective axis.
        let stack_sensors = vertical || section.layout == "vertical";
        // Sensor boxes fill the `thickness` axis only when the sensors run
        // along the main axis (row across a horizontal panel, column down a
        // vertical one); stacked along the thickness they stay content-sized
        // (filling would split the thickness between them) and the group is
        // centred by the container.
        let stretch = vertical || !stack_sensors;
        let (box_w, box_h) = match (vertical, stretch) {
            (false, true) => (Length::Shrink, Length::Fill),
            (true, true) => (Length::Fill, Length::Shrink),
            _ => (Length::Shrink, Length::Shrink),
        };

        // Build sensor widgets with optional per-sensor background. A tray
        // sensor with no items renders nothing at all (no empty box), and
        // so does the notifications sensor with nothing active — its
        // display value is empty in that case (roadmap2 #3), and it
        // reappears with the next notification.
        let sensor_widgets: Vec<Element<'a, Message>> = section.sensors.iter().enumerate().filter_map(|(sensor_idx, s)| {
            if s.tray.as_ref().is_some_and(|icons| icons.is_empty()) {
                return None;
            }
            let is_notifications = panel
                .config
                .panel
                .sections
                .get(section_idx)
                .and_then(|sec| sec.sensors.get(sensor_idx))
                .is_some_and(|cfg_s| matches!(cfg_s.source, config::SensorSource::Notifications {}));
            if is_notifications && s.value.is_empty() {
                return None;
            }
            // A workspace sensor with no buttons (hyprctl unreachable)
            // hides like the tray does (roadmap2 #4).
            if s.workspaces.as_ref().is_some_and(|b| b.is_empty()) {
                return None;
            }
            // Same rule for a taskbar whose client list came back empty
            // (hyprctl unreachable): no empty icon strip.
            if s.taskbar.as_ref().is_some_and(|slots| slots.is_empty()) {
                return None;
            }
            let sensor_text_color = s.text_color.as_ref().map(|c| *c).unwrap_or(section_text);

            // Text colour: a threshold rule the reading reaches (the
            // highest `min` ≤ value) tints the value; the hover variant
            // still wins while the cursor is over the sensor.
            let ruled = sensors::leading_number(&s.value)
                .and_then(|v| {
                    s.color_rules
                        .iter()
                        .rev()
                        .find(|(min, _)| v >= *min)
                        .map(|(_, c)| *c)
                })
                .unwrap_or(sensor_text_color);
            let text_color = if panel.hovered == Some((section_idx, sensor_idx)) {
                s.text_color_hover.unwrap_or(ruled)
            } else {
                ruled
            };

            // The value text of net rates pins its box width (roadmap M5):
            // the container carries the fixed width so the text itself never
            // wraps, whatever the content.
            let value_text = text(&s.value)
                .color(text_color)
                .size(font_size)
                .font(value_font_obj);
            let value_widget: Element<'a, Message> = match s.fixed_width {
                Some(width) => container(value_text)
                    .width(Length::Fixed(f32::from(width)))
                    .into(),
                None => value_text.into(),
            };

            // Name and value side by side, separated by a small gap (the
            // colon separator was removed on request); an empty name (e.g.
            // static_text sensors) renders the value alone, with no gap.
            // A vertical panel stacks the value UNDER the name instead:
            // its width is there to spend, and one long line would leave
            // the value stranded at the far right of a thick side panel.
            // Tray sensors skip the text row: `tray::row` builds their
            // icon row with clicks, wheel and tooltip included. The style
            // passed here drives the tooltip, which mirrors the sensor box;
            // a transparent sensor shows the section behind it on the bar,
            // so the tooltip borrows that colour to stay legible anywhere.
            let main: Element<'a, Message> = if let Some(icons) = &s.tray {
                tray::row(icons)
            } else if let Some(buttons) = &s.workspaces {
                // One button per available workspace (roadmap2 #4): the
                // names shown, the active one inverted, click focuses —
                // the buttons own their mouse handling like tray icons.
                // The sensor's own icon still leads the row (its config
                // predates the buttons), centred like every other sensor
                // row so it never rides above the baseline.
                let ws_row = workspace::row(
                    buttons,
                    font_size,
                    font_obj,
                    &workspace::Style {
                        bg_color: s.bg_color.unwrap_or(section_bg),
                        text_color,
                        border_color: s.border_color,
                        border_width: s.border_width,
                        border_radius: s.border_radius,
                        border_type: s.border_type.clone(),
                        padding: s.padding,
                    },
                );
                let mut row = Row::new().spacing(6).align_y(Alignment::Center);
                if let Some(path) = &s.icon {
                    row = row.push(icons::widget(path, s.icon_size as f32));
                }
                row.push(ws_row).into()
            } else if let Some(slots) = &s.taskbar {
                // One icon per open window: the slots own their mouse
                // handling like tray icons (left focuses, middle closes,
                // right opens the move menu); their fills and tooltip
                // come from the sensor's own colours — the active
                // window keeps the sensor's background, the hovered
                // slot wins over it.
                let active_bg = s.bg_color.or(Some(section_bg));
                let row_style = windows::RowStyle {
                    radius: s.border_radius,
                    hover_bg: s.bg_color_hover.or(active_bg),
                    active_bg,
                };
                windows::row(slots, panel.taskbar_hover.as_deref(), &row_style).into()
            } else {
                // Icon first when the sensor declares one (roadmap N1), then
                // the optional name, then the value: an empty name still
                // renders the value alone, without a gap. The row centres
                // its children: the icon box is `font_size` square while a
                // text line box is `font_size × 1.3` (iced's default line
                // height), so a top-aligned row left the icon riding ~2px
                // above the text baseline (roadmap2: icon alignment).
                let mut row = Row::new().spacing(6).align_y(Alignment::Center);
                if let Some(path) = &s.icon {
                    row = row.push(icons::widget(path, s.icon_size as f32));
                }
                if s.name.is_empty() {
                    row = row.push(value_widget);
                } else {
                    let name_text = text(&s.name)
                        .color(text_color)
                        .size(font_size)
                        .font(font_obj);
                    if vertical {
                        row = row.push(Column::new().push(name_text).push(value_widget).spacing(2));
                    } else {
                        row = row.push(name_text).push(value_widget);
                    }
                }
                row.into()
            };

            let desc_str: String = s.description.clone();
            let has_desc = !desc_str.is_empty();
            let desc_value = desc_str.clone();

            let content: Element<'a, Message> = if has_desc {
                let desc_widget = text(desc_value)
                    .color(Color::from_rgba(0.5, 0.5, 0.5, 0.7))
                    .size(font_size.saturating_sub(2))
                    .font(font_obj);
                Column::new().push(main).push(desc_widget).into()
            } else {
                main
            };

            // Determine colors: hover when the cursor is over this sensor.
            let is_hovered = panel.hovered == Some((section_idx, sensor_idx));
            let bg_color = if is_hovered {
                s.bg_color_hover.or(s.bg_color)
            } else {
                s.bg_color
            };
            let border_color = s.border_color;
            // A workspace sensor's buttons carry the sensor's own
            // padding (they are the boxes), so the outer box adds none:
            // the pill keeps the height and inset of every other sensor
            // instead of stacking the padding twice (roadmap2 #4).
            let pad = if s.workspaces.is_some() { 0 } else { s.padding };

            let styled_container = styled_box(
                content,
                bg_color,
                border_color,
                s.border_width,
                s.border_radius,
                &s.border_type,
                // Sensors sit inside sections: never on a screen edge.
                [false, false, false, false],
                pad,
                box_w,
                box_h,
            );
            // Tagged so the graph popup can query this box's on-screen
            // rect and pin its connector window to it.
            let styled_container =
                container(styled_container).id(sensor_box_id(section_idx, sensor_idx));

            // Tray icons carry their own MouseAreas (press/wheel/tooltip);
            // wrapping them in the sensor MouseArea as well would handle the
            // same events twice, so a tray sensor stays bare — and so does a
            // workspace sensor (its buttons press themselves) and the
            // taskbar (its icon slots press themselves).
            if s.tray.is_some() || s.workspaces.is_some() || s.taskbar.is_some() {
                return Some(styled_container.into());
            }

            // Wrap in MouseArea to deliver hover + click for this specific sensor.
            Some(
                MouseArea::new(styled_container)
                .on_enter(Message::SensorHovered(section_idx, sensor_idx, true))
                .on_exit(Message::SensorHovered(section_idx, sensor_idx, false))
                .on_press(Message::SensorPressed(
                    section_idx,
                    sensor_idx,
                    s.action.clone(),
                ))
                // Wheel over a volume sensor adjusts it (positive y = wheel
                // up, verified on this stack). Per notch the stack sends a
                // pixel delta, a zero delta (axis_source) and the same delta
                // in lines: only non-zero pixel deltas act — the line form
                // always accompanies the pixel one (wayland protocol), so
                // exactly one adjustment happens per notch.
                .on_scroll(move |delta| {
                    let up = match delta {
                        iced::mouse::ScrollDelta::Pixels { y, .. } if y != 0.0 => Some(y > 0.0),
                        _ => None,
                    };
                    Message::SensorScrolled(section_idx, sensor_idx, up)
                })
                .interaction(iced::mouse::Interaction::Pointer)
                .into()
            )
        }).collect();

        // Optionally prepend a section title.
        let sensors_inner: Element<'a, Message> = if stack_sensors {
            let title_widget = container(
                text(section.name.clone())
                    .color(title_color)
                    .size(font_size.saturating_add(2))
                    .font(font_obj)
            ).align_x(Alignment::Center);
            let title_el: Element<'a, Message> = title_widget.padding(SECTION_CONTENT_PAD).into();
            // Spaced like the horizontal row: stacked boxes must not
            // touch and form one continuous strip.
            let sensor_col = Column::new()
                .extend(sensor_widgets)
                .spacing(SENSOR_SPACING)
                .padding(SECTION_CONTENT_PAD);
            // `stretch` here means: stacked sensors that must fill the
            // thickness (width) so the boxes stretch — a vertical panel
            // always, a horizontal one only for vertical sections; the
            // title stays content-sized and is centred above them.
            let sensor_col = if stretch { sensor_col.width(Length::Fill) } else { sensor_col };
            let col = if has_title {
                let col = Column::new().push(title_el).push(sensor_col);
                if stretch { col.width(Length::Fill).align_x(Alignment::Center) } else { col }
            } else {
                sensor_col
            };
            col.into()
        } else {
            let row = Row::new()
                .extend(sensor_widgets)
                .spacing(SENSOR_SPACING)
                .padding(SECTION_CONTENT_PAD);
            // `stretch` here means: horizontal section in a horizontal panel ->
            // fill the thickness (height) so the boxes stretch.
            let row = if stretch { row.height(Length::Fill) } else { row };
            row.into()
        };

        // Wrap in a styled box: section background plus the section border,
        // drawn with the same rules as the sensor boxes. It fills the panel's
        // cross (thickness) axis and centres the section content on that axis,
        // so `thickness` reaches the sections: stretched boxes fill it, stacked
        // groups are centred within it. Content padding stays in the Row/
        // Column above, so the box itself adds none.
        let section_element: Element<'a, Message> = styled_box(
            sensors_inner,
            Some(section_bg),
            section.border_color,
            section.border_width,
            section.border_radius,
            &section.border_type,
            section.touch,
            0,
            if vertical { Length::Fill } else { Length::Shrink },
            if vertical { Length::Shrink } else { Length::Fill },
        );

        section_elements.push(section_element);
    }

    // `align` decides how the sections are spread inside the surface; read
    // it before building the layout so `fill` can pick its own container.
    let align = panel
        .config
        .panel
        .align
        .as_deref()
        .unwrap_or("center");
    let fill = align == "fill";

    // Build the full panel layout. The main axis shrinks to the content (so
    // `align` can place the panel within the surface); the cross axis fills
    // the surface thickness, which is what makes `thickness` reach the
    // sections and the sensor boxes. With `align: "fill"` the sections are
    // spread along the whole main axis by `FillSections` instead.
    let main_layout: Element<'a, Message> = if fill {
        FillSections::new(vertical, section_elements).into()
    } else if vertical {
        Column::new()
            .extend(section_elements)
            .width(Length::Fill)
            .into()
    } else {
        Row::new()
            .extend(section_elements)
            .height(Length::Fill)
            .into()
    };

    let panel_bg = panel_background(&panel.config, panel.palette.as_ref());

    // Position the real panel within the surface according to `align`.
    // The surface is screen-sized on the cross axis and sized to the panel's
    // own thickness on the main axis. The `align` must be applied on a
    // container that fills the main axis of the surface; applied on the
    // shrink-wrapped panel it would be a no-op (centering within itself).
    // With `align: "fill"` the panel fills the main axis by itself, so the
    // container alignment below becomes a no-op too.
    let panel_container = container(main_layout)
        .width(if fill || vertical { Length::Fill } else { Length::Shrink })
        .height(if fill || !vertical { Length::Fill } else { Length::Shrink })
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(panel_bg)),
            ..ContainerStyle::default()
        });

    // Outer container fills the surface on BOTH axes, so `align` has room to
    // move the (shrink) panel on its main axis: left / center / right
    // (horizontal) or top / center / bottom (vertical). "fill" lands on the
    // `_` arm on both axes, where there is nothing to move: the panel
    // already spans the surface.
    let surface_container = container(panel_container)
        .width(Length::Fill)
        .height(Length::Fill);

    let surface_container = if vertical {
        surface_container.align_y(match align {
            "top" => Alignment::Start,
            "bottom" => Alignment::End,
            _ => Alignment::Center,
        })
    } else {
        surface_container.align_x(match align {
            "left" => Alignment::Start,
            "right" => Alignment::End,
            _ => Alignment::Center,
        })
    };

    surface_container.into()
}

// ── positioning helpers ─────────────────────────────────────────────


/// Recompute `tray::icon_target` from the built sections (every tick and on
/// every reload): the smallest fit across all tray sensors wins, so one icon
/// size serves the whole bar and no tray sensor overflows. The stacked-title
/// term applies only to vertically laid out sections, where the title shares
/// the thickness axis; with no tray sensor at all the target is left alone.
fn update_tray_icon_target(panel: &Panel) {
    let thickness = panel_thickness(&panel.config);
    let font = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let mut smallest: Option<u16> = None;
    for section in &panel.sections {
        // Title height as the view draws it, shared with the sensor icon
        // fit: text in a box padded SECTION_CONTENT_PAD on both sides.
        let title_extra = icons::stacked_title_extra(&section.layout, &section.name, font);
        for sensor in &section.sensors {
            if sensor.tray.is_some() {
                let target =
                    icons::row_icon_target(thickness, title_extra, u32::from(sensor.padding));
                smallest = Some(smallest.map_or(target, |m| m.min(target)));
            }
        }
    }
    if let Some(target) = smallest {
        tray::set_icon_target(target);
    }
}

/// The taskbar slots' icon edge: the row height the taskbar sensor
/// really has (`row_icon_target`), minus the slot padding around each
/// icon — the same fit the tray icons and the sensor icons use, so the
/// slots fill the bar instead of sitting at a fixed 16 px.
fn update_taskbar_icon_target(panel: &Panel) {
    let thickness = panel_thickness(&panel.config);
    let font = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let mut smallest: Option<u16> = None;
    for section in &panel.sections {
        let title_extra = icons::stacked_title_extra(&section.layout, &section.name, font);
        for sensor in &section.sensors {
            if sensor.taskbar.is_some() {
                let target = icons::row_icon_target(
                    thickness,
                    title_extra,
                    u32::from(sensor.padding) + u32::from(windows::SLOT_PAD),
                );
                smallest = Some(smallest.map_or(target, |m| m.min(target)));
            }
        }
    }
    if let Some(target) = smallest {
        windows::set_icon_target(target);
    }
}

/// Tray and notification events refresh only their own sensors: the
/// slots and the feed text come from the shared in-memory state and
/// every other reading stays what the last tick read — a chatty tray
/// icon must not re-fork (and re-read) the whole sensor set. The
/// tint follows in the view: colour rules are evaluated per render.
fn refresh_tray_and_notif(panel: &mut Panel) {
    for section in &mut panel.sections {
        for sensor in &mut section.sensors {
            if sensor.tray.is_some() {
                sensor.tray = Some(tray::items());
            }
            if sensor.is_notifications {
                sensor.value = notifications::display_value();
            }
        }
    }
}











// ── popup window (F3) ───────────────────────────────────────────────

/// Keep the popup window in step with the active notifications: no
/// notifications block or nothing active closes it, the first card
/// opens it (Overlay layer, opposite the bar's edge, exclusive zone 0)
/// and a count change resizes the surface to the stack's exact height.
/// `reposition` forces the resize/anchor message (bar geometry changed).
// ── popup style ─────────────────────────────────────────────────────

/// Resolved popup card style: `panel.notifications` values, with the
/// popup defaults for absent/invalid keys — white text on black, grey
/// border (the bar's own colors are not inherited here).
#[derive(Debug, PartialEq)]
pub(crate) struct PopupStyle {
    /// Base text size (body); the summary is +3 bold, the ✕ +2.
    pub(crate) font_size: u32,
    pub(crate) text: Color,
    pub(crate) text_hover: Color,
    pub(crate) bg: Color,
    pub(crate) bg_hover: Color,
    pub(crate) border: Color,
    pub(crate) border_hover: Color,
    pub(crate) border_width: u32,
    pub(crate) border_radius: u32,
    pub(crate) border_type: String,
    pub(crate) padding: u32,
}

impl Default for PopupStyle {
    fn default() -> Self {
        Self {
            font_size: 12,
            text: Color::WHITE,
            text_hover: Color::WHITE,
            bg: Color::BLACK,
            bg_hover: Color::BLACK,
            border: Color::from_rgb(0.5, 0.5, 0.5),
            border_hover: Color::from_rgb(0.5, 0.5, 0.5),
            border_width: 1,
            border_radius: 10,
            border_type: "full".to_string(),
            padding: 12,
        }
    }
}

/// The popup style from `panel.notifications`: negative/absent numerics
/// and unparsable colors fall back to `PopupStyle::default()`, an unknown
/// `border_type` to `full` (the `styled_box` rule). An active wallpaper
/// palette replaces the card's configured COLOURS (bg/hover from the
/// tones, borders from the vibrant one, text from the text tone);
/// sizes and the border type stay configured.
pub(crate) fn popup_style(config: &config::Config, pal: Option<&palette::Palette>) -> PopupStyle {
    let d = PopupStyle::default();
    let Some(n) = config.panel.notifications.as_ref() else {
        return d;
    };
    let color = |key: &Option<String>, tone: Option<&str>, fallback: Color| {
        let configured = key.as_deref().and_then(|s| config::parse_color(s).ok());
        tone.and_then(|t| config::parse_color(t).ok())
            .map(|mut t| {
                // The configured transparency survives the palette's hue.
                if let Some(c) = configured {
                    t.a = c.a;
                }
                t
            })
            .or(configured)
            .unwrap_or(fallback)
    };
    let num = |key: Option<i32>, fallback: u32| {
        key.filter(|v| *v >= 0).map(|v| v as u32).unwrap_or(fallback)
    };
    PopupStyle {
        font_size: n
            .font_size
            .filter(|v| (8..=64).contains(v))
            .unwrap_or(d.font_size),
        text: color(&n.text_color, pal.map(|p| p.text.as_str()), d.text),
        text_hover: color(
            &n.text_color_hover,
            pal.map(|p| p.text_hover.as_str()),
            d.text,
        ),
        bg: color(&n.bg_color, pal.map(|p| p.card.as_str()), d.bg),
        bg_hover: color(&n.bg_color_hover, pal.map(|p| p.card_hover.as_str()), d.bg),
        border: color(
            &n.border_color,
            pal.map(|p| p.card_hover.as_str()),
            d.border,
        ),
        border_hover: color(
            &n.border_color_hover,
            pal.map(|p| p.card_hover.as_str()),
            d.border,
        ),
        border_width: num(n.border_width, d.border_width),
        border_radius: num(n.border_radius, d.border_radius),
        border_type: n
            .border_type
            .as_deref()
            .filter(|t| {
                matches!(
                    *t,
                    "full" | "corners" | "underline" | "upperline" | "left" | "right"
                )
            })
            .unwrap_or(&d.border_type)
            .to_string(),
        padding: num(n.padding, d.padding),
    }
}

/// Card height for the surface math: 2×padding + five lines of font —
/// the default style gives exactly the F3 geometry (2×12 + 5×12 = 84),
/// so the stack stays click-exact for every style.
fn popup_card_h(style: &PopupStyle) -> u32 {
    2 * style.padding + 5 * style.font_size
}



/// Last monitor list with its timestamp: outputs change rarely (hotplug),
/// so the wallpaper sync — which asks every tick — gets a cached answer
/// for `MONITORS_TTL` instead of spawning and parsing `hyprctl` at 1 Hz.
pub(crate) static MONITORS: std::sync::Mutex<Option<(Instant, Vec<String>)>> = std::sync::Mutex::new(None);

/// Freshness window of [`MONITORS`]: long enough to make the per-tick
/// spawn rare, short enough that a hotplugged output gets its wallpaper
/// surface within seconds.
pub(crate) const MONITORS_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// Names of the currently active outputs, from `hyprctl monitors -j` —
/// best-effort, the same source the bar's geometry helpers use (cached,
/// see [`MONITORS`]); no answer = no surface for now (retried on the
/// next tick).
pub(crate) fn monitor_names() -> Vec<String> {
    if let Ok(cached) = MONITORS.lock() {
        if let Some((at, names)) = cached.as_ref() {
            if at.elapsed() < MONITORS_TTL && !names.is_empty() {
                return names.clone();
            }
        }
    }
    let Ok(output) = std::process::Command::new("hyprctl").args(["monitors", "-j"]).output() else {
        return Vec::new();
    };
    let names = monitor_names_json(&String::from_utf8_lossy(&output.stdout));
    // An empty answer is never cached: a transient hyprctl failure
    // retries on the next tick instead of freezing the list.
    if !names.is_empty() {
        if let Ok(mut cached) = MONITORS.lock() {
            *cached = Some((Instant::now(), names.clone()));
        }
    }
    names
}

/// `name` of every entry of the `hyprctl monitors -j` array; anything
/// unparseable yields no outputs instead of breaking the bar.
pub(crate) fn monitor_names_json(json: &str) -> Vec<String> {
    let Ok(monitors) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    monitors
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect()
}

/// The focused output — the bar's own, since Hyprland focuses the
/// monitor under the cursor and everything cursor-driven (graph popup,
/// tray menu, notification cards) belongs there — as `(name, x, y)`
/// from a fresh `hyprctl monitors -j`. `OutputOption::LastOutput`
/// picked another output once a second monitor joined (the popup
/// opened over on it); no answer falls back to the old behaviour.
pub(crate) fn focused_output() -> Option<(String, i32, i32)> {
    let output = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    focused_output_in(&String::from_utf8_lossy(&output.stdout))
}

/// The (`name`, `x`, `y`) of the `hyprctl monitors -j` entry with
/// `"focused": true`; unparseable input yields no output.
pub(crate) fn focused_output_in(json: &str) -> Option<(String, i32, i32)> {
    let monitors = serde_json::from_str::<Vec<serde_json::Value>>(json).ok()?;
    let m = monitors
        .iter()
        .find(|m| m.get("focused").and_then(|f| f.as_bool()) == Some(true))?;
    Some((
        m.get("name")?.as_str()?.to_string(),
        m.get("x")?.as_i64()? as i32,
        m.get("y")?.as_i64()? as i32,
    ))
}

/// The output the cursor-driven popup surfaces pin themselves to (the
/// bar's own, see [`focused_output`]); `LastOutput` as the fallback.
pub(crate) fn active_output_name() -> Option<String> {
    focused_output().map(|(name, _, _)| name)
}

/// The output for the graph popup and its connector: the bar's own
/// ([`active_output_name`]), falling back to the old `LastOutput`
/// behaviour when hyprctl can't answer.
pub(crate) fn popup_output_option() -> OutputOption {
    active_output_name()
        .map(OutputOption::OutputName)
        .unwrap_or(OutputOption::LastOutput)
}










// ── notification list window ────────────────────────────────────────




// ── tray menu window ────────────────────────────────────────────────







/// Both hover popups in step with one sensor hover: the trend popup
/// (its own sensor type) and the calendar one — each opens only for
/// its own source, so they share no condition beyond the hover.
fn hover_sync(panel: &mut Panel, section: usize, sensor: usize, hovered: bool) -> Command<Message> {
    Command::batch([
        graph_hover_sync(panel, section, sensor, hovered),
        calendar_hover_sync(panel, section, sensor, hovered),
    ])
}
















// ── notification list popup ─────────────────────────────────────────



// ── taskbar menu window ─────────────────────────────────────────────



// ── bar tooltip window ──────────────────────────────────────────────


// ── style ───────────────────────────────────────────────────────────

fn style(_panel: &Panel, _theme: &iced::Theme) -> iced::theme::Style {
    // Root stays transparent: the panel background is drawn by the inner
    // container, and a root background would force the surface to the whole
    // screen.
    iced::theme::Style {
        background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        text_color: _theme.palette().text,
    }
}

// ── main ────────────────────────────────────────────────────────────

fn main() -> Result<(), iced_exwlshell::Error> {
    // Parse CLI args for config path.
    let config_path = parse_config_arg();

    // Initialise logging.
    let _ = simple_logger::SimpleLogger::new()
        .env()
        .with_utc_timestamps()
        .init();

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
    let (config, resolved_config_path) = config::load_config(config_path.clone());

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

    // Roadmap N3: render the GTK theme bridge from the bar's own style
    // (writes/removes only piuma's own file; off = untouched).
    // The palette arrives with PaletteReady below, which resyncs.
    app_theme::sync(&resolved_config_path, &config.panel, None);

    // Best-effort primary monitor dimensions for screen-wide surface sizing.
    unsafe {
        SCREEN_WIDTH = active_screen_width().unwrap_or(1920);
        SCREEN_HEIGHT = active_screen_height().unwrap_or(1080);
    }

    let _ = CONFIG.set(config.clone());
    let _ = CONFIG_PATH.set(resolved_config_path.to_string_lossy().into_owned());

    let panel_config = CONFIG.get().cloned().unwrap_or_else(|| config::default_config());

    // Run the iced app on a dedicated thread so the close action can be
    // observed from the main thread without blocking the event loop.
    let handle = std::thread::Builder::new()
        .name("piuma-panel".to_string())
        .spawn(move || {
            let outcome = daemon(
                || {
                    let cfg = CONFIG
                        .get()
                        .cloned()
                        .unwrap_or_else(|| config::default_config());
                    // Initial geometry (size, anchor, exclusive zone) must
                    // match what wl_settings applies at startup, so reloads
                    // only emit LayoutChange/ExclusiveZoneChange on change.
                    let size = panel_size(&cfg, unsafe { SCREEN_WIDTH }, unsafe { SCREEN_HEIGHT });
                    let anchor = panel_anchor(&cfg);
                    let zone = panel_thickness(&cfg) as i32;
                    let graphs = graphs::History::new(&cfg.panel.sections);
                    // Colour chains resolve once, off the tick (see
                    // `restyle`): the palette starts absent, so the
                    // configured defaults lead until one arrives.
                    let styles = resolve_styles(&cfg, None);
                    Panel {
                        config: cfg,
                        sections: Vec::new(),
                        styles,
                        read_pending: false,
                        cfg_gen: 0,
                        popup_resync: false,
                        hovered: None,
                        last_reload: config_file_state(),
                        last_size: size,
                        last_anchor: anchor,
                        last_exclusive_zone: zone,
                        last_cursor: None,
                        popup: None,
                        popup_cards: 0,
                        popup_hover: None,
                        tray_menu: None,
                        tray_menu_hover: None,
                        tray_menu_gen: 0,
                        notif_list: None,
                        taskbar_menu: None,
                        taskbar_menu_hover: None,
                        taskbar_hover: None,
                        bar_tooltip: None,
                        graphs,
                        graph_popup: None,
                        graph_link: None,
                        calendar: None,
                        wallpaper: Vec::new(),
                        palette: None,
                        palette_key: None,
                    }
                },
                namespace,
                update,
                view,
            )
            .subscription(subscription)
            .style(style)
            .wl_settings(ExWlSettings {
                layer_settings: LayerShellSettings {
                    size: panel_size(&panel_config, unsafe { SCREEN_WIDTH }, unsafe { SCREEN_HEIGHT }),
                    // Reserve exactly the configured thickness: a constant
                    // here would leave dead space whenever thickness != 32.
                    exclusive_zone: panel_thickness(&panel_config) as i32,
                    anchor: panel_anchor(&panel_config),
                    layer: Layer::Top,
                    start_mode: StartMode::Active,
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

    // Detach the iced thread so the process exit terminates it.
    std::mem::forget(handle);
    Ok(())
}

/// Best-effort primary monitor width, via `hyprctl`.
fn active_screen_width() -> Option<u32> {
    let output = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Take the first monitor's width field.
    let key = "\"width\"";
    let mut parts = text.rsplitn(2, key);
    let after = parts.next()?;
    let num_part = after.split(':').next()?;
    num_part.trim().trim_matches(|c: char| c == ',' || c.is_whitespace()).parse::<u32>().ok()
}

/// Best-effort primary monitor height, via `hyprctl`.
fn active_screen_height() -> Option<u32> {
    let output = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Take the first monitor's height field.
    let key = "\"height\"";
    let mut parts = text.rsplitn(2, key);
    let after = parts.next()?;
    let num_part = after.split(':').next()?;
    num_part.trim().trim_matches(|c: char| c == ',' || c.is_whitespace()).parse::<u32>().ok()
}

/// Parse `-c <path>` or `--config <path>` from CLI args.
fn parse_config_arg() -> Option<String> {
    let args: Vec<String> = std::env::args().map(|s| s.to_string()).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-c" || arg == "--config" {
            if i + 1 < args.len() {
                return Some(args[i + 1].clone());
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        config, focused_output_in, icons_theme, monitor_names_json, palette, popup_card_h,
        popup_style, PopupStyle,
    };

    /// The default popup style must keep the F3 geometry exactly (84px
    /// cards): the surface size is computed from it, and any drift would
    /// leave a dead (click-blocking) strip below the stack.
    #[test]
    fn popup_card_h_defaults_to_84_and_scales_with_style() {
        assert_eq!(popup_card_h(&PopupStyle::default()), 84);
        let styled = PopupStyle {
            font_size: 16,
            padding: 8,
            ..PopupStyle::default()
        };
        assert_eq!(popup_card_h(&styled), 96);
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


}
