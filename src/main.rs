//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.
//!
//! Configuration is loaded from a JSON file (CLI arg or default path).
//! See `src/config.rs` for the config schema.

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use iced::widget::{container, text, Column, Row, MouseArea, Space, Stack};
use iced::widget::image::Image;
use iced::{
    Alignment, Background, Color, Element, Event, Length, Rectangle, Size,
    Subscription, Task as Command, Vector,
};
use iced::event;
use iced::time;
use iced::font::Font;
use iced::widget::container::Style as ContainerStyle;
use iced::advanced::layout::{Limits, Node};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Layout, Shell};

use iced_exwlshell::ExWlSettings;
use iced_exwlshell::daemon;
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, Margin, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::{LayerShellSettings, LayerSize, StartMode};
use iced_exwlshell::to_layer_message;
use iced_runtime::core::widget::operation::Outcome;

mod app_theme;
mod autostart;
mod calendar;
mod config;
mod dbusmenu;
mod graphs;
mod icons;
mod notifications;
mod palette;
mod sensor_runner;
mod sensors;
mod text_width;
mod tray;
mod windows;
mod workspace;
// Layout padding lives with the icon sizing helpers (icons.rs): the view
// and the row-height size formula must read the same number.
use icons::SECTION_CONTENT_PAD;

// ── globals ────────────────────────────────────────────────────────

/// Startup config, handed to the first window factory call and to any
/// later surface: written once before the threads start, read from any
/// thread afterwards (`OnceLock` keeps that sound without `unsafe`).
static CONFIG: std::sync::OnceLock<config::Config> = std::sync::OnceLock::new();
/// The config file watched for hot reloads (string for the log lines);
/// also written once at startup.
static CONFIG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static mut PANEL_SHOULD_CLOSE: bool = false;
static mut SCREEN_WIDTH: u32 = 1920;
static mut SCREEN_HEIGHT: u32 = 1080;

// ── colours / defaults ────────────────────────────────────────────

const DEFAULT_FONT: &str = "Noto Sans";
const DEFAULT_FONT_SIZE: u32 = 14;
const DEFAULT_BG_COLOR: Color = Color::from_rgba(0.06, 0.06, 0.08, 0.85);
const DEFAULT_TEXT_COLOR: Color = Color::from_rgb(0.88, 0.88, 0.94);

/// Internal action identifiers.
const ACTION_CLOSE_PANEL: &str = "close_panel";

// ── popup card geometry (F3) ────────────────────────────────────────
// The popup surface is sized to the stack exactly (cards × height +
// gaps): the region below the stack simply is not surface, so clicks
// there reach the window underneath with no input-region handling.
// The card height is style-derived (`popup_card_h`); with the default
// style (12px font, 12px padding) it is exactly 84px.
const POPUP_MAX_CARDS: usize = 5;
const POPUP_WIDTH: u32 = 360;
const POPUP_CARD_GAP: u32 = 8;
/// Gap between the screen edges and the popup corner.
const POPUP_MARGIN: i32 = 8;

// ── sensor graph popup geometry (roadmap2 #11) ──────────────────────
// One popup at a time, on the sensor under the cursor: fixed size, the
// surface ends where the box ends (no input-region games — the layout
// below knows nothing of it, the popup is its own window).
const GRAPH_POPUP_W: u32 = 260;
const GRAPH_POPUP_H: u32 = 140;
/// Padding around the connector's bounding box: its surface is the
/// box plus this on every side (the end dots must not clip).
const GRAPH_LINK_PAD: i32 = 4;
/// Content geometry inside the popup: a fixed header box and gap, so
/// the average label and the canvas share one exact coordinate system
/// (the label sits on the guide line the draw pass computes).
const GRAPH_HEADER_H: u32 = 20;
const GRAPH_SPACING: u32 = 6;
/// Grace ticks an open graph popup gets once its sensor and its
/// surface both lost the cursor (≈2 s at the 1 Hz tick): the sensor's
/// leave can act on stale coordinates and the widget's exit can be
/// missed, so the reap is the backstop that cannot hang.
const GRAPH_ABANDON_GRACE: u8 = 2;
/// Alpha of the tinted plate behind the average label: opaque enough
/// to hold the text where it crosses the trend line, translucent
/// enough to keep the guide visible through it.
const LABEL_BG_ALPHA: f32 = 0.7;
/// The plate's padding around the average label (v, h).
const LABEL_PAD_V: f32 = 1.0;
const LABEL_PAD_H: f32 = 4.0;

// ── calendar popup geometry (the `calendar` sensor) ────────────────
/// Grace ticks the calendar popup gets after the cursor has left both
/// its sensor and its surface (1 tick = 1 s at the 1 Hz tick): the
/// spec asks for three seconds before it disappears.
const CALENDAR_ABANDON_GRACE: u8 = 3;
/// Horizontal padding inside one day/label cell, either side.
const CALENDAR_CELL_PAD: u32 = 6;

// ── tray menu geometry ──────────────────────────────────────────────
// One row per entry, sized exactly (`menu_row_h` × count + 2× pad, see
// `menu_size`): like the popup, the surface ends where the list ends,
// so a stray click below it reaches the window underneath.
/// Padding around the entry list (all four sides).
const MENU_PAD: u32 = 6;
/// Distance between a tooltip and the widget it describes (icons and
/// tray items): enough to read it as attached, never on the widget.
const TOOLTIP_GAP: f32 = 8.0;
/// Indent per submenu level (rows and the width estimate agree on it).
const MENU_INDENT: u32 = 14;
/// Width bounds: a one-item menu stays grabbable, a long label does
/// not take half the screen (it gets clipped instead).
const MENU_MIN_W: u32 = 120;
const MENU_MAX_W: u32 = 420;

/// The notification list popup (the sensor's click): entries in the
/// snapshot, rows shown at once (the wheel scrolls the rest) and the
/// width cap for the measured rows.
const NOTIF_LIST_MAX: usize = 30;
const NOTIF_LIST_ROWS: usize = 8;
const NOTIF_LIST_W_MAX: u32 = 460;

// ── model ───────────────────────────────────────────────────────────

#[derive(Clone)]
struct SensorValue {
    name: String,
    description: String,
    value: String,
    text_color: Option<iced::Color>,
    /// Threshold colour rules (config `color_rules`) parsed to colours
    /// and sorted ascending by `min`: the view tints the value with
    /// the highest rule the displayed reading reaches.
    color_rules: Vec<(f32, Color)>,
    bg_color: Option<Color>,
    text_color_hover: Option<iced::Color>,
    bg_color_hover: Option<Color>,
    border_color: Option<Color>,
    border_width: u16,
    border_radius: u16,
    /// Border drawing style: `full`, `corners`, `underline`, `upperline`,
    /// `left` or `right`.
    border_type: String,
    padding: u16,
    action: Option<String>,
    /// Tray icons when the source is a `tray` sensor (`None` otherwise);
    /// the icons themselves live in `tray.rs`.
    tray: Option<Vec<tray::TrayIcon>>,
    /// Workspace buttons when the source is a `workspace` sensor (`None`
    /// otherwise), built by `workspace::buttons()` (roadmap2 #4).
    workspaces: Option<Vec<workspace::WorkspaceButton>>,
    /// Taskbar slots when the source is a `taskbar` sensor (`None`
    /// otherwise), one per open window, built per tick by
    /// `windows::task_icons()`.
    taskbar: Option<Vec<windows::TaskIcon>>,
    /// Pinned pixel width of the value text (roadmap M5): net rate sensors
    /// keep a stable box while the value moves inside its unit, and the pin
    /// follows the value across ×1024 unit crossings (same tick), so it is
    /// never narrower than the content.
    fixed_width: Option<u16>,
    /// Resolved theme file for the sensor's `icon` name (roadmap N1);
    /// `None` = no icon configured or the theme has no such icon.
    icon: Option<PathBuf>,
    /// Edge (px) of `icon`: the sensor row's real height
    /// (`icons::row_icon_target`), so the icon fills its box like the
    /// tray's do. The resolve caches by this size and the widget draws
    /// at it — both must stay the same number.
    icon_size: u16,
}

/// Rendered section data.
#[derive(Clone)]
struct SectionData {
    name: String,
    layout: String,       // "horizontal" or "vertical"
    bg_color: Color,
    text_color: Color,
    title_color: Color,
    /// Resolved border (section > section_defaults): drawn by `styled_box`
    /// with the same rule as the sensors (color + positive width).
    border_color: Option<Color>,
    border_width: u16,
    border_radius: u16,
    border_type: String,
    /// Which sides `[top, right, bottom, left]` sit flush on a screen edge
    /// with `section_defaults.respect_screen_edges` active (roadmap M3):
    /// those sides render square instead of rounded.
    touch: [bool; 4],
    sensors: Vec<SensorValue>,
}

#[derive(Clone)]
struct Panel {
    config: config::Config,
    sections: Vec<SectionData>,
    /// Index into `sections[s]` and `sections[s].sensors[sensor]` under the cursor.
    hovered: Option<(usize, usize)>,
    /// State (mtime, len) of the last config read attempt: unchanged files
    /// are skipped on the next tick instead of being re-read and parsed.
    last_reload: Option<(SystemTime, u64)>,
    /// Geometry currently applied to the layer surface; compared after a
    /// reload to decide whether a runtime `LayoutChange` is needed.
    last_size: LayerSize,
    last_anchor: Anchor,
    /// Exclusive zone currently applied (equals the panel thickness, the
    /// space hyprland subtracts for windows); re-sent when it changes.
    last_exclusive_zone: i32,
    /// Last known cursor position, window-relative: feeds the screen-coordinate
    /// hints of tray activation (see `tray::hint_point`).
    last_cursor: Option<(f32, f32)>,
    /// Popup stack window (F3): its id while open, `None` while hidden.
    popup: Option<iced::window::Id>,
    /// Card count the popup surface is currently sized for.
    popup_cards: usize,
    /// Card under the cursor (`hover_*` style fields), `None` outside.
    popup_hover: Option<u32>,
    /// Open tray DBusMenu window: id + the menu it renders, `None`
    /// while hidden (see `dbusmenu`).
    tray_menu: Option<(iced::window::Id, TrayMenu)>,
    /// Menu row under the cursor (`TrayMenuEnter`/`TrayMenuLeave`),
    /// `None` outside every row.
    tray_menu_hover: Option<i32>,
    /// Bumped by every tray press: a `TrayMenuLoaded` carrying an older
    /// generation belongs to a superseded press and must not open.
    tray_menu_gen: u64,
    /// The notification history list popup (the notifications sensor's
    /// click): window id + the wheel scroll state.
    notif_list: Option<(iced::window::Id, NotifList)>,
    /// The taskbar's right-click menu: window id + the window it acts
    /// on.
    taskbar_menu: Option<(iced::window::Id, TaskbarMenu)>,
    /// Workspace row under the cursor in the taskbar menu
    /// (`TaskbarMenuHover`), `None` outside every row.
    taskbar_menu_hover: Option<u32>,
    /// Address of the taskbar icon under the cursor (`None` = none):
    /// the row's hover fill.
    taskbar_hover: Option<String>,
    /// The tooltip window (taskbar or tray hover): window id + source
    /// and fitted text.
    bar_tooltip: Option<(iced::window::Id, BarTooltip)>,
    /// Trend history for the `draw_graph` sensors' hover popup: one
    /// dense ring of samples per config, rebuilt on reload (see
    /// `graphs`).
    graphs: graphs::History,
    /// Open graph popup: its surface id, the sensor it draws and the
    /// output-space rect the hover guards compare the cursor with,
    /// `None` while hidden.
    graph_popup: Option<GraphPopup>,
    /// The connector surface joining the graph popup to its sensor (see
    /// `sync_graph_link`): its id and the id of the popup it draws —
    /// `None` while hidden.
    graph_link: Option<(iced::window::Id, iced::window::Id)>,
    /// Open calendar popup of a hovered `calendar` sensor: surface id,
    /// the sensor it belongs to, the shown month and its pointer/grace
    /// state, `None` while hidden.
    calendar: Option<CalendarPopup>,
    /// Wallpaper windows (roadmap2 #1): one background surface per
    /// active output, keyed by the compositor's output name; empty
    /// while the wallpaper is off.
    wallpaper: Vec<(String, iced::window::Id)>,
    /// Colours derived from the wallpaper (`panel.wallpaper_palette`):
    /// applied over the configured defaults while present, dropped when
    /// the option or the wallpaper turns off. Runtime-only — never
    /// written back to the config file.
    palette: Option<palette::Palette>,
    /// The wallpaper file (path + mtime) the current palette was
    /// extracted from: the extraction re-runs only when this changes.
    palette_key: Option<(String, SystemTime)>,
}

/// One open tray menu: which item it belongs to (the item's bus + path
/// tell us when it dies), the menu object path events go to, and the
/// flattened rows the view renders.
#[derive(Clone)]
struct TrayMenu {
    bus: String,
    path: String,
    menu_path: String,
    entries: Vec<dbusmenu::MenuEntry>,
}

/// The open notification list popup (the notifications sensor's click):
/// its window and the wheel scroll offset (whole rows from the newest).
#[derive(Clone, Debug)]
struct NotifList {
    offset: u32,
}

/// The taskbar's right-click menu: which window the workspace rows
/// move (the address, never an index — a tick may rebuild the icons
/// while the menu is open).
#[derive(Clone, Debug)]
struct TaskbarMenu {
    address: String,
}

/// The open bar tooltip window: its source (so a stale leave knows
/// whether it still owns the surface) and the fitted text shown.
#[derive(Clone, Debug)]
struct BarTooltip {
    id: String,
    text: String,
}

/// The open sensor-graph popup: which sensor's trend it draws and the
/// output-space rect `(x0, y0, x1, y1)` it occupies (placement of the
/// moment, for the hover guards). `pointer_on_popup`/`missed` feed the
/// tick's abandonment reap (see `popup_missed_tick`).
#[derive(Clone)]
struct GraphPopup {
    id: iced::window::Id,
    section: usize,
    sensor: usize,
    rect: (i32, i32, i32, i32),
    /// Which output edge the popup anchors to: an anchored surface sits
    /// past the bar's reserved strip, so the real surface rect derives
    /// from `rect` + the anchor (see `popup_surface_rect`).
    anchor: Anchor,
    /// The sensor box's on-screen rect in output space, as answered by
    /// `FindSensorRect`: the connector window is pinned to it.
    sensor_rect: Option<(i32, i32, i32, i32)>,
    /// The cursor is on the popup's surface — set and cleared by the
    /// surface's own cursor events (position-checked both ways, plus
    /// the window's `CursorLeft`) and by the widget's exit. Never
    /// one-way: a single "seen" setter with only the widget's exit as
    /// a reset pins the popup open forever when that exit never fires
    /// (the very bug `31771e1` fixed on the calendar popup).
    pointer_on_popup: bool,
    /// Grace ticks spent without sensor and without pointer.
    missed: u8,
}

/// The calendar popup: which sensor's hover opened it, the month
/// paging offset and the localized data fetched at open — plus the
/// pointer/grace bookkeeping that decides when it disappears.
#[derive(Clone)]
struct CalendarPopup {
    id: iced::window::Id,
    section: usize,
    sensor: usize,
    /// Whole months between today's month and the shown one: the
    /// arrows page it, a close drops it (a reopen starts at today).
    offset: i32,
    /// Today (local): the highlight and the month the offset starts at.
    today: (u16, u8, u8),
    /// Monday-first weekday labels, fetched once at open.
    weekdays: [String; 7],
    /// The header's "month year", re-fetched on every page change.
    month_label: String,
    /// The cursor is on the popup's surface — set and cleared by the
    /// surface's own cursor events (position-checked both ways, plus
    /// the window's `CursorLeft`) and by the open-time test against
    /// `rect`. Never one-way: a single "seen" setter with only the
    /// widget's exit as a reset pins the popup open forever when that
    /// exit never fires (its enter can be missed: the surface maps
    /// under an already-moving cursor before the widget state exists).
    pointer_on_popup: bool,
    /// The surface's real output-space rect (the placement shifted by
    /// the anchors, `shifted_surface_rect`): the pointer checks compare
    /// against where the surface really is — a top bar pushes it down
    /// by the exclusive zone, so the intended rect contains the cursor
    /// while the surface does not.
    rect: (i32, i32, i32, i32),
    /// Grace ticks spent without sensor and without pointer.
    missed: u8,
}

// ── messages ────────────────────────────────────────────────────────

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    /// Marks a second edge: the tick handler reloads, polls and advances.
    Tick,
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
struct FindSensorRect {
    section: usize,
    sensor: usize,
    target: iced::widget::Id,
    found: Option<(i32, i32, i32, i32)>,
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

fn resolve_color<'a>(cfg: Option<Color>, default: Color) -> Color {
    cfg.unwrap_or(default)
}

/// The panel's background: the wallpaper palette's base tone when it's
/// active — keeping the configured colour's TRANSPARENCY (a `#000000ff`
/// bar renders opaque in the palette's hue, a `#00000000` one stays
/// see-through) — else the configured `default_bg_color`, else the
/// built-in default. Used by both the surface's container and the
/// sections' fallback, so they can't disagree.
fn panel_background(config: &config::Config, pal: Option<&palette::Palette>) -> Color {
    let configured = config
        .panel
        .default_bg_color
        .as_ref()
        .and_then(|s| config::parse_color(s).ok());
    pal.and_then(|p| config::parse_color(&p.bg).ok())
        .map(|mut c| {
            if let Some(b) = configured {
                c.a = b.a;
            }
            c
        })
        .or(configured)
        .unwrap_or(DEFAULT_BG_COLOR)
}

/// A solid bar widget used to compose partial sensor borders.
fn border_bar(color: Color, width: Length, height: Length) -> Element<'static, Message> {
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
                // The graph history follows the new sensor set: one
                // ring per `draw_graph` column of the new config.
                panel.graphs = graphs::History::new(&panel.config.panel.sections);
                // Roadmap N3: keep the generated GTK bridge in step with
                // hot reloads (rewritten only when its content changes).
                app_theme::sync(std::path::Path::new(&path), &panel.config.panel);
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

            // Roadmap N1: with no icon-declaring sensor the whole icon path
            // stays dormant — `icons_theme` is `None`, so no theme lookup
            // (it spawns `gsettings`), no file resolution, no cache entries.
            let icon_theme = icons_theme(&panel.config);

            // Wallpaper palette (`panel.wallpaper_palette`): runtime-only
            // tones applied over the configured defaults below — the
            // config file never sees them (sync_palette keeps or drops
            // the palette in memory).
            let pal = panel.palette.as_ref();

            // Samples of this tick for the graph history (one read per
            // sensor, collected here so the closure and the history
            // never borrow the panel at once).
            let mut graph_samples: Vec<((usize, usize), f64)> = Vec::new();
            let mut sections: Vec<SectionData> = Vec::new();
            for (section_idx, section_cfg) in panel.config.panel.sections.iter().enumerate() {
                let section_default_txt = pal
                    .and_then(|p| config::parse_color(&p.text).ok())
                    .or_else(|| {
                        panel
                            .config
                            .panel
                            .default_text_color
                            .as_ref()
                            .and_then(|s| config::parse_color(s).ok())
                    })
                    .unwrap_or(DEFAULT_TEXT_COLOR);

                // Text colour chain: section > wallpaper palette (when
                // active) > section_defaults > panel > built-in. The
                // palette keeps the configured transparency, like bg.
                let section_text_color = section_cfg.text_color.as_ref()
                    .and_then(|s| config::parse_color(s).ok())
                    .or_else(|| {
                        let configured = panel
                            .config
                            .panel
                            .section_defaults
                            .as_ref()
                            .and_then(|sd| sd.text_color.as_ref())
                            .and_then(|s| config::parse_color(s).ok());
                        pal.and_then(|p| config::parse_color(&p.text).ok())
                            .map(|mut t| {
                                if let Some(c) = configured {
                                    t.a = c.a;
                                }
                                t
                            })
                    })
                    .or_else(|| {
                        panel.config.panel.section_defaults.as_ref()
                            .and_then(|sd| sd.text_color.as_ref())
                            .and_then(|s| config::parse_color(s).ok())
                    })
                    .unwrap_or(section_default_txt);

                let sensors: Vec<SensorValue> = section_cfg
                    .sensors
                    .iter()
                    .enumerate()
                    .map(|(sensor_idx, s)| {
                        let read = sensor_runner::run(&s.source);
                        // Graph engine: record this tick's sample for the
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
                        // hyprctl (roadmap2 #4): one entry per available
                        // workspace, the active one flagged.
                        let workspace_buttons =
                            matches!(s.source, config::SensorSource::Workspace {})
                                .then(workspace::buttons);
                        // Taskbar sensors rebuild their icon slots from
                        // hyprctl per tick (one fork, like every polling
                        // sensor); the icons resolve through the memoized
                        // theme lookups.
                        let taskbar_icons =
                            matches!(s.source, config::SensorSource::Taskbar {})
                                .then(windows::task_icons);
                        // Style precedence, per style GROUP: a sensor takes a
                        // group over only when it defines the group's key
                        // colour (`border_color`, `bg_color`, `text_color`).
                        // When that key is null on the sensor, every override
                        // of that group on the sensor is ignored and
                        // `panel.sensor_defaults` applies as a whole. When the
                        // key is set, the sensor's own fields win and the
                        // remaining ones fall back to the defaults.
                        // `padding` is a single value: sensor when set,
                        // otherwise the default. It also feeds the icon fit
                        // below, so it is resolved before the icon.
                        let defaults = panel.config.panel.sensor_defaults.as_ref();
                        let padding = config::style_num(s.padding)
                            .or(defaults.and_then(|sd| config::style_num(sd.padding)))
                            .unwrap_or(0) as u16;
                        // Roadmap N1: theme file for the sensor's icon, sized
                        // to this sensor row's real height — the fit the tray
                        // already uses (thickness minus the layout chrome,
                        // clamped), so the icon fills its box instead of
                        // trailing the font size. The size travels with the
                        // path (resolve and widget must agree on it); the
                        // lookup is memoized, so the per-second rebuild only
                        // clones the cached path. With `icon_theme == None`
                        // (gate) this arm never runs.
                        let icon_size = icons::row_icon_target(
                            panel_thickness(&panel.config),
                            icons::stacked_title_extra(
                                section_cfg.layout.as_deref().unwrap_or("horizontal"),
                                &section_cfg.name,
                                panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE),
                            ),
                            u32::from(padding),
                        );
                        let icon = match (icon_theme, s.icon.as_deref()) {
                            (Some(theme), Some(icon_name)) => {
                                icons::resolve(theme, icon_name, icon_size)
                            }
                            _ => None,
                        };
                        let own_border = s.border_color.is_some();
                        let own_bg = s.bg_color.is_some();
                        let own_text = s.text_color.is_some();
                        // Key fields: per-sensor, then the wallpaper
                        // palette (when active), then defaults, then
                        // section/built-in. The palette overrides the
                        // configured defaults only — an explicit
                        // per-sensor value always wins — and the
                        // configured TRANSPARENCY survives it (a
                        // `#..80` default keeps its alpha in the
                        // wallpaper's hue).
                        let tc_pal = pal.map(|p| {
                            palette::keep_config_alpha(
                                &p.text,
                                defaults.and_then(|sd| sd.text_color.as_deref()),
                            )
                        });
                        let tc_str = s.text_color.as_deref()
                            .or(tc_pal.as_deref())
                            .or(defaults.and_then(|sd| sd.text_color.as_deref()));
                        let bg_pal = pal.map(|p| {
                            palette::keep_config_alpha(
                                &p.card,
                                defaults.and_then(|sd| sd.bg_color.as_deref()),
                            )
                        });
                        let bg_str = s.bg_color.as_deref()
                            .or(bg_pal.as_deref())
                            .or(defaults.and_then(|sd| sd.bg_color.as_deref()));
                        let bc_str = s.border_color.as_deref().or(defaults.and_then(|sd| sd.border_color.as_deref()));
                        // Secondary fields only count when the sensor owns the group.
                        let bg_h_pal = pal.map(|p| {
                            palette::keep_config_alpha(
                                &p.card_hover,
                                defaults.and_then(|sd| sd.bg_color_hover.as_deref()),
                            )
                        });
                        let bg_h_str = (if own_bg { s.bg_color_hover.as_deref() } else { None })
                            .or(bg_h_pal.as_deref())
                            .or(defaults.and_then(|sd| sd.bg_color_hover.as_deref()));
                        let tc_h_pal = pal.map(|p| {
                            palette::keep_config_alpha(
                                &p.text_hover,
                                defaults.and_then(|sd| sd.text_color_hover.as_deref()),
                            )
                        });
                        let tc_h_str = (if own_text { s.text_color_hover.as_deref() } else { None })
                            .or(tc_h_pal.as_deref())
                            .or(defaults.and_then(|sd| sd.text_color_hover.as_deref()));
                        // Numeric style fields follow the -1 = disabled rule:
                        // only an enabled value (>= 0) overrides the defaults.
                        let bw_val = (if own_border { config::style_num(s.border_width) } else { None })
                            .or(defaults.and_then(|sd| config::style_num(sd.border_width)))
                            .unwrap_or(0);
                        let br_val = (if own_border { config::style_num(s.border_radius) } else { None })
                            .or(defaults.and_then(|sd| config::style_num(sd.border_radius)))
                            .unwrap_or(0);
                        let bt_str = (if own_border { s.border_type.as_deref() } else { None })
                            .or(defaults.and_then(|sd| sd.border_type.as_deref()))
                            .unwrap_or("full");

                        let sensor_tc = tc_str
                            .and_then(|sc| config::parse_color(sc).ok())
                            .unwrap_or(section_text_color);
                        let sensor_bg: Option<Color> = bg_str.and_then(|sc| config::parse_color(sc).ok());
                        let sensor_bg_hover: Option<Color> = bg_h_str.and_then(|sc| config::parse_color(sc).ok());
                        let sensor_tc_hover: Option<iced::Color> = tc_h_str.and_then(|sc| config::parse_color(sc).ok());
                        let sensor_border: Option<Color> = bc_str.and_then(|sc| config::parse_color(sc).ok());
                        // Threshold colour rules, colours resolved once
                        // per rebuild and sorted ascending: the view
                        // picks the highest rule the reading reaches.
                        let mut color_rules: Vec<(f32, Color)> = s
                            .color_rules
                            .as_deref()
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|r| {
                                config::parse_color(&r.color)
                                    .ok()
                                    .map(|c| (r.min as f32, c))
                            })
                            .collect();
                        color_rules.sort_by(|a, b| a.0.total_cmp(&b.0));
                        // Only the net rate sensors pin their value box
                        // (roadmap M5), and the pin is the displayed value's
                        // OWN tier: exact width (no headroom for units this
                        // state never shows), constant while the value moves
                        // inside the tier (figure spaces), moving with it on
                        // a ×1024 crossing — same tick, same string, so the
                        // box can never undershoot its content. Computed
                        // before the literal because `value` is moved into
                        // it; the measurement is cached per
                        // font/size/patterns, so the per-tick rebuild only
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
                            // static_text and tray sensors render only their
                            // content: the name stays in the config (setup
                            // keeps editing it) but never appears on the bar.
                            name: if matches!(
                                s.source,
                                config::SensorSource::StaticText { .. }
                                    | config::SensorSource::Tray {}
                                    // The buttons carry the names already.
                                    | config::SensorSource::Workspace {}
                            ) {
                                String::new()
                            } else {
                                s.name.clone()
                            },
                            description: s.description.as_ref().map(|d| d.to_string()).unwrap_or_default(),
                            value,
                            text_color: Some(sensor_tc),
                            color_rules,
                            bg_color: sensor_bg,
                            text_color_hover: sensor_tc_hover,
                            bg_color_hover: sensor_bg_hover,
                            border_color: sensor_border,
                            border_width: bw_val as u16,
                            border_radius: br_val as u16,
                            border_type: bt_str.to_string(),
                            padding,
                            action: s.action.clone(),
                            tray: tray_icons,
                            workspaces: workspace_buttons,
                            taskbar: taskbar_icons,
                            icon,
                            icon_size,
                            fixed_width,
                        }
                    })
                    .collect();

                let default_bg = panel_background(&panel.config, pal);

                // Section style precedence, per style GROUP (the same rule as
                // `panel.sensor_defaults`): the border group belongs to the
                // section only when it sets `border_color`; otherwise the
                // whole group comes from `panel.section_defaults`. bg/text
                // are single keys: section value when set, then the active
                // wallpaper palette, else the default.
                let sdefaults = panel.config.panel.section_defaults.as_ref();
                let own_border = section_cfg.border_color.is_some();
                let bg_pal = pal.map(|p| {
                    palette::keep_config_alpha(
                        &p.card,
                        sdefaults.and_then(|sd| sd.bg_color.as_deref()),
                    )
                });
                let bg_str = section_cfg.bg_color.as_deref()
                    .or(bg_pal.as_deref())
                    .or(sdefaults.and_then(|sd| sd.bg_color.as_deref()));
                let bc_str = section_cfg.border_color.as_deref()
                    .or(sdefaults.and_then(|sd| sd.border_color.as_deref()));
                // Numeric fields follow the -1 = disabled rule: only an
                // enabled value (>= 0) overrides the defaults.
                let bw_val = (if own_border { config::style_num(section_cfg.border_width) } else { None })
                    .or(sdefaults.and_then(|sd| config::style_num(sd.border_width)))
                    .unwrap_or(0);
                let br_val = (if own_border { config::style_num(section_cfg.border_radius) } else { None })
                    .or(sdefaults.and_then(|sd| config::style_num(sd.border_radius)))
                    .unwrap_or(0);
                let bt_str = (if own_border { section_cfg.border_type.as_deref() } else { None })
                    .or(sdefaults.and_then(|sd| sd.border_type.as_deref()))
                    .unwrap_or("full");

                sections.push(SectionData {
                    name: section_cfg.name.clone(),
                    layout: section_cfg.layout.as_ref().map(|s| s.to_string()).unwrap_or("horizontal".to_string()),
                    bg_color: resolve_color(
                        bg_str.and_then(|s| config::parse_color(s).ok()),
                        default_bg,
                    ),
                    text_color: section_text_color,
                    // Title = section text colour at 80% alpha (same chain).
                    title_color: section_text_color.scale_alpha(0.8),
                    border_color: bc_str.and_then(|s| config::parse_color(s).ok()),
                    border_width: bw_val as u16,
                    border_radius: br_val as u16,
                    border_type: bt_str.to_string(),
                    // Roadmap M3: the edge rule is a layout hint coming from
                    // the defaults only (a section's own border group
                    // doesn't carry it).
                    touch: if sdefaults
                        .and_then(|sd| sd.respect_screen_edges)
                        .unwrap_or(false)
                    {
                        section_touch(
                            &panel.config,
                            section_idx,
                            panel.config.panel.sections.len(),
                        )
                    } else {
                        [false; 4]
                    },
                    sensors,
                });
            }
            panel.sections = sections;
            // One ring advance per tick: columns repeat their last value
            // when a reading was missing (time stays uniform).
            panel.graphs.record(&graph_samples);
            // Icons size to the space their sensor box really has; both the
            // view and the icon resolution read tray::icon_target.
            update_tray_icon_target(panel);
            let mut cmds: Vec<Command<Message>> = Vec::new();
            // A popup that lost both its sensor and its pointer gets
            // reaped once its grace runs out: the sensor's leave can act
            // on stale coordinates and, after the cursor rests on another
            // application's window, no event ever arrives again.
            if let Some(gp) = reap_abandoned_popup(panel) {
                cmds.push(iced::window::close(gp.id));
            }
            // The calendar popup's three-second grace runs on the same
            // tick: neither sensor nor surface under the cursor spends it.
            if let Some(cp) = reap_abandoned_calendar(panel) {
                cmds.push(iced::window::close(cp.id));
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
            Command::batch(cmds)
        }
        Message::PaletteReady(pal, key) => {
            // The next view applies the tones — or resolves the user's
            // own colours again when the extraction failed (`None`).
            panel.palette = pal;
            panel.palette_key = Some(key);
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
            cmds.push(update(panel, Message::Tick));
            Command::batch(cmds)
        }
        Message::NotificationsChanged => {
            // A notification arrived, was replaced, expired or closed:
            // rebuild at once (same full path as the tray poke).
            update(panel, Message::Tick)
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
fn icons_theme(config: &config::Config) -> Option<&str> {
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
static BAR_ID: std::sync::OnceLock<iced::window::Id> = std::sync::OnceLock::new();

/// Configured family names kept alive for [`Font::Name`]'s `'static`
/// bound: a name is leaked the first time it is used and the same
/// pointer is reused after, so reloads — even alternating between two
/// fonts — never leak a fresh copy per change.
static FONT_NAMES: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());

/// The `'static` family name of `name` (roadmap M2): one leak per
/// distinct family, so `Font::with_name` can take it repeatedly.
fn family_name_static(name: &str) -> &'static str {
    let mut names = FONT_NAMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((leaked, _)) = names.iter().find(|(_, original)| original == name) {
        return *leaked;
    }
    let leaked = Box::leak(name.to_string().into_boxed_str());
    names.push((leaked, name.to_string()));
    leaked
}

/// The panel font exactly as configured: family from `panel.font` (default
/// [`DEFAULT_FONT`]) and the `panel.bold` flag as the weight (roadmap M2).
/// Single source for every bar text and for the M5 width measurement.
fn panel_font(config: &config::Config) -> Font {
    let weight = if config.panel.bold.unwrap_or(false) {
        iced::font::Weight::Bold
    } else {
        iced::font::Weight::Normal
    };
    Font {
        weight,
        ..Font::with_name(family_name_static(
            config.panel.font.as_deref().unwrap_or(DEFAULT_FONT),
        ))
    }
}

/// The font of the sensors' value texts: `panel.value_font` when set
/// (a monospaced family keeps the digits one width wide, so the
/// values never jitter), the panel font otherwise — same weight.
fn value_font(config: &config::Config) -> Font {
    let weight = if config.panel.bold.unwrap_or(false) {
        iced::font::Weight::Bold
    } else {
        iced::font::Weight::Normal
    };
    match config.panel.value_font.as_deref() {
        Some(name) => Font {
            weight,
            ..Font::with_name(family_name_static(name))
        },
        None => panel_font(config),
    }
}

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

        // Sensor boxes fill the `thickness` axis only when the section arranges
        // them *across* it (section cross axis == panel cross axis); stacked
        // along the thickness they stay content-sized (filling would split the
        // thickness between them) and the group is centred by the container.
        let stretch = vertical == (section.layout == "vertical");
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
                if !s.name.is_empty() {
                    row = row.push(text(&s.name).color(text_color).size(font_size).font(font_obj));
                }
                row.push(value_widget).into()
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
        let sensors_inner: Element<'a, Message> = if section.layout == "vertical" {
            let title_widget = container(
                text(section.name.clone())
                    .color(title_color)
                    .size(font_size.saturating_add(2))
                    .font(font_obj)
            ).align_x(Alignment::Center);
            let title_el: Element<'a, Message> = title_widget.padding(SECTION_CONTENT_PAD).into();
            let sensor_col = Column::new()
                .extend(sensor_widgets)
                .padding(SECTION_CONTENT_PAD);
            // `stretch` here means: vertical section in a vertical panel ->
            // fill the thickness (width) so the boxes stretch; the title stays
            // content-sized and is centred above them.
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
                .spacing(8)
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

// ── fill sections widget ───────────────────────────────────────────

/// Sections container used by `align: "fill"`: it fills the whole main axis
/// of the panel and distributes the sections along it, so the panel always
/// spans 100% of the available screen space (width for a horizontal panel,
/// height for a vertical one). Sections keep their intrinsic size; only the
/// gaps between them move.
///
/// The offsets are recomputed on every layout pass from the measured
/// section sizes:
///
/// * **exactly 3 sections** and enough room for both side gaps → the middle
///   section is centred on the panel: `x1 = avail/2 - w1/2`, which implies
///   `gap_left = avail/2 - w0 - w1/2` and `gap_right = avail/2 - w2 - w1/2`;
/// * any other section count, or not enough room (a gap would be negative)
///   → fallback to the standard distribution: equal spacers between the
///   sections (space-between: the `n-1` gaps share the free space, clamped
///   to 0 when the content overflows the panel).
struct FillSections<'a> {
    /// Main axis of the panel: `true` for a vertical panel (y axis).
    vertical: bool,
    children: Vec<Element<'a, Message>>,
}

impl<'a> FillSections<'a> {
    fn new(vertical: bool, children: Vec<Element<'a, Message>>) -> Self {
        Self { vertical, children }
    }

    /// Main-axis offset of each section for the given measured sizes; the
    /// two distribution rules are documented on [`FillSections`].
    fn distribute(mains: &[f32], avail: f32) -> Vec<f32> {
        let n = mains.len();

        // 3 sections: centre the middle one if both side gaps stay >= 0
        // (which also guarantees the sections fit the panel).
        if n == 3 {
            let gap_left = avail / 2.0 - mains[0] - mains[1] / 2.0;
            let gap_right = avail / 2.0 - mains[2] - mains[1] / 2.0;
            if gap_left >= 0.0 && gap_right >= 0.0 {
                return vec![0.0, avail / 2.0 - mains[1] / 2.0, avail - mains[2]];
            }
        }

        // Standard: equal spacers, first section flush at the start edge,
        // last one at the end edge (0 gap when the content overflows).
        let content: f32 = mains.iter().sum();
        let gap = if n > 1 {
            (avail - content).max(0.0) / (n as f32 - 1.0)
        } else {
            0.0
        };
        let mut x = 0.0;
        mains
            .iter()
            .map(|&m| {
                let at = x;
                x += m + gap;
                at
            })
            .collect()
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for FillSections<'_> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size { width: Length::Fill, height: Length::Fill }
    }

    /// Measures every section unbounded on the main axis (its intrinsic size
    /// decides the positions) and capped to the panel thickness on the cross
    /// axis (`Fill` children resolve to `max_cross`), then places each node
    /// at the offset given by [`FillSections::distribute`].
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &Limits,
    ) -> Node {
        // Main axis = width (horizontal panel) / height (vertical); the
        // cross axis carries the thickness and every section spans it fully.
        let (max_main, max_cross) = if self.vertical {
            (limits.max().height, limits.max().width)
        } else {
            (limits.max().width, limits.max().height)
        };

        let mut nodes = Vec::with_capacity(self.children.len());
        let mut mains: Vec<f32> = Vec::with_capacity(self.children.len());

        for (child, subtree) in self
            .children
            .iter_mut()
            .zip(tree.children.iter_mut())
        {
            let child_limits = Limits::new(
                Size::ZERO,
                if self.vertical {
                    Size::new(max_cross, f32::INFINITY)
                } else {
                    Size::new(f32::INFINITY, max_cross)
                },
            );
            let node = child
                .as_widget_mut()
                .layout(subtree, renderer, &child_limits);
            mains.push(if self.vertical {
                node.size().height
            } else {
                node.size().width
            });
            nodes.push(node);
        }

        let positions = Self::distribute(&mains, max_main);
        for (node, &at) in nodes.iter_mut().zip(&positions) {
            node.move_to_mut(if self.vertical { (0.0, at) } else { (at, 0.0) });
        }

        // Fill on both axes: the panel node spans the whole surface.
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);
        Node::with_children(size, nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
                .for_each(|((child, state), layout)| {
                    child
                        .as_widget_mut()
                        .operate(state, layout, renderer, operation);
                });
        });
    }

    /// Forwards the events to the sections (hover/click on the sensors).
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        for ((child, tree), layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
                tree, event, layout, cursor, renderer, clipboard, shell,
                viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, tree), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, tree), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .filter(|(_, layout)| layout.bounds().intersects(viewport))
        {
            child.as_widget().draw(
                tree, renderer, theme, style, layout, cursor, viewport,
            );
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a> From<FillSections<'a>> for Element<'a, Message> {
    fn from(fill: FillSections<'a>) -> Self {
        Element::new(fill)
    }
}

// ── positioning helpers ─────────────────────────────────────────────

/// Panel thickness on the cross axis, in pixels: `panel.thickness` from the
/// config (default 32px, minimum 1px). Single source of truth for the surface
/// size and for the exclusive zone hyprland reserves.
fn panel_thickness(config: &config::Config) -> u32 {
    config
        .panel
        .thickness
        .unwrap_or(config::DEFAULT_HEIGHT)
        .max(1)
}

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

/// Panel size (as the LayerSurface requests it).
///
/// The surface matches the screen on the axis *perpendicular* to the panel
/// orientation, and matches the panel's own thickness on the axis it
/// extends along:
///
/// * horizontal panel -> `screen_width  x thickness`
/// * vertical panel   -> `thickness x screen_height`
///
/// `thickness` comes from `panel.thickness` in the config (default 32px).
fn panel_size(config: &config::Config, screen_width: u32, screen_height: u32) -> iced_exwlshell::settings::LayerSize {
    let layout = config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal");
    let vertical = layout == "vertical";
    let thickness = panel_thickness(config);

    let (width, height) = if vertical {
        (iced_exwlshell::settings::Extent::px(thickness), iced_exwlshell::settings::Extent::px(screen_height))
    } else {
        (iced_exwlshell::settings::Extent::px(screen_width), iced_exwlshell::settings::Extent::px(thickness))
    };

    iced_exwlshell::settings::LayerSize { width, height }
}

/// Anchor set for the surface.
///
/// The surface is pinned to the screen edge on the axis it extends along.
/// `align` is handled inside the surface by iced (the inner panel is aligned
/// to left/center/right or top/center/bottom), so the anchor only needs the
/// single edge that sticks the panel to the screen border, which comes from
/// `panel.anchor`:
///
/// * horizontal, top    -> `Top`
/// * horizontal, bottom -> `Bottom`
/// * vertical,   left   -> `Left`
/// * vertical,   right  -> `Right`
fn panel_anchor(config: &config::Config) -> Anchor {
    let vertical = config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal") == "vertical";

    // Edge from `anchor` (default: top for horizontal, left for vertical);
    // `align` positions the panel INSIDE the surface, it does not pick the
    // edge — reading it here ignored `panel.anchor` completely.
    let anchor = config.panel.anchor.as_deref().unwrap_or(if vertical { "left" } else { "top" });

    if vertical {
        match anchor {
            "bottom" => Anchor::Bottom,
            "right" => Anchor::Right,
            _ => Anchor::Left,
        }
    } else {
        match anchor {
            "bottom" => Anchor::Bottom,
            _ => Anchor::Top,
        }
    }
}

/// Anchor + edge margins for a tooltip window at the cursor point:
/// just off the bar's anchored edge on the screen's inner side (a top
/// bar's tooltip drops below the surface, never onto it — the bar
/// surface is one `thickness` tall, there is no room inside it),
/// centred on the cursor along the bar and clamped inside the output.
fn tooltip_placement(
    anchor: Anchor,
    at: (i32, i32),
    w: u32,
    h: u32,
    thickness: i32,
    sw: i32,
    sh: i32,
) -> (Anchor, Margin) {
    let (w, h) = (w as i32, h as i32);
    let gap = TOOLTIP_GAP as i32;
    if anchor.contains(Anchor::Top) {
        let left = (at.0 - w / 2).clamp(0, (sw - w).max(0));
        (
            Anchor::Left | Anchor::Top,
            Margin {
                left,
                top: thickness + gap,
                right: 0,
                bottom: 0,
            },
        )
    } else if anchor.contains(Anchor::Bottom) {
        let left = (at.0 - w / 2).clamp(0, (sw - w).max(0));
        (
            Anchor::Left | Anchor::Bottom,
            Margin {
                left,
                top: 0,
                right: 0,
                bottom: thickness + gap,
            },
        )
    } else if anchor.contains(Anchor::Left) {
        let top = (at.1 - h / 2).clamp(0, (sh - h).max(0));
        (
            Anchor::Left | Anchor::Top,
            Margin {
                left: thickness + gap,
                top,
                right: 0,
                bottom: 0,
            },
        )
    } else {
        let top = (at.1 - h / 2).clamp(0, (sh - h).max(0));
        (
            Anchor::Right | Anchor::Top,
            Margin {
                left: 0,
                top,
                right: thickness + gap,
                bottom: 0,
            },
        )
    }
}

/// The cursor point for tooltip placement, output-relative: the
/// compositor's own view (`hyprctl cursorpos`) — `last_cursor` is
/// stale when the pointer enters the bar straight from a window below
/// (a layer surface only receives motion once the cursor is inside),
/// and the tooltip would open at the previous hover's spot.
fn tooltip_cursor_point(panel: &Panel) -> (i32, i32) {
    if let Some((gx, gy)) = hyprctl_cursor_global() {
        let (ox, oy) = tray::output_origin();
        return (gx - ox, gy - oy);
    }
    tray::cursor_output_point(panel).unwrap_or((0, 0))
}

/// The cursor's global position straight from the compositor.
fn hyprctl_cursor_global() -> Option<(i32, i32)> {
    let out = std::process::Command::new("hyprctl")
        .args(["cursorpos", "-j"])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some((v["x"].as_i64()? as i32, v["y"].as_i64()? as i32))
}

/// Open (or retarget) the tooltip window for a bar source: the text is
/// fitted to half the output width, so a long title is cut like the
/// tray menu's labels instead of stretching off-screen.
fn tooltip_open(panel: &mut Panel, id: String, text: String) -> Command<Message> {
    if text.is_empty() {
        return Command::none();
    }
    // Already this source's tooltip: nothing to rebuild.
    if panel
        .bar_tooltip
        .as_ref()
        .is_some_and(|(_, tip)| tip.id == id)
    {
        return Command::none();
    }
    let mut cmds: Vec<Command<Message>> = Vec::new();
    if let Some((win, _)) = panel.bar_tooltip.take() {
        cmds.push(iced::window::close(win));
    }
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
    let budget = (sw as u32 / 2).saturating_sub(2 * MENU_PAD);
    let fitted = text_width::fit(&font, font_size, &[(text, budget)])
        .pop()
        .unwrap_or_default();
    let text_w = u32::from(text_width::widest(&font, font_size, std::slice::from_ref(&fitted)));
    let w = text_w.saturating_add(2 * MENU_PAD).max(48);
    let h = menu_row_h(font_size).saturating_add(2 * MENU_PAD);
    let at = tooltip_cursor_point(panel);
    let thickness = panel_thickness(&panel.config) as i32;
    let (anchor, margin) =
        tooltip_placement(panel_anchor(&panel.config), at, w, h, thickness, sw, sh);
    let (win, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(w, h),
        layer: Layer::Overlay,
        anchor,
        // `-1` = ignore other surfaces' exclusive zones: the default
        // (`None` → protocol default 0) had the tooltip land *below*
        // the bar's own 30 px reservation — `thickness + gap` turned
        // into `thickness + gap + thickness`.
        exclusive_zone: Some(-1),
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: popup_output_option(),
        namespace: Some("piuma-tooltip".to_string()),
        ..Default::default()
    });
    panel.bar_tooltip = Some((win, BarTooltip { id, text: fitted }));
    cmds.push(open);
    Command::batch(cmds)
}

/// Close the tooltip — only if `id` still owns it: an exit delivered
/// after another source's enter (widget order) is stale.
fn tooltip_close(panel: &mut Panel, id: String) -> Command<Message> {
    if let Some((win, tip)) = panel.bar_tooltip.take() {
        if tip.id == id {
            return iced::window::close(win);
        }
        panel.bar_tooltip = Some((win, tip));
    }
    Command::none()
}

/// Which sides `[top, right, bottom, left]` of the section at `idx` sit
/// flush on a screen edge (roadmap M3): the bar's anchored side touches for
/// every section (the sections span the full thickness and the surface has
/// no root padding); on the length axis the flush ends follow `align` —
/// `fill` keeps the first/last section at the panel ends (both flush when a
/// single section fills the row), `left`/`top` parks only the first one
/// there, `right`/`bottom` only the last, `center` none.
fn section_touch(config: &config::Config, idx: usize, count: usize) -> [bool; 4] {
    let vertical = config.panel.section_layout.as_deref() == Some("vertical");
    let anchor = panel_anchor(config);
    let mut touch = [
        anchor.contains(Anchor::Top),
        anchor.contains(Anchor::Right),
        anchor.contains(Anchor::Bottom),
        anchor.contains(Anchor::Left),
    ];
    let align = config.panel.align.as_deref().unwrap_or("center");
    let at_start = if vertical { align == "top" } else { align == "left" };
    let at_end = if vertical { align == "bottom" } else { align == "right" };
    let (start, end) = if align == "fill" {
        (idx == 0, idx + 1 == count)
    } else if at_start {
        (idx == 0, false)
    } else if at_end {
        (false, idx + 1 == count)
    } else {
        (false, false)
    };
    if vertical {
        touch[0] |= start;
        touch[2] |= end;
    } else {
        touch[3] |= start;
        touch[1] |= end;
    }
    touch
}

/// Border radius per corner for roadmap M3: `touch = [top, right, bottom,
/// left]`; a corner renders square when **either** of its two sides is flush
/// with a screen edge, rounded to `radius` otherwise.
fn corner_radius(touch: [bool; 4], radius: f32) -> iced::border::Radius {
    let corner = |flush: bool| if flush { 0.0 } else { radius };
    iced::border::Radius {
        top_left: corner(touch[0] || touch[3]),
        top_right: corner(touch[0] || touch[1]),
        bottom_right: corner(touch[2] || touch[1]),
        bottom_left: corner(touch[2] || touch[3]),
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
struct PopupStyle {
    /// Base text size (body); the summary is +3 bold, the ✕ +2.
    font_size: u32,
    text: Color,
    text_hover: Color,
    bg: Color,
    bg_hover: Color,
    border: Color,
    border_hover: Color,
    border_width: u32,
    border_radius: u32,
    border_type: String,
    padding: u32,
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
fn popup_style(config: &config::Config, pal: Option<&palette::Palette>) -> PopupStyle {
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

/// Open or close the wallpaper surface (roadmap2 #1) so it matches the
/// config: the window exists while `panel.wallpaper` is enabled with an
/// existing file and closes otherwise. Called on every tick — the
/// comparison is cheap, and the first tick materializes the setting
/// after startup (the daemon's init closure cannot open windows).
/// One wallpaper surface per active output: a layer surface lives on a
/// single output, so the window set tracks the monitor list. Runs on
/// every tick — the output query only executes while something must be
/// shown (the disabled case costs nothing), a monitor plugged later
/// joins within a tick, and a removed one closes its surface.
fn sync_wallpaper(panel: &mut Panel) -> Command<Message> {
    let wanted = panel.config.panel.wallpaper.as_ref().is_some_and(|w| {
        w.enabled && !w.path.is_empty() && PathBuf::from(&w.path).is_file()
    });
    let desired = if wanted { monitor_names() } else { Vec::new() };
    let (to_open, to_close) = wallpaper_plan(&panel.wallpaper, &desired);
    let mut cmds: Vec<Command<Message>> = to_close.into_iter().map(iced::window::close).collect();
    for name in to_open {
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            // `FILL` = both axes to the compositor: the surface covers
            // this output whatever its size and scale (all four anchors
            // are what `FILL` requires).
            size: LayerSize::FILL,
            layer: Layer::Background,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            // -1 = ignore other surfaces' reservations: the bar's strip
            // must not inset the background (full output, exactly what
            // a wallpaper tool does).
            exclusive_zone: Some(-1),
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::OutputName(name.clone()),
            // Clicks fall through to the bare desktop: the wallpaper
            // never eats input meant for the compositor.
            events_transparent: true,
            namespace: Some("piuma-wallpaper".to_string()),
            ..Default::default()
        });
        panel.wallpaper.push((name, id));
        cmds.push(open);
    }
    Command::batch(cmds)
}

/// The derived palette rides the tick (see `sync_wallpaper`): the
/// extraction re-runs only when the wallpaper file's path or mtime
/// change — or the option just turned on — never every second. With the
/// option (or the wallpaper) off the in-memory palette is dropped, so
/// the next rebuild resolves the user's own configured colours again:
/// nothing is ever written back to the config file.
fn sync_palette(panel: &mut Panel) -> Command<Message> {
    let cfg = &panel.config.panel;
    let wanted = cfg
        .wallpaper_palette
        .unwrap_or(false)
        .then_some(cfg.wallpaper.as_ref())
        .flatten()
        .filter(|w| w.enabled)
        .filter(|w| !w.path.is_empty())
        .and_then(|w| {
            let mtime = std::fs::metadata(&w.path).ok().and_then(|m| m.modified().ok())?;
            PathBuf::from(&w.path).is_file().then_some((w.path.clone(), mtime))
        });
    let Some(key) = wanted else {
        panel.palette = None;
        panel.palette_key = None;
        return Command::none();
    };
    if panel.palette_key.as_ref() == Some(&key) {
        return Command::none();
    }
    // The decode runs off the UI thread (a 4K image is tens of ms of
    // CPU); the key travels with the result, so a failed decode retries
    // only on the next file change, not every tick.
    let task_key = key.clone();
    Command::perform(
        async move {
            let path = task_key.0.clone();
            smol::unblock(move || palette::extract(std::path::Path::new(&path))).await
        },
        move |pal| Message::PaletteReady(pal, key),
    )
}

/// Last monitor list with its timestamp: outputs change rarely (hotplug),
/// so the wallpaper sync — which asks every tick — gets a cached answer
/// for `MONITORS_TTL` instead of spawning and parsing `hyprctl` at 1 Hz.
static MONITORS: std::sync::Mutex<Option<(Instant, Vec<String>)>> = std::sync::Mutex::new(None);

/// Freshness window of [`MONITORS`]: long enough to make the per-tick
/// spawn rare, short enough that a hotplugged output gets its wallpaper
/// surface within seconds.
const MONITORS_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// Names of the currently active outputs, from `hyprctl monitors -j` —
/// best-effort, the same source the bar's geometry helpers use (cached,
/// see [`MONITORS`]); no answer = no surface for now (retried on the
/// next tick).
fn monitor_names() -> Vec<String> {
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
fn monitor_names_json(json: &str) -> Vec<String> {
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
fn focused_output() -> Option<(String, i32, i32)> {
    let output = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    focused_output_in(&String::from_utf8_lossy(&output.stdout))
}

/// The (`name`, `x`, `y`) of the `hyprctl monitors -j` entry with
/// `"focused": true`; unparseable input yields no output.
fn focused_output_in(json: &str) -> Option<(String, i32, i32)> {
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
fn active_output_name() -> Option<String> {
    focused_output().map(|(name, _, _)| name)
}

/// The output for the graph popup and its connector: the bar's own
/// ([`active_output_name`]), falling back to the old `LastOutput`
/// behaviour when hyprctl can't answer.
fn popup_output_option() -> OutputOption {
    active_output_name()
        .map(OutputOption::OutputName)
        .unwrap_or(OutputOption::LastOutput)
}

/// Pure part of the wallpaper sync: outputs that still need a window
/// and windows to close (their output is gone, or the whole feature
/// switched off — an empty wanted list then closes everything).
fn wallpaper_plan<T: Clone>(current: &[(String, T)], desired: &[String]) -> (Vec<String>, Vec<T>) {
    let to_open = desired
        .iter()
        .filter(|name| !current.iter().any(|(n, _)| n == *name))
        .cloned()
        .collect();
    let to_close = current
        .iter()
        .filter(|(name, _)| !desired.contains(name))
        .map(|(_, id)| id.clone())
        .collect();
    (to_open, to_close)
}

/// The wallpaper surface: the configured image covering the whole output
/// (cover fit keeps the aspect ratio and crops what spills over). The
/// file is re-checked here — a path removed behind the config's back
/// renders the surface empty instead of breaking the window.
fn wallpaper_view(panel: &Panel) -> Element<'_, Message> {
    let path = panel
        .config
        .panel
        .wallpaper
        .as_ref()
        .map(|w| w.path.as_str())
        .unwrap_or_default();
    if path.is_empty() || !PathBuf::from(path).is_file() {
        return container(text("")).width(Length::Fill).height(Length::Fill).into();
    }
    Image::new(iced::widget::image::Handle::from_path(path))
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(iced::ContentFit::Cover)
        .into()
}

fn sync_popup(panel: &mut Panel, reposition: bool) -> Option<Command<Message>> {
    let enabled = panel.config.panel.notifications.is_some();
    let cards = if enabled {
        // Count only, and reap the icon handles that lost their card:
        // the old path cloned every (possibly image-bearing) card each
        // tick just to read `.len()`.
        notifications::prune_icon_handles();
        notifications::active_cards_len(POPUP_MAX_CARDS)
    } else {
        0
    };
    // A changed set invalidates the hover id: expiry reaches this path
    // without passing through popup_dismiss.
    if cards != panel.popup_cards {
        panel.popup_hover = None;
    }
    match (panel.popup, cards) {
        (None, 0) => None,
        (Some(id), 0) => {
            panel.popup = None;
            panel.popup_cards = 0;
            panel.popup_hover = None;
            Some(iced::window::close(id))
        }
        (None, n) => {
            let (anchor, margin) = popup_placement(&panel.config);
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: popup_size(panel, n),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: None,
                margin: Some(margin),
                keyboard_interactivity: KeyboardInteractivity::None,
                // The focused output, like every other popup (margins
                // are pure offsets — safe on any output).
                output_option: popup_output_option(),
                namespace: Some("piuma-notifications".to_string()),
                ..Default::default()
            });
            panel.popup = Some(id);
            panel.popup_cards = n;
            Some(open)
        }
        (Some(id), n) if reposition || n != panel.popup_cards => {
            panel.popup_cards = n;
            let (anchor, margin) = popup_placement(&panel.config);
            // The margin rides along: position and margins are config keys
            // (roadmap M4) and a reload can change them at any time.
            Some(Command::batch([
                Command::done(Message::LayoutChange {
                    id,
                    anchor,
                    size: popup_size(panel, n),
                }),
                Command::done(Message::MarginChange { id, margin }),
            ]))
        }
        _ => None,
    }
}

/// Surface size for a stack of `cards`: exact, so there is no dead
/// (click-blocking) surface below the last card. The card height comes
/// from the configured popup style (see `popup_card_h`).
fn popup_size(panel: &Panel, cards: usize) -> LayerSize {
    let n = cards as u32;
    let h = popup_card_h(&popup_style(&panel.config, panel.palette.as_ref()));
    LayerSize::px(POPUP_WIDTH, n * h + n.saturating_sub(1) * POPUP_CARD_GAP)
}

/// Right-hand corner opposite to the bar's edge: a bottom bar leaves
/// the top corner, a top bar the bottom one, side bars the top corner.
fn popup_anchor(config: &config::Config) -> Anchor {
    let bar = panel_anchor(config);
    if bar == Anchor::Top {
        Anchor::Bottom | Anchor::Right
    } else {
        Anchor::Top | Anchor::Right
    }
}

/// Anchor + screen-edge margins for the notification popup (roadmap M4),
/// from `panel.notifications.popup_position` / `popup_margin_h` /
/// `popup_margin_v`. `auto` (absent/unknown key) reproduces the historic
/// layout: opposite the bar on the right, 8px from the top/right edges.
fn popup_placement(config: &config::Config) -> (Anchor, Margin) {
    let notify = config.panel.notifications.as_ref();
    let margin_h = notify.and_then(|n| n.popup_margin_h).unwrap_or(POPUP_MARGIN);
    let margin_v = notify.and_then(|n| n.popup_margin_v).unwrap_or(POPUP_MARGIN);
    let position = notify
        .and_then(|n| n.popup_position.as_deref())
        .unwrap_or("auto");
    // Margins on the two sides of a corner position (the others stay 0).
    let corner = |top: bool, left: bool| Margin {
        top: if top { margin_v } else { 0 },
        bottom: if top { 0 } else { margin_v },
        left: if left { margin_h } else { 0 },
        right: if left { 0 } else { margin_h },
    };
    // Centered positions anchor both horizontal edges (the compositor
    // centres the fixed-width surface); equal margins keep it centred.
    let center = |top: bool| Margin {
        top: if top { margin_v } else { 0 },
        bottom: if top { 0 } else { margin_v },
        left: margin_h,
        right: margin_h,
    };
    match position {
        "top-left" => (Anchor::Top | Anchor::Left, corner(true, true)),
        "top-center" => (Anchor::Top | Anchor::Left | Anchor::Right, center(true)),
        "top-right" => (Anchor::Top | Anchor::Right, corner(true, false)),
        "bottom-left" => (Anchor::Bottom | Anchor::Left, corner(false, true)),
        "bottom-center" => {
            (Anchor::Bottom | Anchor::Left | Anchor::Right, center(false))
        }
        "bottom-right" => (Anchor::Bottom | Anchor::Right, corner(false, false)),
        // `auto` and anything unknown keep the historic layout.
        _ => (
            popup_anchor(config),
            Margin {
                top: POPUP_MARGIN,
                right: POPUP_MARGIN,
                bottom: 0,
                left: 0,
            },
        ),
    }
}

/// Dismiss a popup card: close the entry with the user's reason (and
/// its `NotificationClosed` signal), refresh the sensor text at once
/// and resize the stack.
fn popup_dismiss(panel: &mut Panel, id: u32) -> Command<Message> {
    if notifications::dismiss(id) {
        notifications::spawn_closed_signal(id);
        refresh_notification_sensor(panel);
    }
    sync_popup(panel, false).unwrap_or_else(Command::none)
}

/// Rewrite every `notifications` sensor's value in place after a UI
/// change (the next tick confirms it), like the scroll/click arms do.
fn refresh_notification_sensor(panel: &mut Panel) {
    let value = notifications::display_value();
    for (si, sec) in panel.config.panel.sections.iter().enumerate() {
        for (sni, s) in sec.sensors.iter().enumerate() {
            if matches!(s.source, config::SensorSource::Notifications {}) {
                if let Some(sv) = panel
                    .sections
                    .get_mut(si)
                    .and_then(|sec| sec.sensors.get_mut(sni))
                {
                    sv.value = value.clone();
                }
            }
        }
    }
}

/// The popup stack: one card per active notification (newest on top).
/// The content block is one MouseArea (click → `PopupCardClicked` =
/// default action + dismiss) and the ✕ a separate MouseArea inside the
/// same card, so the two targets never overlap. Both targets also feed
/// the hover id, and the card's look comes from the configured style
/// through the shared `styled_box` helper (the sensors' border machinery).
fn popup_view(panel: &Panel) -> Element<'_, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let card_h = popup_card_h(&st);
    // Font audit (roadmap M2): every popup text takes the configured family;
    // the summary renders bold on top of it, body and ✕ as configured.
    let font = panel_font(&panel.config);
    let bold = Font {
        weight: iced::font::Weight::Bold,
        ..font
    };

    let mut stack: Column<'_, Message> = Column::new().spacing(POPUP_CARD_GAP as f32);
    for card in notifications::active_cards(POPUP_MAX_CARDS) {
        // Hovered cards swap text/bg/border for their hover counterparts.
        let (fg, bg, border_c) = if panel.popup_hover == Some(card.id) {
            (st.text_hover, st.bg_hover, st.border_hover)
        } else {
            (st.text, st.bg, st.border)
        };
        // Icon: image-data wins (handle cached per notification — one
        // decode, not one per frame), a real file path follows; anything
        // else (e.g. a themed icon name) renders without an icon.
        let icon: Option<Image> = match &card.icon {
            Some(notifications::CardIcon::Image) => {
                notifications::icon_handle(card.id).map(|handle| {
                    Image::new(handle)
                        .width(32)
                        .height(32)
                        .content_fit(iced::ContentFit::Contain)
                })
            }
            Some(notifications::CardIcon::Path(p)) => std::path::Path::new(p)
                .exists()
                .then(|| {
                    Image::new(iced::widget::image::Handle::from_path(p.clone()))
                        .width(32)
                        .height(32)
                        .content_fit(iced::ContentFit::Contain)
                }),
            None => None,
        };
        let mut lines: Column<'_, Message> = Column::new()
            .spacing(2)
            .push(text(card.summary).size(st.font_size + 3).font(bold).color(fg).width(Length::Fill));
        if !card.body.is_empty() {
            lines = lines.push(
                text(card.body)
                    .size(st.font_size)
                    .color(fg)
                    .font(font)
                    .width(Length::Fill),
            );
        }
        let mut content = Row::new()
            .spacing(8)
            .align_y(Alignment::Center);
        if let Some(icon) = icon {
            content = content.push(icon);
        }
        let content = content
            .push(lines.width(Length::Fill))
            .width(Length::Fill);
        let card_row: Row<'_, Message> = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .push(
                MouseArea::new(content)
                    .on_press(Message::PopupCardClicked(card.id))
                    .on_enter(Message::PopupCardEnter(card.id))
                    .on_exit(Message::PopupCardLeave),
            )
            // The ✕ gets 8px of padding: a small glyph alone is too small
            // a target, and its MouseArea must not overlap the card's.
            .push(
                MouseArea::new(
                    container(
                        text("✕")
                            .size(st.font_size + 2)
                            .color(fg)
                            .font(font),
                    )
                    .padding(8),
                )
                .on_press(Message::PopupCardDismissed(card.id))
                .on_enter(Message::PopupCardEnter(card.id))
                .on_exit(Message::PopupCardLeave),
            );
        let styled = styled_box(
            card_row.into(),
            Some(bg),
            Some(border_c),
            st.border_width.min(u16::MAX as u32) as u16,
            st.border_radius.min(u16::MAX as u32) as u16,
            &st.border_type,
            [false, false, false, false],
            st.padding.min(u16::MAX as u32) as u16,
            Length::Fill,
            Length::Fixed(card_h as f32),
        );
        // The clip keeps an overlong body inside the card box; the outer
        // size is the same `card_h` the surface math uses.
        stack = stack.push(
            container(styled)
                .width(Length::Fill)
                .height(Length::Fixed(card_h as f32))
                .clip(true),
        );
    }
    container(stack)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── notification list window ────────────────────────────────────────

/// Vertical rhythm of one list row: a double-height row (summary +
/// body, spaced) — `menu_row_h`'s maths for two stacked lines.
fn notif_list_row_h(font_size: u32) -> u32 {
    menu_row_h(font_size) * 2 + 4
}

/// The wheel's effect on the scroll offset: whole rows towards the
/// oldest, clamped so the last page lands exactly on the bottom rows.
fn scroll_rows(offset: u32, delta: i32, len: usize, visible: usize) -> u32 {
    let max = len.saturating_sub(visible);
    (offset as i32 + delta).clamp(0, max as i32) as u32
}

/// Surface size for `rows`: exact height (one `notif_list_row_h` per
/// shown row plus padding — nothing clips, every row stays hit-exact)
/// and the width from the widest truncated string, capped so a long
/// body does not take half the screen.
fn notif_list_size(rows: &[notifications::NotifRow], font: &Font, font_size: u32) -> (u32, u32) {
    let patterns: Vec<String> = rows
        .iter()
        .flat_map(|r| [r.summary.clone(), r.body.clone()])
        .collect();
    let text_w = u32::from(text_width::widest(font, font_size, &patterns));
    let w = text_w
        .saturating_add(2 * MENU_PAD)
        .min(NOTIF_LIST_W_MAX.saturating_add(2 * MENU_PAD));
    let shown = rows.len().min(NOTIF_LIST_ROWS) as u32;
    let h = shown
        .saturating_mul(notif_list_row_h(font_size))
        .saturating_add(2 * MENU_PAD);
    (w, h)
}

// ── tray menu window ────────────────────────────────────────────────

/// Vertical rhythm of one menu row: the text line (~1.2× the font)
/// plus a little air, so centered rows never clip at any font size.
fn menu_row_h(font_size: u32) -> u32 {
    font_size + font_size / 2 + 4
}

/// Surface size for `entries`: the height is exact (one `menu_row_h`
/// per row plus padding — nothing clips, every row stays hit-exact);
/// the width measures the widest label with the same shaper iced
/// renders with (`text_width::widest`), adds the deepest indent and
/// the horizontal margins on both sides — two `MENU_PAD` layers per
/// side (the list container's and each row's own) — and is clamped so
/// a one-item menu is still grabbable and a long label does not take
/// half the screen.
fn menu_size(entries: &[dbusmenu::MenuEntry], font: &Font, font_size: u32) -> (u32, u32) {
    let patterns: Vec<String> = entries
        .iter()
        .map(|e| format!("{}{}", dbusmenu::toggle_glyph(e), e.label))
        .collect();
    let text_w = u32::from(text_width::widest(font, font_size, &patterns));
    let max_depth = entries.iter().map(|e| e.depth).max().unwrap_or(0);
    let w = text_w
        .saturating_add(max_depth * MENU_INDENT)
        .saturating_add(4 * MENU_PAD)
        .clamp(MENU_MIN_W, MENU_MAX_W);
    let h = entries.len() as u32 * menu_row_h(font_size) + 2 * MENU_PAD;
    (w, h)
}

/// Cut every label to its row budget inside the finished surface width `w`
/// (the deepest indent narrows the text area of its own row, and the
/// budget keeps both `MENU_PAD` layers per side — the list container's
/// and the row's own — so a cut label ends with the same margin as a
/// short one): the toggle glyph measures with the label, so it is fitted
/// too and then stripped back off. Runs once when the menu loads —
/// [`tray_menu_view`] renders the result on a single line, so a too-long
/// label ends with an ellipsis instead of spilling over the next row.
fn fit_menu_labels(entries: &mut [dbusmenu::MenuEntry], w: u32, font: &Font, font_size: u32) {
    let jobs: Vec<(String, u32)> = entries
        .iter()
        .map(|e| {
            (
                format!("{}{}", dbusmenu::toggle_glyph(e), e.label),
                w.saturating_sub(4 * MENU_PAD + e.depth * MENU_INDENT),
            )
        })
        .collect();
    for (entry, fitted) in entries.iter_mut().zip(text_width::fit(font, font_size, &jobs)) {
        let glyph = dbusmenu::toggle_glyph(entry);
        entry.label = fitted.strip_prefix(glyph).unwrap_or_default().to_string();
    }
}

/// Anchor + edge margins for the menu at the output-relative point
/// (x, y): it opens right/down from the cursor — the natural quadrant
/// under a top/left bar — and flips to the opposite side when the
/// surface would leave the output, with the flipped edge **clamped to
/// the output**: a menu that doesn't fit on that side is cut at the
/// output edge instead of being pinned at the cursor and leaving the
/// screen (the 116-row Cachy-Update menu used to open with its base at
/// the top bar and ~2300 px of it above the screen). Cursor-driven
/// callers pin the surface to the bar's own output via
/// [`popup_output_option`], since `OutputOption::LastOutput` lands on
/// another output once a second monitor joins.
fn menu_placement(x: i32, y: i32, w: u32, h: u32, sw: i32, sh: i32) -> (Anchor, Margin) {
    let (w, h) = (w as i32, h as i32);
    // Fits: anchored at the point. Overflows: the far edge sticks to the
    // output and the surface ends at the point — unless that would push
    // the near edge off-screen, in which case the surface sits flush
    // with the output edge (cut, not lost).
    let (anchor_h, left, right) = if x + w <= sw {
        (Anchor::Left, x, 0)
    } else {
        (Anchor::Right, 0, (sw - x).clamp(0, (sw - w).max(0)))
    };
    let (anchor_v, top, bottom) = if y + h <= sh {
        (Anchor::Top, y, 0)
    } else {
        (Anchor::Bottom, 0, (sh - y).clamp(0, (sh - h).max(0)))
    };
    (
        anchor_h | anchor_v,
        Margin {
            top,
            right,
            bottom,
            left,
        },
    )
}

/// One tick of an open popup's abandonment bookkeeping: a pointer on
/// its surface or back on any sensor resets the count (the popup is
/// live), otherwise the grace ticks accumulate — `true` once they run
/// out and the popup must close.
///
/// The leave-guard alone cannot decide this: `last_cursor` can be
/// stale when the sensor's leave processes (widget messages run
/// before the cursor's own update), and once the cursor rests on
/// another application's window no event ever arrives again.
fn popup_missed_tick(gp: &mut GraphPopup, sensor_under_cursor: bool) -> bool {
    if sensor_under_cursor || gp.pointer_on_popup {
        gp.missed = 0;
        return false;
    }
    gp.missed = gp.missed.saturating_add(1);
    gp.missed >= GRAPH_ABANDON_GRACE
}

/// Run once per tick: the abandoned popup to close (its grace spent —
/// see `popup_missed_tick`), `None` while it lives.
fn reap_abandoned_popup(panel: &mut Panel) -> Option<GraphPopup> {
    let sensor_under_cursor = panel.hovered.is_some();
    let expired = match panel.graph_popup.as_mut() {
        Some(gp) => popup_missed_tick(gp, sensor_under_cursor),
        None => return None,
    };
    if !expired {
        return None;
    }
    eprintln!("[piuma] graph: close (cursor left the sensor and the popup)");
    panel.graph_popup.take()
}

/// Both hover popups in step with one sensor hover: the trend popup
/// (its own sensor type) and the calendar one — each opens only for
/// its own source, so they share no condition beyond the hover.
fn hover_sync(panel: &mut Panel, section: usize, sensor: usize, hovered: bool) -> Command<Message> {
    Command::batch([
        graph_hover_sync(panel, section, sensor, hovered),
        calendar_hover_sync(panel, section, sensor, hovered),
    ])
}

/// Keep the calendar popup in step with a sensor hover: hovering a
/// `calendar` sensor opens it at the cursor (the trend popup's
/// output-clamped placement), crossing to another calendar sensor
/// swaps it. No hover close runs here — the popup must survive three
/// more seconds after the cursor leaves both it and its sensor: the
/// tick's grace decides (see `reap_abandoned_calendar`).
fn calendar_hover_sync(
    panel: &mut Panel,
    section: usize,
    sensor: usize,
    hovered: bool,
) -> Command<Message> {
    let showing = panel
        .calendar
        .as_ref()
        .is_some_and(|cp| (cp.section, cp.sensor) == (section, sensor));
    if !hovered || showing {
        return Command::none();
    }
    // Opted in? Only the `calendar` source opens this popup (a hover
    // on any other sensor leaves an open popup counting down).
    let is_calendar = panel
        .config
        .panel
        .sections
        .get(section)
        .and_then(|sec| sec.sensors.get(sensor))
        .is_some_and(|s| matches!(s.source, config::SensorSource::Calendar { .. }));
    if !is_calendar {
        return Command::none();
    }
    let Some((x, y)) = tray::cursor_output_point(panel) else {
        // The enter can beat the first CursorMoved event (nothing moved
        // since startup): the motion itself retries via `IcedEvent`.
        eprintln!("[piuma] calendar: no cursor position yet, waiting for the motion event");
        return Command::none();
    };
    let Some(today) = calendar::today() else {
        // Without today's date there is no month to open on.
        eprintln!("[piuma] calendar: no local date, staying closed");
        return Command::none();
    };
    let mut cmds: Vec<Command<Message>> = Vec::new();
    // Crossing between two calendar sensors: the old surface dies with
    // its own leave events too, but closing here keeps the order definite.
    if let Some(old) = panel.calendar.take() {
        cmds.push(iced::window::close(old.id));
    }
    let weekdays = calendar::weekday_labels();
    let (w, h) = calendar_size(&panel.config, &weekdays);
    let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
    let (anchor, margin) = menu_placement(x, y, w, h, sw, sh);
    let rect = placement_rect(anchor, &margin, w, h, sw, sh);
    // Where the surface really lands: the bar's reservation on the
    // shared edge pushes it, so the pointer tests below must not use
    // the intended rect (it contains the cursor while the surface,
    // e.g. for a top bar, sits a zone lower).
    let surface = shifted_surface_rect(rect, anchor, panel.last_anchor, panel.last_exclusive_zone);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(w, h),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        // The bar's own output (the graph popup's `LastOutput` landed
        // on the wrong monitor once a second one joined the session).
        output_option: popup_output_option(),
        namespace: Some("piuma-calendar".to_string()),
        ..Default::default()
    });
    panel.calendar = Some(CalendarPopup {
        id,
        section,
        sensor,
        offset: 0,
        today,
        weekdays,
        month_label: calendar::month_label(today.0, today.1),
        // The cursor is on the surface only if it really lies inside
        // it: on the sensor (the common open) that is false and the
        // sensor's own hover keeps the popup alive; a flipped or
        // pushed placement opens away from the cursor either way.
        pointer_on_popup: (x >= surface.0 && x < surface.2 && y >= surface.1 && y < surface.3),
        rect: surface,
        missed: 0,
    });
    eprintln!(
        "[piuma] calendar: open rect={rect:?} surface={surface:?} cursor=({x},{y}) \
         pointer={} size={w}x{h}",
        x >= surface.0 && x < surface.2 && y >= surface.1 && y < surface.3,
    );
    cmds.push(open);
    Command::batch(cmds)
}

/// Page the calendar popup by `delta` months and refresh the header's
/// month label (one `date` spawn per click).
fn calendar_page(panel: &mut Panel, delta: i32) -> Command<Message> {
    let Some(cp) = panel.calendar.as_mut() else {
        return Command::none(); // a click can race the grace's close
    };
    cp.offset += delta;
    let (year, month) = calendar::shift_month(cp.today.0, cp.today.1, cp.offset);
    cp.month_label = calendar::month_label(year, month);
    Command::none()
}

/// Run once per tick: the calendar popup whose grace is spent —
/// neither its own sensor nor its surface has had the cursor for
/// [`CALENDAR_ABANDON_GRACE`] seconds — `None` while it lives. The
/// popup's pointer flag is event-driven (enter/exit), so this counts
/// only real absences, including after the cursor rests for good on
/// another application's window where no event ever arrives again.
fn reap_abandoned_calendar(panel: &mut Panel) -> Option<CalendarPopup> {
    let Some(cp) = panel.calendar.as_mut() else {
        return None;
    };
    if panel.hovered == Some((cp.section, cp.sensor)) || cp.pointer_on_popup {
        cp.missed = 0;
        return None;
    }
    cp.missed = cp.missed.saturating_add(1);
    if cp.missed < CALENDAR_ABANDON_GRACE {
        return None;
    }
    eprintln!("[piuma] calendar: close (cursor left the sensor and the popup)");
    panel.calendar.take()
}

/// The calendar popup's surface size from the panel's font: every cell
/// fits the widest of a day number, the weekday labels and the paging
/// arrows, and the eight rows (header, labels, six weeks) grow with
/// `panel.font_size` — the surface adapts to the font, never the
/// reverse. Returns `(width, height)`.
fn calendar_size(config: &config::Config, weekdays: &[String; 7]) -> (u32, u32) {
    let font = panel_font(config);
    let fs = config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    // Only the padding matters for the size — no colour to resolve.
    let st = popup_style(config, None);
    let mut patterns: Vec<String> = vec!["88".into(), "←".into(), "→".into()];
    patterns.extend(weekdays.iter().cloned());
    let cell_w = u32::from(text_width::widest(&font, fs, &patterns)) + 2 * CALENDAR_CELL_PAD;
    let row_h = fs.saturating_mul(14) / 10; // line height + breathing room
    let w = st.padding.saturating_mul(2).saturating_add(cell_w.saturating_mul(7));
    let h = st.padding.saturating_mul(2).saturating_add(row_h.saturating_mul(8));
    (w, h)
}

/// Keep the hover graph popup in step with a sensor hover: it opens at
/// the cursor (same output-clamped placement as the tray menu) when
/// the sensor opted in with `draw_graph` on a graphable source that
/// already recorded a sample (no data, no popup). On
/// leave it closes only when the cursor sits outside the popup rect —
/// right after opening the surface maps **under the cursor** and steals
/// the pointer from the bar, and that leave must not close it (every
/// other departure either clears the rect test or is caught by the
/// popup's own `GraphPopupExited`). When neither lands — stale
/// coordinates, a missed widget exit — the tick's reap closes the
/// popup after `GRAPH_ABANDON_GRACE` (see `reap_abandoned_popup`).
fn graph_hover_sync(
    panel: &mut Panel,
    section: usize,
    sensor: usize,
    hovered: bool,
) -> Command<Message> {
    let showing = panel
        .graph_popup
        .as_ref()
        .is_some_and(|gp| (gp.section, gp.sensor) == (section, sensor));
    if !hovered {
        if showing && !cursor_in_graph_popup(panel) {
            let gp = panel.graph_popup.take().expect("showing");
            eprintln!("[piuma] graph: close (sensor left, cursor off the popup)");
            return iced::window::close(gp.id);
        }
        if showing {
            eprintln!("[piuma] graph: keep (sensor left, cursor still on the popup)");
        }
        return Command::none();
    }
    if showing {
        return Command::none(); // this sensor is already on screen
    }
    // Opted in and already carrying data? The samples themselves
    // arrive with the ticks; without one there is no trend to show.
    let opted_in = panel
        .config
        .panel
        .sections
        .get(section)
        .and_then(|sec| sec.sensors.get(sensor))
        .is_some_and(|s| s.draw_graph && graphs::scale_of(&s.source).is_some())
        && panel.graphs.has_samples(section, sensor);
    if !opted_in {
        return Command::none();
    }
    eprintln!("[piuma] graph: hover on a graphable sensor ({section},{sensor})");
    let mut cmds: Vec<Command<Message>> = Vec::new();
    // Crossing between two graph sensors: the old surface dies with its
    // own leave events too, but closing here keeps the order definite.
    if let Some(old) = panel.graph_popup.take() {
        cmds.push(iced::window::close(old.id));
    }
    let Some((x, y)) = tray::cursor_output_point(panel) else {
        // The enter can beat the first CursorMoved event (nothing moved
        // since startup): the motion itself retries via `IcedEvent`.
        eprintln!("[piuma] graph: no cursor position yet, waiting for the motion event");
        return Command::batch(cmds);
    };
    let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
    let (anchor, margin) = menu_placement(x, y, GRAPH_POPUP_W, GRAPH_POPUP_H, sw, sh);
    let rect = placement_rect(anchor, &margin, GRAPH_POPUP_W, GRAPH_POPUP_H, sw, sh);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(GRAPH_POPUP_W, GRAPH_POPUP_H),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        // The bar's own output: `LastOutput` landed the popup on a
        // second monitor once one joined the session.
        output_option: popup_output_option(),
        namespace: Some("piuma-graph".to_string()),
        ..Default::default()
    });
    panel.graph_popup = Some(GraphPopup {
        id,
        section,
        sensor,
        rect,
        anchor,
        sensor_rect: None,
        pointer_on_popup: false,
        missed: 0,
    });
    eprintln!("[piuma] graph: open rect={rect:?} cursor=({x},{y})");
    cmds.push(open);
    // Ask the bar for the sensor box's on-screen rect: the connector
    // window is pinned to it once the answer lands (`sync_graph_link`).
    cmds.push(iced_runtime::task::widget(FindSensorRect::new(section, sensor)));
    Command::batch(cmds)
}

/// Output-space rect `(x0, y0, x1, y1)` of a surface placed by
/// `menu_placement`: the anchored side starts at its margin, the
/// flipped side's margin pins the far edge to the output.
fn placement_rect(anchor: Anchor, margin: &Margin, w: u32, h: u32, sw: i32, sh: i32) -> (i32, i32, i32, i32) {
    let (w, h) = (w as i32, h as i32);
    let (x0, x1) = if anchor.contains(Anchor::Left) {
        (margin.left, margin.left + w)
    } else {
        (sw - margin.right - w, sw - margin.right)
    };
    let (y0, y1) = if anchor.contains(Anchor::Top) {
        (margin.top, margin.top + h)
    } else {
        (sh - margin.bottom - h, sh - margin.bottom)
    };
    (x0, y0, x1, y1)
}

/// Whether the cursor sits inside the open graph popup's rect.
fn cursor_in_graph_popup(panel: &Panel) -> bool {
    let Some((x, y)) = tray::cursor_output_point(panel) else {
        return false;
    };
    panel
        .graph_popup
        .as_ref()
        .is_some_and(|gp| x >= gp.rect.0 && x < gp.rect.2 && y >= gp.rect.1 && y < gp.rect.3)
}

fn popup_surface_rect(gp: &GraphPopup, bar_anchor: Anchor, zone: i32) -> (i32, i32, i32, i32) {
    shifted_surface_rect(gp.rect, gp.anchor, bar_anchor, zone)
}

/// The placement rect shifted to where the surface really lands (the
/// graph popup's `popup_surface_rect` shares this math): a layer
/// surface is pushed past the reservations of the edge it anchors to —
/// and the bar's reservation lives on the edge the BAR anchors to. So
/// the shift applies per axis, only where the two anchors point to the
/// same side (measured on this stack: top bar, popup anchored Left|Top
/// → +28 on y, none on x).
fn shifted_surface_rect(
    rect: (i32, i32, i32, i32),
    anchor: Anchor,
    bar_anchor: Anchor,
    zone: i32,
) -> (i32, i32, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = rect;
    if anchor.contains(Anchor::Top) && bar_anchor.contains(Anchor::Top) {
        y0 += zone;
        y1 += zone;
    }
    if anchor.contains(Anchor::Bottom) && bar_anchor.contains(Anchor::Bottom) {
        y0 -= zone;
        y1 -= zone;
    }
    if anchor.contains(Anchor::Left) && bar_anchor.contains(Anchor::Left) {
        x0 += zone;
        x1 += zone;
    }
    if anchor.contains(Anchor::Right) && bar_anchor.contains(Anchor::Right) {
        x0 -= zone;
        x1 -= zone;
    }
    (x0, y0, x1, y1)
}

/// The connector drawing in the link surface's own coordinates, the
/// surface's top-left output position and its size — `None` while the
/// popup or its sensor rect is missing (the window renders empty in
/// that gap and the tick closes it).
fn graph_link_geometry(panel: &Panel) -> Option<(graphs::Connector, (i32, i32), (u32, u32))> {
    let gp = panel.graph_popup.as_ref()?;
    let sensor = gp.sensor_rect?;
    let popup = popup_surface_rect(gp, panel.last_anchor, panel.last_exclusive_zone);
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let (d1, d2, path) = graphs::connector_geometry(sensor, popup);
    // The surface is the connector's bounding box (the end dots are
    // its extremes — the rail lies between them) plus a dot-sized
    // padding on every side; the drawing shifts into local space.
    let x0 = d1.x.min(d2.x).floor() as i32 - GRAPH_LINK_PAD;
    let y0 = d1.y.min(d2.y).floor() as i32 - GRAPH_LINK_PAD;
    let x1 = d1.x.max(d2.x).ceil() as i32 + GRAPH_LINK_PAD;
    let y1 = d1.y.max(d2.y).ceil() as i32 + GRAPH_LINK_PAD;
    let shift = |p: iced::Point| iced::Point::new(p.x - x0 as f32, p.y - y0 as f32);
    let conn = graphs::Connector {
        d1: shift(d1),
        d2: shift(d2),
        path: path.iter().copied().map(shift).collect(),
        color: st.border,
        width: (st.border_width as f32).max(1.0),
    };
    Some((conn, (x0, y0), ((x1 - x0) as u32, (y1 - y0) as u32)))
}

/// One place drives the connector surface: it opens once the popup's
/// sensor rect has landed (the query answers a render after the popup
/// opened), it swaps when the popup jumps to another sensor, and it
/// dies with the popup. With `exclusive_zone = -1` the surface ignores
/// every reservation, so its margins are the exact output position of
/// the connector's bounding box.
fn sync_graph_link(panel: &mut Panel) -> Option<Command<Message>> {
    let wanted = panel
        .graph_popup
        .as_ref()
        .filter(|gp| gp.sensor_rect.is_some())
        .map(|gp| gp.id);
    // Settled: the open connector draws the popup that is showing.
    if let (Some((_, popup_id)), Some(wanted)) = (panel.graph_link, wanted) {
        if popup_id == wanted {
            return None;
        }
    }
    let mut cmds: Vec<Command<Message>> = Vec::new();
    // The popup died or moved to another sensor: the old connector
    // goes (a swap reopens it on the new geometry right after).
    if let Some((id, _)) = panel.graph_link.take() {
        eprintln!("[piuma] graph: link close");
        cmds.push(iced::window::close(id));
    }
    if let Some(popup_id) = wanted {
        let (_, (ox, oy), (w, h)) = graph_link_geometry(panel)?;
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            size: LayerSize::px(w, h),
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Left,
            // -1 = ignore other surfaces' reservations (the bar's
            // strip must not inset this one — as on the wallpaper).
            exclusive_zone: Some(-1),
            margin: Some(Margin {
                top: oy,
                right: 0,
                bottom: 0,
                left: ox,
            }),
            keyboard_interactivity: KeyboardInteractivity::None,
            // Same output as the popup it points at (see above).
            output_option: popup_output_option(),
            // A pure drawing over the desktop: hovers and clicks
            // pass through (as on the wallpaper).
            events_transparent: true,
            namespace: Some("piuma-graph-link".to_string()),
            ..Default::default()
        });
        panel.graph_link = Some((id, popup_id));
        eprintln!("[piuma] graph: link at ({ox},{oy}) size={w}x{h}");
        cmds.push(open);
    }
    Some(Command::batch(cmds))
}

/// The connector window's view: one canvas with the polyline and the
/// two end dots. The surface lives exactly as long as the popup; in
/// the tick-sized gap before its close the drawing is empty.
fn graph_link_view<'a>(panel: &'a Panel) -> Element<'a, Message> {
    match graph_link_geometry(panel) {
        Some((conn, _, _)) => graphs::link_widget(conn),
        None => graphs::link_widget(graphs::Connector {
            d1: iced::Point::new(0.0, 0.0),
            d2: iced::Point::new(0.0, 0.0),
            path: Vec::new(),
            color: Color::TRANSPARENT,
            width: 0.0,
        }),
    }
}

/// The hover graph popup: the sensor's name and current value over its
/// trend line, inside the shared popup box (the notification cards'
/// style, same as the tray menu). The whole surface is one MouseArea
/// whose exit reports the departure to `GraphPopupExited`.
fn graph_popup_view<'a>(panel: &'a Panel, gp: &GraphPopup) -> Element<'a, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let sensor = panel
        .sections
        .get(gp.section)
        .and_then(|sec| sec.sensors.get(gp.sensor));
    let (name, current, color) = match sensor {
        // The threshold rule tints the popup like the bar (header,
        // trend, labels and average all follow the reading).
        Some(s) => {
            let base = s.text_color.unwrap_or(st.text);
            let ruled = sensors::leading_number(&s.value)
                .and_then(|v| {
                    s.color_rules
                        .iter()
                        .rev()
                        .find(|(min, _)| v >= *min)
                        .map(|(_, c)| *c)
                })
                .unwrap_or(base);
            (s.name.clone(), s.value.clone(), ruled)
        }
        // The sensor died under the popup (its exit will follow): keep
        // the box empty rather than missing layers.
        None => (String::new(), "N/A".to_string(), st.text),
    };
    let (scale, samples) = panel
        .graphs
        .window(gp.section, gp.sensor)
        .unwrap_or((graphs::Scale::Auto, Vec::new()));
    // Fixed content geometry: header box + gap, canvas takes the rest
    // of the fixed-size window — the label below is positioned against
    // exactly these numbers.
    let graph_h = GRAPH_POPUP_H
        .saturating_sub(2 * st.padding)
        .saturating_sub(GRAPH_HEADER_H)
        .saturating_sub(GRAPH_SPACING) as f32;
    let header: Element<'_, Message> = container(
        Row::new()
            .push(
                text(name)
                    .size(st.font_size)
                    .color(st.text)
                    .font(font.clone())
                    .wrapping(iced::widget::text::Wrapping::None)
                    .width(Length::Fill),
            )
            // The current reading leads the header: bigger than the
            // sensor's name, in the same colour as the trend below.
            .push(
                text(current)
                    .size(st.font_size.saturating_add(6))
                    .color(color)
                    .font(font),
            )
            .align_y(Alignment::Center)
            .spacing(8)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .height(Length::Fixed(GRAPH_HEADER_H as f32))
    .into();
    // The average guide and its label share the canvas's coordinates:
    // the same `axis_y` / `y_bounds` math the draw pass uses.
    let guide = graphs::mean(&samples).map(|avg| {
        let (lo, hi) = graphs::y_bounds(scale, &samples);
        (avg, graphs::axis_y(graph_h, (avg - lo) / (hi - lo)))
    });
    // The sensor's unit feeds both the canvas scale labels and the
    // average label.
    let unit = panel
        .config
        .panel
        .sections
        .get(gp.section)
        .and_then(|sec| sec.sensors.get(gp.sensor))
        .map(|s| graphs::unit_of(&s.source))
        .unwrap_or("");
    let graph_box: Element<'_, Message> = container(graphs::widget(samples, scale, color, unit))
        .width(Length::Fill)
        .height(Length::Fixed(graph_h))
        .into();
    let graph_area: Element<'_, Message> = match guide {
        Some((avg, y)) => {
            // The label rides centred on the guide, on a tinted plate
            // that keeps it readable where it crosses the trend line
            // (line-height estimate + the plate's own padding).
            let label_h = st.font_size as f32 * 1.2 + 2.0 * LABEL_PAD_V;
            let plate = container(
                text(graphs::avg_label(avg, unit))
                    .size(st.font_size.saturating_sub(1).max(8))
                    .color(color),
            )
            .style(move |_theme: &iced::Theme| ContainerStyle {
                background: Some(Background::Color(Color {
                    a: LABEL_BG_ALPHA,
                    ..st.bg
                })),
                ..ContainerStyle::default()
            })
            .padding(iced::Padding {
                top: LABEL_PAD_V,
                bottom: LABEL_PAD_V,
                left: LABEL_PAD_H,
                right: LABEL_PAD_H,
            });
            let label: Element<'_, Message> = container(plate)
                .width(Length::Fill)
                .height(Length::Fixed(graph_h))
                .align_x(Alignment::Center)
                .padding(iced::Padding {
                    top: (y - label_h / 2.0).clamp(0.0, (graph_h - label_h).max(0.0)),
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .into();
            Stack::with_children([graph_box, label])
                .width(Length::Fill)
                .height(Length::Fixed(graph_h))
                .into()
        }
        None => graph_box,
    };
    let content: Element<'_, Message> = Column::new()
        .push(header)
        .push(graph_area)
        .spacing(GRAPH_SPACING as f32)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    let surface = styled_box(
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(st.padding as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The content carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    MouseArea::new(surface).on_exit(Message::GraphPopupExited).into()
}

/// The calendar popup: the month header with its paging arrows, the
/// weekday labels and the month's Monday-first grid, inside the shared
/// popup box (the notification cards' style, like the graph popup).
/// The panel's font and font size draw every cell — the surface was
/// sized from them when the popup opened (`calendar_size`) — and today's
/// number stands out on the accent plate of the notifications' border.
fn calendar_popup_view<'a>(panel: &'a Panel, cp: &CalendarPopup) -> Element<'a, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let fs = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let accent = st.border;
    let (year, month) = calendar::shift_month(cp.today.0, cp.today.1, cp.offset);
    let weeks = calendar::month_matrix(year, month);

    // One grid cell: its content centred in the cell's equal share.
    let cell = |content: Element<'a, Message>| -> Element<'a, Message> {
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    };
    // A paging arrow: a hit target as tall as the header row.
    let arrow = |label: &'static str, msg: Message| -> Element<'a, Message> {
        MouseArea::new(
            container(text(label).size(fs).color(st.text).font(font.clone()))
                .width(Length::Fixed(fs as f32 * 1.6))
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(msg)
        .into()
    };

    let header = Row::new()
        .push(arrow("←", Message::CalendarPrev))
        .push(
            text(cp.month_label.clone())
                .size(fs)
                .color(st.text)
                .font(font.clone())
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .push(arrow("→", Message::CalendarNext))
        .width(Length::Fill)
        .height(Length::Fill);
    let mut labels = Row::new().width(Length::Fill).height(Length::Fill);
    for label in &cp.weekdays {
        labels = labels.push(cell(
            text(label.clone())
                .size(fs)
                .color(st.text)
                .font(font.clone())
                .into(),
        ));
    }
    // Eight equal rows: header, weekday labels and the six week rows.
    let mut rows = Column::new().width(Length::Fill).height(Length::Fill);
    rows = rows.push(header).push(labels);
    for week in weeks {
        let mut row = Row::new().width(Length::Fill).height(Length::Fill);
        for day in week {
            // Today only exists in the month the offset started from.
            let is_today = cp.offset == 0 && day == Some(cp.today.2);
            let day_font = if is_today {
                iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font.clone()
                }
            } else {
                font.clone()
            };
            let body: Element<'_, Message> = match day {
                Some(d) => text(d.to_string())
                    .size(fs)
                    .color(st.text)
                    .font(day_font)
                    .into(),
                None => Space::new().into(),
            };
            let c = container(body)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center);
            let day_cell: Element<'_, Message> = if is_today {
                c.style(move |_theme: &iced::Theme| {
                    let mut style = ContainerStyle::default();
                    style.background = Some(Background::Color(accent));
                    style.border.radius = corner_radius([false; 4], 4.0);
                    style
                })
                .into()
            } else {
                c.into()
            };
            row = row.push(day_cell);
        }
        rows = rows.push(row);
    }
    let content: Element<'_, Message> = rows.into();
    let surface = styled_box(
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(st.padding as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The content carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    // The whole surface reports its departure: the tick's grace takes
    // it from there (the cursor may still return within three seconds).
    MouseArea::new(surface).on_exit(Message::CalendarExited).into()
}

/// The tray menu surface: one row per entry inside the styled box
/// (shared with the sensors and the notification cards), the whole list
/// wrapped in a MouseArea whose exit dismisses the menu (the approved
/// auto-close). Clickable rows carry their own MouseArea — press reports
/// the entry, enter/leave drive the hover style.
fn tray_menu_view(panel: &Panel) -> Element<'_, Message> {
    let Some((_, menu)) = panel.tray_menu.as_ref() else {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let row_h = menu_row_h(font_size) as f32;
    let dim = Color::from_rgba(st.text.r, st.text.g, st.text.b, 0.45);
    // The configured hover colors equal the base ones — a color swap on
    // hover would be invisible, so the feedback is an underline bar under
    // the label (plus a hand pointer on the row itself).
    let underline_hover = st.text_hover == st.text && st.bg_hover == st.bg;
    let mut list: Column<'_, Message> = Column::new();
    for entry in &menu.entries {
        let padding = iced::Padding {
            top: 0.0,
            right: MENU_PAD as f32,
            bottom: 0.0,
            left: (MENU_PAD + entry.depth * MENU_INDENT) as f32,
        };
        let row: Element<'_, Message> = match entry.kind {
            dbusmenu::EntryKind::Separator => container(
                container(Space::new().width(Length::Fill).height(1))
                    .style(move |_theme| ContainerStyle {
                        background: Some(Background::Color(st.border)),
                        ..ContainerStyle::default()
                    }),
            )
            .width(Length::Fill)
            .height(Length::Fixed(row_h))
            .align_y(Alignment::Center)
            .padding(padding)
            .into(),
            // A submenu header is a section label, never a target.
            dbusmenu::EntryKind::Header => container(
                text(&entry.label)
                    .size(font_size)
                    .color(st.text)
                    .font(font)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .width(Length::Fill)
            .height(Length::Fixed(row_h))
            .align_y(Alignment::Center)
            .padding(padding)
            .into(),
            dbusmenu::EntryKind::Item => {
                let hovered = entry.enabled && panel.tray_menu_hover == Some(entry.id);
                let fg = if !entry.enabled {
                    dim
                } else if hovered {
                    st.text_hover
                } else {
                    st.text
                };
                let label = text(format!(
                    "{}{}",
                    dbusmenu::toggle_glyph(entry),
                    entry.label
                ))
                .size(font_size)
                .color(fg)
                .font(font)
                .wrapping(iced::widget::text::Wrapping::None);
                // In underline mode the column shrinks to the label and the
                // bar fills it — the text's exact width without measuring,
                // and the bar is laid out even while transparent (only its
                // color changes on hover), so nothing jumps.
                let inner: Element<'_, Message> = if underline_hover {
                    Column::new()
                        .push(label.width(Length::Shrink))
                        .push(
                            container(Space::new().width(Length::Fill).height(Length::Fixed(2.0)))
                                .style(move |_theme| ContainerStyle {
                                    background: hovered.then(|| Background::Color(fg)),
                                    ..ContainerStyle::default()
                                }),
                        )
                        .spacing(1)
                        .into()
                } else {
                    label.width(Length::Fill).into()
                };
                let body = container(inner)
                    .width(Length::Fill)
                    .height(Length::Fixed(row_h))
                    .align_y(Alignment::Center)
                    .padding(padding);
                let styled = if hovered {
                    body.style(move |_theme| ContainerStyle {
                        background: Some(Background::Color(st.bg_hover)),
                        ..ContainerStyle::default()
                    })
                } else {
                    body.style(move |_theme| ContainerStyle::default())
                };
                if entry.enabled {
                    MouseArea::new(styled)
                        .on_press(Message::TrayMenuClicked(entry.id))
                        .on_enter(Message::TrayMenuEnter(entry.id))
                        .on_exit(Message::TrayMenuLeave(entry.id))
                        .interaction(iced::mouse::Interaction::Pointer)
                        .into()
                } else {
                    styled.into()
                }
            }
        };
        list = list.push(row);
    }
    let surface = styled_box(
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(MENU_PAD as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The list carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    MouseArea::new(surface).on_exit(Message::TrayMenuClosed).into()
}

// ── notification list popup ─────────────────────────────────────────

/// Toggle the notification history list (the notifications sensor's
/// click): closes it when open; opens it at the notification cards'
/// corner with the newest entries — nothing to reread → nothing to
/// open.
fn toggle_notif_list(panel: &mut Panel) -> Command<Message> {
    if let Some((id, _)) = panel.notif_list.take() {
        return iced::window::close(id);
    }
    let rows = notifications::history_rows(NOTIF_LIST_MAX);
    if rows.is_empty() {
        return Command::none();
    }
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let (w, h) = notif_list_size(&rows, &font, font_size);
    // The cards' own corner: the list belongs to the same notifications.
    let (anchor, margin) = popup_placement(&panel.config);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(w, h),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: popup_output_option(),
        namespace: Some("piuma-notif-list".to_string()),
        ..Default::default()
    });
    panel.notif_list = Some((id, NotifList { offset: 0 }));
    open
}

/// The history list: newest on top, the wheel scrolls by whole rows, a
/// row invokes its notification's spec `default` action and closes the
/// popup (the sensor's old click). Closed entries render dimmed; the
/// rows follow the notification popup style (palette included).
fn notif_list_view(panel: &Panel) -> Element<'_, Message> {
    let Some((_, state)) = panel.notif_list.as_ref() else {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let row_h = notif_list_row_h(font_size) as f32;
    let dim = Color::from_rgba(st.text.r, st.text.g, st.text.b, 0.45);
    let dim_closed = Color::from_rgba(st.text.r, st.text.g, st.text.b, 0.30);
    let mut list: Column<'_, Message> = Column::new().spacing(2.0);
    // Bound so the rows outlive the loop (the elements borrow them).
    let rows = notifications::history_rows(NOTIF_LIST_MAX);
    for row in rows
        .iter()
        .skip(state.offset as usize)
        .take(NOTIF_LIST_ROWS)
    {
        let (summary_fg, body_fg) = if row.closed {
            (dim_closed, dim_closed)
        } else {
            (st.text, dim)
        };
        let inner = Column::new()
            .push(
                text(row.summary.clone())
                    .size(font_size)
                    .color(summary_fg)
                    .font(font),
            )
            .push(
                text(row.body.clone())
                    .size(font_size.saturating_sub(2))
                    .color(body_fg)
                    .font(font),
            )
            .spacing(2);
        let styled = container(inner)
            .width(Length::Fill)
            .height(Length::Fixed(row_h))
            .align_y(Alignment::Center)
            .padding(iced::Padding::from(MENU_PAD as f32));
        list = list.push(
            MouseArea::new(styled)
                .on_press(Message::NotifListClicked(row.id))
                .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    let surface = styled_box(
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(MENU_PAD as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The list carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    // Only non-zero pixel deltas act (one adjustment per notch: the
    // line form always accompanies the pixel one — see the sensors).
    MouseArea::new(surface)
        .on_scroll(move |delta| match delta {
            iced::mouse::ScrollDelta::Pixels { y, .. } if y != 0.0 => Message::NotifListScrolled(y),
            _ => Message::NotifListScrolled(0.0),
        })
        .into()
}

// ── taskbar menu window ─────────────────────────────────────────────

/// Surface size for the taskbar menu: one `menu_row_h` row per
/// workspace (1..9), width from the widest label — exact, so every
/// row stays hit-exact.
fn taskbar_menu_size(font: &Font, font_size: u32) -> (u32, u32) {
    let labels: Vec<String> = (1..=9).map(|n| n.to_string()).collect();
    let text_w = u32::from(text_width::widest(font, font_size, &labels));
    let w = text_w.saturating_add(2 * MENU_PAD).max(48);
    let h = 9u32
        .saturating_mul(menu_row_h(font_size))
        .saturating_add(2 * MENU_PAD);
    (w, h)
}

/// The taskbar's move menu: workspace rows 1..9, the hovered one
/// highlighted; a press moves the menu's window there silently and
/// closes the menu, and leaving the surface closes without acting
/// (same exit rule as the tray menu).
fn taskbar_menu_view(panel: &Panel) -> Element<'_, Message> {
    if panel.taskbar_menu.is_none() {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let row_h = menu_row_h(font_size);
    // A hover row rounds at most to half its own height.
    let radius = (st.border_radius as f32).min(row_h as f32 / 2.0);
    let mut list: Column<'_, Message> = Column::new();
    for workspace in 1..=9u32 {
        let hovered = panel.taskbar_menu_hover == Some(workspace);
        // The hover fill follows the menu's own corners: the top row
        // rounds up, the last rounds down, the middle rows meet their
        // neighbours square (`corner_radius`: touching sides are flat).
        let touch = if workspace == 1 {
            [false, false, true, false]
        } else if workspace == 9 {
            [true, false, false, false]
        } else {
            [true, true, true, true]
        };
        let cell = container(
            // Centred in the cell (width Fill only on the cell): the
            // label keeps its own width and sits mid-menu instead of
            // glued to the left edge.
            text(workspace.to_string())
                .size(font_size)
                .color(st.text)
                .font(font),
        )
        .width(Length::Fill)
        .height(Length::Fixed(row_h as f32))
        .center_x(Length::Fill)
        .align_y(Alignment::Center)
        .style(move |_theme| ContainerStyle {
            background: hovered.then(|| Background::Color(st.bg_hover)),
            border: iced::Border {
                radius: corner_radius(touch, radius),
                ..Default::default()
            },
            ..ContainerStyle::default()
        });
        list = list.push(
            MouseArea::new(cell)
                .on_press(Message::TaskbarMenuWs(workspace))
                .on_enter(Message::TaskbarMenuHover(Some(workspace)))
                // The exit names its row: a stale one (arriving after
                // the next row's enter) must not clear the new hover.
                .on_exit(Message::TaskbarMenuLeft(workspace))
                .interaction(iced::mouse::Interaction::Pointer),
        );
    }
    let surface = styled_box(
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(MENU_PAD as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The list carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    MouseArea::new(surface).on_exit(Message::TaskbarMenuClosed).into()
}

// ── bar tooltip window ──────────────────────────────────────────────

/// The tooltip window: one fitted line in the popup style — a label
/// just off the bar (see [`tooltip_placement`]), never a strip inside
/// the bar surface itself.
fn tooltip_window_view(panel: &Panel) -> Element<'_, Message> {
    let Some((_, tip)) = panel.bar_tooltip.as_ref() else {
        // The window id exists only together with its state.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let body = container(
        text(&tip.text)
            .size(font_size)
            .color(st.text)
            .font(font)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .align_y(Alignment::Center)
    .padding(iced::Padding::from(MENU_PAD as f32));
    styled_box(
        body.into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        0,
        Length::Fill,
        Length::Fill,
    )
}

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

// ── single instance ─────────────────────────────────────────────────

/// The bytes of a `/proc/<pid>/cmdline` are NUL separated; the holder
/// counts as the bar only when some argument *is* the bar binary (any
/// launch form), so a recycled pid or the setup binary is never
/// signalled.
fn cmdline_is_piuma(cmdline: &[u8]) -> bool {
    cmdline
        .split(|b| *b == 0)
        .any(|arg| arg == b"piuma" || arg.ends_with(b"/piuma"))
}

/// The holder pid written inside the lock file (plain digits); anything
/// else means "nothing to replace".
fn lock_pid(bytes: &[u8]) -> Option<i32> {
    std::str::from_utf8(bytes).ok()?.trim().parse().ok()
}

/// One bar per session: hyprland's own `hyprland.start` block re-runs
/// piuma whenever the compositor restarts, so an exclusive flock on
/// `$XDG_RUNTIME_DIR/piuma.lock`, held for the whole life of the
/// process, keeps a single instance — the `File` *is* the lock
/// (dropping it, or dying, releases it, so a stale file can never wedge
/// a start). A holder still alive is replaced, the most recent launch
/// winning. `None` means "run unlocked" (no runtime dir, unusable
/// lock): a degraded start, never a failed one.
fn acquire_instance_lock() -> Option<std::fs::File> {
    use std::fs::TryLockError;
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("[piuma] no XDG_RUNTIME_DIR — running unlocked");
        return None;
    };
    let path = PathBuf::from(runtime).join("piuma.lock");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) => {
            eprintln!(
                "[piuma] cannot open {} ({e}) — running unlocked",
                path.display()
            );
            return None;
        }
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = std::fs::write(&path, std::process::id().to_string());
            Some(file)
        }
        // A previous instance still holds it: replace it and wait,
        // bounded, for its lock to come free.
        Err(TryLockError::WouldBlock) => {
            if take_over_lock(&file, &path) {
                Some(file)
            } else {
                None
            }
        }
        Err(e) => {
            eprintln!("[piuma] lock unusable ({e}) — running unlocked");
            None
        }
    }
}

/// Replacement loop: identify the holder from the lock file and SIGTERM
/// it once its `/proc` cmdline is confirmed to be piuma — the pid is
/// re-read inside the loop, because the holder may still be in the
/// window between locking and writing it — then claim the lock as soon
/// as the old process has released it. piuma installs no signal
/// handler, so SIGTERM is an immediate exit by default; the two second
/// bound only guards a holder that would refuse to die (and any
/// holder, foreign or dead, that is not piuma is never signalled).
/// Returns whether the lock was claimed in time.
fn take_over_lock(file: &std::fs::File, path: &std::path::Path) -> bool {
    let mut signalled: Option<i32> = None;
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if file.try_lock().is_ok() {
            let _ = std::fs::write(path, std::process::id().to_string());
            eprintln!("[piuma] single instance: lock acquired");
            return true;
        }
        if let Some(pid) = std::fs::read(path)
            .ok()
            .and_then(|bytes| lock_pid(&bytes))
            .filter(|&pid| {
                pid != std::process::id() as i32 && signalled != Some(pid)
            })
        {
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if cmdline_is_piuma(&cmdline) {
                eprintln!("[piuma] single instance: replacing the previous bar (pid {pid})");
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status();
                signalled = Some(pid);
            }
        }
        if Instant::now() >= deadline {
            eprintln!("[piuma] single instance: previous bar did not exit — running unlocked");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
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
    let _instance_lock = acquire_instance_lock();

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
    app_theme::sync(&resolved_config_path, &config.panel);

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
                    Panel {
                        config: cfg,
                        sections: Vec::new(),
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
        calendar_size, cmdline_is_piuma, config, corner_radius, dbusmenu, fit_menu_labels,
        focused_output_in, icons_theme, lock_pid, menu_placement, menu_row_h, menu_size,
        monitor_names_json, notif_list_row_h, notif_list_size, notifications, palette,
        panel_background, popup_card_h, popup_missed_tick, popup_placement, popup_style,
        scroll_rows, section_touch, shifted_surface_rect, taskbar_menu_size, text_width,
        tooltip_placement, wallpaper_plan, GraphPopup, PopupStyle, MENU_INDENT, MENU_MAX_W,
        MENU_MIN_W, MENU_PAD, NOTIF_LIST_W_MAX,
    };
    use iced::font::Font;
    use iced_exwlshell::reexport::Anchor;

    /// The lock's holder must be recognised as the bar itself in every
    /// launch form — and never as the setup binary or an unrelated
    /// process that happens to own a recycled pid.
    #[test]
    fn lock_cmdline_only_matches_the_bar_binary() {
        assert!(cmdline_is_piuma(b"./target/release/piuma\0"));
        assert!(cmdline_is_piuma(
            b"/home/effedi/Documenti/Rust/Piuma/target/release/piuma\0"
        ));
        assert!(cmdline_is_piuma(b"piuma\0"));
        assert!(!cmdline_is_piuma(b"/usr/bin/piuma-setup\0"));
        assert!(!cmdline_is_piuma(b"python3\0"));
        assert!(!cmdline_is_piuma(b""));
    }

    /// The pid is plain digits inside the lock file; anything else
    /// (empty or corrupted file) means "no holder to replace".
    #[test]
    fn lock_pid_reads_the_holder() {
        assert_eq!(lock_pid(b"12345\n"), Some(12345));
        assert_eq!(lock_pid(b" 42 "), Some(42));
        assert_eq!(lock_pid(b""), None);
        assert_eq!(lock_pid(b"not-a-pid"), None);
    }

    /// The flock semantics the takeover relies on: two handles of the
    /// same file (two launches) conflict, and the lock is released with
    /// the closing handle — a dead holder can never wedge the start.
    #[test]
    fn lock_blocks_the_second_handle_until_the_first_drops() {
        use std::fs::TryLockError;
        let path =
            std::env::temp_dir().join(format!("piuma-lock-test-{}", std::process::id()));
        let open = || {
            std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path)
                .expect("lock file must open")
        };
        let first = open();
        first.try_lock().expect("a free lock must be claimable");
        let second = open();
        assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
        drop(first); // process death behaves like this drop
        second.try_lock().expect("the lock must be free again");
        drop(second);
        let _ = std::fs::remove_file(&path);
    }

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

    /// `auto` (no `popup_position`) keeps the historic layout exactly: the
    /// corner opposite the bar on the right, 8px from the top/right edges.
    #[test]
    fn popup_placement_auto_keeps_the_original_layout() {
        let cfg = crate::config::default_config(); // horizontal bar, top
        let (anchor, margin) = popup_placement(&cfg);
        assert_eq!(anchor, Anchor::Bottom | Anchor::Right);
        assert_eq!(
            (margin.top, margin.right, margin.bottom, margin.left),
            (8, 8, 0, 0)
        );
    }

    /// An explicit position anchors the popup on that corner and only the
    /// two relevant margins carry the configured values (roadmap M4).
    #[test]
    fn popup_placement_honours_explicit_position_and_margins() {
        let cfg: crate::config::Config = serde_json::from_str(
            r##"{
                "panel": {
                    "anchor": "top",
                    "section_layout": "horizontal",
                    "notifications": {
                        "popup_position": "bottom-left",
                        "popup_margin_h": 20,
                        "popup_margin_v": 4
                    },
                    "sections": []
                }
            }"##,
        )
        .expect("config must parse");
        let (anchor, margin) = popup_placement(&cfg);
        assert_eq!(anchor, Anchor::Bottom | Anchor::Left);
        assert_eq!(
            (margin.top, margin.right, margin.bottom, margin.left),
            (0, 0, 4, 20)
        );
    }

    /// An unknown position key falls back to `auto` (bar-opposite layout).
    #[test]
    fn popup_placement_falls_back_to_auto_on_unknown_keys() {
        let cfg: crate::config::Config = serde_json::from_str(
            r##"{
                "panel": {
                    "anchor": "top",
                    "section_layout": "horizontal",
                    "notifications": { "popup_position": "diagonal" },
                    "sections": []
                }
            }"##,
        )
        .expect("config must parse");
        let (anchor, _) = popup_placement(&cfg);
        assert_eq!(anchor, Anchor::Bottom | Anchor::Right);
    }

    /// Touching sides `[top, right, bottom, left]` (roadmap M3): the bar's
    /// anchored side always touches; on the length axis `fill` flushes the
    /// first/last section (a single one on both ends), `center` none, and a
    /// vertical bar parked at the top gives its first section the top side.
    #[test]
    fn section_touch_follows_anchor_and_align() {
        let mut cfg = crate::config::default_config(); // horizontal, top, center
        assert_eq!(section_touch(&cfg, 0, 3), [true, false, false, false]);
        assert_eq!(section_touch(&cfg, 2, 3), [true, false, false, false]);

        cfg.panel.align = Some("fill".to_string());
        assert_eq!(section_touch(&cfg, 0, 2), [true, false, false, true]);
        assert_eq!(section_touch(&cfg, 1, 2), [true, true, false, false]);
        assert_eq!(section_touch(&cfg, 0, 1), [true, true, false, true]);

        cfg.panel.section_layout = Some("vertical".to_string());
        cfg.panel.align = Some("top".to_string());
        assert_eq!(section_touch(&cfg, 0, 2), [true, false, false, true]);
        assert_eq!(section_touch(&cfg, 1, 2), [false, false, false, true]);
    }

    /// A corner squares when either of its sides touches (roadmap M3): a
    /// top-anchored row's first section keeps only the bottom-right rounding,
    /// and a section touching nothing keeps the pre-M3 look on all corners.
    #[test]
    fn corner_radius_squares_only_corners_by_touching_sides() {
        let r = corner_radius([true, false, false, true], 12.0);
        assert_eq!(r.top_left, 0.0); // top AND left touch
        assert_eq!(r.top_right, 0.0); // top touches (right does not)
        assert_eq!(r.bottom_left, 0.0); // left touches (bottom does not)
        assert_eq!(r.bottom_right, 12.0); // neither side touches
        let none = corner_radius([false; 4], 12.0);
        assert_eq!(
            (none.top_left, none.top_right, none.bottom_right, none.bottom_left),
            (12.0, 12.0, 12.0, 12.0)
        );
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

    /// The height is exact (rows × row height + padding — the surface
    /// must match the view pixel for pixel) and the width honours the
    /// minimum/maximum bounds whatever the labels are.
    #[test]
    fn menu_size_is_exact_and_width_stays_in_bounds() {
        let font = Font::with_name("Noto Sans");
        let entry = |depth: u32, label: &str| dbusmenu::MenuEntry {
            id: 1,
            label: label.to_string(),
            enabled: true,
            toggle: 0,
            depth,
            kind: dbusmenu::EntryKind::Item,
        };
        let rows = vec![entry(0, "one"), entry(1, "two"), entry(2, "three")];
        let (_, h) = menu_size(&rows, &font, 14);
        assert_eq!(h, 3 * menu_row_h(14) + 2 * MENU_PAD);
        // A tiny menu never shrinks below the minimum…
        let (w, _) = menu_size(&[entry(0, "x")], &font, 14);
        assert_eq!(w, MENU_MIN_W);
        // …nor a huge label past the maximum.
        let (w, _) = menu_size(&[entry(0, &"l".repeat(400))], &font, 14);
        assert_eq!(w, MENU_MAX_W);
    }

    /// Labels are cut to their row budget inside the finished surface (the
    /// toggle glyph measures with them and comes back stripped off), while
    /// short labels pass through untouched.
    #[test]
    fn fit_menu_labels_cuts_long_and_keeps_short_labels() {
        let font = Font::with_name("Noto Sans");
        let entry = |label: &str, toggle: i32| dbusmenu::MenuEntry {
            id: 1,
            label: label.to_string(),
            enabled: true,
            toggle,
            depth: 0,
            kind: dbusmenu::EntryKind::Item,
        };
        // The narrowest legal surface: the long label cannot fit it.
        let w = MENU_MIN_W;
        let mut rows = vec![entry("Short", 0), entry(&"L".repeat(400), 1)];
        fit_menu_labels(&mut rows, w, &font, 12);
        assert_eq!(rows[0].label, "Short");
        assert!(rows[1].label.ends_with('…'), "{}", rows[1].label);
        // The drawn string (glyph included) measures within the budget,
        // which carries both MENU_PAD layers per side (list + row).
        let drawn = format!("{}{}", dbusmenu::toggle_glyph(&rows[1]), rows[1].label);
        let budget = w - 4 * MENU_PAD;
        assert!(text_width::widest(&font, 12, &[drawn]) <= budget as u16);
    }

    /// In the unclamped middle of the range, depth costs exactly
    /// `MENU_INDENT` per level (the rows' indent and the estimate agree).
    #[test]
    fn menu_size_prices_submenus_by_indent() {
        let font = Font::with_name("Noto Sans");
        let entry = |depth: u32| dbusmenu::MenuEntry {
            id: 1,
            label: "Clickable menu entry row".to_string(),
            enabled: true,
            toggle: 0,
            depth,
            kind: dbusmenu::EntryKind::Item,
        };
        let (flat, _) = menu_size(&[entry(0)], &font, 14);
        let (deep, _) = menu_size(&[entry(3)], &font, 14);
        // Both widths must sit in the unclamped middle for the diff to
        // be exact (guards against a pathological font measure).
        assert!(flat > MENU_MIN_W && deep < MENU_MAX_W, "{flat} / {deep}");
        assert_eq!(deep - flat, 3 * MENU_INDENT);
    }

    /// The list popup's wheel: whole rows towards the oldest, clamped
    /// at both ends (the last page lands on the bottom rows) and a
    /// history shorter than a page never scrolls.
    #[test]
    fn notif_list_scroll_clamps_by_whole_rows() {
        assert_eq!(scroll_rows(0, 3, 20, 8), 3);
        assert_eq!(scroll_rows(3, -5, 20, 8), 0, "up clamps at the newest");
        assert_eq!(
            scroll_rows(10, 4, 20, 8),
            12,
            "down clamps at the last page"
        );
        assert_eq!(scroll_rows(0, 2, 5, 8), 0, "a short history never scrolls");
    }

    /// The list popup's surface: exact height for the shown rows (the
    /// cap at `NOTIF_LIST_ROWS`) and the measured width inside the cap.
    #[test]
    fn notif_list_size_is_exact_and_capped() {
        let font = Font::with_name("Noto Sans");
        let row = |i: usize| notifications::NotifRow {
            id: i as u32,
            summary: format!("summary {i}"),
            body: "a body line".to_string(),
            closed: false,
        };
        let rows: Vec<_> = (0..10)
            .map(|i| notifications::NotifRow {
                id: i as u32,
                summary: format!("summary {i} with a tail long enough to widen the box"),
                body: "a body line".to_string(),
                closed: false,
            })
            .collect();
        let (w, h) = notif_list_size(&rows, &font, 14);
        assert_eq!(
            h,
            8 * notif_list_row_h(14) + 2 * MENU_PAD,
            "only NOTIF_LIST_ROWS rows are shown"
        );
        assert!(
            w <= NOTIF_LIST_W_MAX + 2 * MENU_PAD,
            "the measured width stays capped, got {w}"
        );
        let (w_short, h_short) = notif_list_size(&[row(0)], &font, 14);
        assert_eq!(h_short, notif_list_row_h(14) + 2 * MENU_PAD);
        assert!(w_short < w, "fewer rows measure narrower");
    }

    /// The taskbar menu is exactly nine workspace rows (1..9) plus the
    /// pad layers, and never narrower than a hit-friendly minimum.
    #[test]
    fn taskbar_menu_size_prices_nine_workspace_rows() {
        let font = Font::with_name("Noto Sans");
        let (w, h) = taskbar_menu_size(&font, 14);
        assert_eq!(h, 9 * menu_row_h(14) + 2 * MENU_PAD);
        assert!(w >= 48, "the menu keeps a hit-friendly minimum, got {w}");
        let (w_big, h_big) = taskbar_menu_size(&font, 24);
        assert!(w_big >= w, "a bigger font never measures narrower");
        assert!(h_big > h, "a bigger font never measures shorter");
    }

    /// A tooltip window sits just off the bar's anchored edge on the
    /// inner side of the screen — a top bar's drops below the surface —
    /// centred on the cursor and clamped inside the output.
    #[test]
    fn tooltip_placement_sits_off_the_bar_and_inside_the_output() {
        // Top bar: below the surface (bar 30 + gap 8), centred on x.
        let (a, m) = tooltip_placement(Anchor::Top, (100, 15), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Left | Anchor::Top);
        assert_eq!((m.left, m.top), (60, 38));
        // Near the left edge the centring clamps to the output edge…
        let (_, m) = tooltip_placement(Anchor::Top, (10, 15), 80, 20, 30, 1920, 1080);
        assert_eq!(m.left, 0);
        // …and near the right edge to its far edge.
        let (_, m) = tooltip_placement(Anchor::Top, (1915, 15), 80, 20, 30, 1920, 1080);
        assert_eq!(m.left, 1920 - 80);
        // Bottom bar: above the surface.
        let (a, m) = tooltip_placement(Anchor::Bottom, (100, 1065), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Left | Anchor::Bottom);
        assert_eq!(m.bottom, 38);
        // Side bars push inward.
        let (a, m) = tooltip_placement(Anchor::Left, (15, 500), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Left | Anchor::Top);
        assert_eq!(m.left, 38);
        let (a, m) = tooltip_placement(Anchor::Right, (1905, 500), 80, 20, 30, 1920, 1080);
        assert_eq!(a, Anchor::Right | Anchor::Top);
        assert_eq!(m.right, 38);
    }

    /// Down-right from the cursor; flips to the opposite edge when the
    /// surface would leave the output (margins never go negative).
    #[test]
    fn menu_placement_opens_down_right_and_flips_on_overflow() {
        let (anchor, m) = menu_placement(100, 50, 200, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::Left | Anchor::Top);
        assert_eq!((m.left, m.top, m.right, m.bottom), (100, 50, 0, 0));

        // Near the right edge → the menu opens to the cursor's left…
        let (anchor, m) = menu_placement(1850, 50, 200, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::Right | Anchor::Top);
        assert_eq!((m.right, m.left), (1920 - 1850, 0));

        // …near the bottom → above it; an oversized surface is cut at
        // the output edge (flush with the bottom-right corner), not
        // pinned at the cursor.
        let (anchor, m) = menu_placement(100, 1050, 200, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::Left | Anchor::Bottom);
        assert_eq!((m.bottom, m.top), (1080 - 1050, 0));

        let (anchor, m) = menu_placement(10, 10, 4000, 4000, 1920, 1080);
        assert_eq!(anchor, Anchor::Right | Anchor::Bottom);
        assert_eq!((m.right, m.bottom), (0, 0));

        // Regression: the 116-row Cachy-Update menu under a top bar (at
        // = the tray icon, h ≈ 2332 px) used to open with its base at
        // the cursor and 2300 px above the screen — only a sliver at
        // the bar was visible. It now hangs from the right edge and is
        // cut at the screen bottom.
        let (anchor, m) = menu_placement(1700, 14, 240, 2332, 1920, 1080);
        assert_eq!(anchor, Anchor::Right | Anchor::Bottom);
        assert_eq!((m.right, m.bottom), (1920 - 1700, 0));

        // Mid-screen cursor: a menu too tall to open downward flips up
        // and stays fully visible (clamped to the top edge, not to the
        // cursor).
        let (anchor, m) = menu_placement(100, 500, 200, 600, 1920, 1080);
        assert_eq!(anchor, Anchor::Left | Anchor::Bottom);
        assert_eq!((m.bottom, m.top), (1080 - 600, 0));
    }

    /// The surface really lands shifted past the bar's reservation on
    /// the shared edge only — the calendar popup's pointer tests and
    /// the graph link both rely on this math: a top bar pushes a
    /// top-anchored popup down by the zone (the intended rect then
    /// contains the cursor while the surface does not); mismatched
    /// edges leave the placement untouched.
    #[test]
    fn surface_rect_shifts_only_the_shared_edge() {
        let rect = (100, 15, 397, 167);
        assert_eq!(
            shifted_surface_rect(rect, Anchor::Left | Anchor::Top, Anchor::Top, 30),
            (100, 45, 397, 197),
            "top bar + top popup: +30 on y, none on x"
        );
        assert_eq!(
            shifted_surface_rect(rect, Anchor::Left | Anchor::Top, Anchor::Bottom, 30),
            rect,
            "different edges: no shift"
        );
        assert_eq!(
            shifted_surface_rect(rect, Anchor::Right | Anchor::Bottom, Anchor::Bottom, 28),
            (100, -13, 397, 139),
            "bottom bar + bottom popup: -28 on y"
        );
    }

    /// The calendar popup's surface adapts to the panel font: doubling
    /// `panel.font_size` must grow both dimensions strictly (the cells
    /// fit the numbers and labels they show, the rows follow the size).
    #[test]
    fn calendar_size_follows_the_font() {
        let weekdays = ["lun", "mar", "mer", "gio", "ven", "sab", "dom"].map(String::from);
        let mut cfg = crate::config::default_config();
        cfg.panel.font_size = Some(12);
        let small = calendar_size(&cfg, &weekdays);
        cfg.panel.font_size = Some(24);
        let large = calendar_size(&cfg, &weekdays);
        assert!(large.0 > small.0 && large.1 > small.1, "{small:?} vs {large:?}");
        // With the default style the surface holds seven columns of at
        // least two characters plus padding — never a degenerate box.
        assert!(small.0 >= 7 * (12 + 2 * 6), "narrow: {:?}", small);
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

    /// Roadmap2 #1: the sync plan opens one window per wanted output,
    /// keeps the ones already placed there and closes the rest — an
    /// empty wanted list (feature off) closes everything.
    #[test]
    fn wallpaper_plan_opens_keeps_and_closes() {
        let current = vec![("HDMI-A-2".to_string(), 1u32), ("GONE-1".to_string(), 2u32)];
        let desired = vec!["HDMI-A-2".to_string(), "eDP-1".to_string()];
        let (open, close) = wallpaper_plan(&current, &desired);
        assert_eq!(open, vec!["eDP-1"]);
        assert_eq!(close, vec![2]);

        let (open, close) = wallpaper_plan(&current, &[]);
        assert!(open.is_empty());
        assert_eq!(close, vec![1, 2]);
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

    /// The panel background follows the active palette's base tone,
    /// keeping the configured colour's transparency — and the configured
    /// colour itself when no palette is active.
    #[test]
    fn panel_background_follows_the_palette() {
        let opaque: config::Config = serde_json::from_str(
            r##"{"panel":{"sections":[],"default_bg_color":"#000000ff"}}"##,
        )
        .unwrap();
        let pal = palette::Palette {
            bg: "#481919".to_string(),
            card: "#481919".to_string(),
            card_hover: "#8a3a3a".to_string(),
            text: "#e0d2d2".to_string(),
            text_hover: "#f6f4f4".to_string(),
        };
        assert_eq!(
            panel_background(&opaque, Some(&pal)),
            config::parse_color("#481919").unwrap(),
            "an opaque configured bar renders the palette's hue opaque"
        );
        assert_eq!(
            panel_background(&opaque, None),
            config::parse_color("#000000ff").unwrap()
        );
        let clear: config::Config = serde_json::from_str(
            r##"{"panel":{"sections":[],"default_bg_color":"#00000000"}}"##,
        )
        .unwrap();
        assert_eq!(
            panel_background(&clear, Some(&pal)),
            config::parse_color("#48191900").unwrap(),
            "a transparent configured bar stays see-through in the hue"
        );
    }

    /// The tick reaps a popup whose sensor and surface both lost the
    /// cursor — but only after the grace, and never while a sensor is
    /// under the cursor or the pointer sits on the surface. The flag
    /// is two-way: a pointer that visits and then leaves falls back to
    /// the grace instead of pinning the popup open forever.
    #[test]
    fn an_abandoned_popup_is_reaped_after_its_grace() {
        let fresh = || GraphPopup {
            id: iced::window::Id::unique(),
            section: 0,
            sensor: 0,
            rect: (0, 0, 10, 10),
            anchor: Anchor::Top,
            sensor_rect: None,
            pointer_on_popup: false,
            missed: 0,
        };
        let mut abandoned = fresh();
        assert!(
            !popup_missed_tick(&mut abandoned, false),
            "the first tick only grants grace"
        );
        assert!(
            popup_missed_tick(&mut abandoned, false),
            "the second tick reaps it"
        );

        let mut on_surface = fresh();
        on_surface.pointer_on_popup = true;
        assert!(
            !popup_missed_tick(&mut on_surface, false),
            "pointer on it: never abandoned"
        );

        let mut visited = fresh();
        visited.pointer_on_popup = true;
        assert!(
            !popup_missed_tick(&mut visited, false),
            "pointer on it: never abandoned"
        );
        visited.pointer_on_popup = false;
        assert!(
            !popup_missed_tick(&mut visited, false),
            "the pointer left: the first tick only grants grace"
        );
        assert!(
            popup_missed_tick(&mut visited, false),
            "the pointer left: the second tick reaps it"
        );

        let mut on_sensor = fresh();
        on_sensor.missed = 1;
        assert!(!popup_missed_tick(&mut on_sensor, true), "still hovered");
        assert_eq!(on_sensor.missed, 0, "a hover resets the count");
    }
}
