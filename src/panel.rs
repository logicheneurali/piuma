//! The bar's data model: the readings the view draws, the shared panel
//! state, the app wrapper and the message enum the daemon routes.
//!
//! The types live here (not in `main.rs`) so every module can import them
//! without depending on the crate root; `main.rs` re-exports them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use iced::Color;
use iced_exwlshell::reexport::Anchor;
use iced_exwlshell::settings::LayerSize;


use crate::hover::HoverGate;
use crate::menus::TaskbarRow;
use crate::{
    config, dashboard, dbusmenu, graphs, hypr, layouts, mpris, osd, palette, popup_anim, screensaver, tray, window_menu, windows, workspace,
};

#[derive(Clone)]
pub(crate) struct SensorValue {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) value: String,
    /// Opted into `panel.value_effect`; the view reserves the effect
    /// layer only for these sensors.
    pub(crate) effect: bool,
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
    /// The selected MPRIS player when the source is a `media` sensor
    /// (`None` otherwise); the view draws its transport buttons and the
    /// hover popup from it.
    pub(crate) media: Option<mpris::MediaState>,
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
    pub(crate) sensors: Vec<SensorStyle>,
}

/// The sensor hover's dwell gate: an enter is only *armed* (with a
/// fresh sequence number), and it activates when — and only if — its
/// confirmation arrives while still pending: a leave on that sensor,
/// or a newer enter, drops it first. This is what gives the hover its
/// delay — a pointer sweeping past a sensor fires no style and opens
/// no popup.
pub(crate) struct Panel {
    /// The effective config: the user's file with the active theme
    /// applied over its colour keys (in memory only).
    pub(crate) config: config::Config,
    /// The config exactly as the user wrote it: every theme change
    /// re-applies over this copy, so switching to "custom" restores the
    /// user's own colours and no theme value leaks into `piuma.json`.
    pub(crate) config_base: config::Config,
    /// Slug of the active theme (`theme.json`), `None` = custom colours.
    pub(crate) active_theme: Option<&'static str>,
    /// Change stamp of `theme.json` seen on the last tick: the bar only
    /// watches that file, it never writes it.
    pub(crate) theme_state: Option<(SystemTime, u64)>,
    pub(crate) sections: Vec<SectionData>,
    /// Every section's and sensor's resolved style (`resolve_styles`),
    /// rebuilt on config reload and on wallpaper-palette change — the
    /// tick pairs readings with it instead of re-resolving colours.
    pub(crate) styles: Vec<SectionStyle>,
    /// True while the tick's data pass runs on a worker thread: one
    /// read at a time, so replies can never arrive out of order.
    pub(crate) read_pending: bool,
    /// When the running pass started and the token it answers with: a
    /// pass abandoned by the watchdog is dropped by token.
    pub(crate) read_started: Option<Instant>,
    pub(crate) read_pass: u64,
    /// No new pass before this instant (after a stuck one).
    pub(crate) read_backoff: Option<Instant>,
    /// Bumped on every config reload: a reply built against an older
    /// config no longer lines up with the sensor list and is dropped.
    pub(crate) cfg_gen: u64,
    /// Set when geometry or config moved in the tick that spawned the
    /// current read; consumed with the fresh sections, so the popup
    /// re-anchors on them (`apply_read` → `sync_popup`).
    pub(crate) popup_resync: bool,
    /// Index into `sections[s]` and `sections[s].sensors[sensor]` under the cursor.
    pub(crate) hovered: Option<(usize, usize)>,
    /// The dwell gate behind `hovered`: enters arm here and reach it
    /// only after `SENSOR_HOVER_DELAY` while still pending.
    pub(crate) hover_gate: HoverGate,
    /// When the pointer last moved over the bar: a hover whose arm has no
    /// motion next to it came from a widget whose bounds changed under a
    /// still pointer (see `hover_is_fresh`).
    pub(crate) last_motion: Option<Instant>,
    /// The hover waiting for its sensor box (see `FindSensorRect`): set at
    /// confirmation, cleared by the answer or by any newer enter/leave.
    pub(crate) pending_hover: Option<(usize, usize)>,
    /// State (mtime, len) of the last config read attempt: unchanged files
    /// are skipped on the next tick instead of being re-read and parsed.
    pub(crate) last_reload: Option<(SystemTime, u64)>,
    /// The first panel's own anchor (`panels[0].anchor`, or the legacy
    /// `panel.anchor`): the tick recomputes that panel's geometry and
    /// orientation from it.
    pub(crate) first_anchor: Option<String>,
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
    /// Popups sliding out: their state was cleared, the window keeps
    /// rendering this ghost until the close animation ends.
    pub(crate) closing: Vec<(iced::window::Id, popup_anim::Ghost)>,
    /// Card count the popup surface is currently sized for.
    pub(crate) popup_cards: usize,
    /// Card under the cursor (`hover_*` style fields), `None` outside.
    pub(crate) popup_hover: Option<u32>,
    /// Per-card scroll viewport (absolute offset, max offset) of the
    /// notification body; drives the two nav arrows instead of a rail.
    pub(crate) notif_scroll: HashMap<u32, (f32, f32)>,
    /// Value-change effects currently alive: (section, sensor) → start.
    /// Only opted sensors enter this map; empty means no animation.
    pub(crate) value_effects: HashMap<(usize, usize), Instant>,
    /// Volume meter under a drag: (section, sensor, target percent).
    /// The sink is written only on release, so a tick in between must
    /// not replace the bar with the old reading.
    pub(crate) volume_drag: Option<(usize, usize, f32)>,
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
    /// The floating window-actions menu (`panel.tweaks.window_menu`):
    /// its surface and the window it follows, absent while the tweak is
    /// off.
    pub(crate) window_menu: Option<window_menu::WindowMenu>,
    /// Row under the cursor in the taskbar menu (`TaskbarMenuHover`),
    /// `None` outside every row.
    pub(crate) taskbar_menu_hover: Option<TaskbarRow>,
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

    /// Open calendar popup of a hovered `calendar` sensor: surface id,
    /// the sensor it belongs to, the shown month and its pointer/grace
    /// state, `None` while hidden.
    pub(crate) calendar: Option<CalendarPopup>,
    /// Wallpaper windows (roadmap2 #1): one background surface per
    /// active output, keyed by the compositor's output name. They stay
    /// open when the wallpaper is switched off (see `wallpaper_hidden`).
    pub(crate) wallpaper: Vec<(String, iced::window::Id)>,
    /// Wallpaper surfaces hidden while the feature is off: closing them
    /// makes the renderer/driver retain the decoded image (~8 MB per
    /// output), so the window stays alive and only its view goes empty.
    pub(crate) wallpaper_hidden: Vec<iced::window::Id>,
    /// Rounded-screen-corner masks (roadmap angoli-schermo): one square
    /// overlay surface per output corner, keyed "output:corner:radius";
    /// empty while the feature is off.
    pub(crate) corners: Vec<(String, iced::window::Id)>,
    /// Idle screensaver surfaces, one per output, keyed by output name:
    /// empty while the overlay is down.
    pub(crate) screensaver: Vec<(String, iced::window::Id)>,
    /// Whether the overlay is up (the app decides; the idle event only
    /// starts the fullscreen check).
    pub(crate) screensaver_on: bool,
    /// When the overlay opened: the animation clock and the input grace.
    pub(crate) screensaver_start: Option<Instant>,
    /// Idle arrived while the focused window is fullscreen: checked
    /// again every minute until it clears.
    pub(crate) screensaver_wait: bool,
    /// Next fullscreen check while `screensaver_wait`.
    pub(crate) screensaver_retry: Option<Instant>,
    /// Every cell's palette phase, row-major (`COLS`×`ROWS`): the frame
    /// message advances it (see `screensaver::advance`).
    pub(crate) screensaver_phases: Vec<f32>,
    /// When the phases last advanced: the frame's dt.
    pub(crate) screensaver_last: Option<Instant>,
    /// The password field's text while the lock screen is up (never
    /// logged, cleared after every attempt).
    pub(crate) screensaver_password: String,
    /// The last failed attempt's message, `None` while none failed.
    pub(crate) screensaver_error: Option<String>,
    /// A PAM check is running: further submits are ignored.
    pub(crate) screensaver_pending: bool,
    /// How the lock is held: the session-lock protocol when the
    /// compositor has it, the layer overlay otherwise.
    pub(crate) screensaver_lock: screensaver::LockMode,
    /// The runtime's surface events: the session-lock windows it creates.
    pub(crate) shell: iced_exwlshell::shell::ShellReceiver,
    /// Whether the password card is on screen: it appears on the first
    /// input (a click, a key), never by itself — the idle lock shows
    /// just the grid.
    pub(crate) screensaver_prompt_open: bool,
    /// The `caffeine` sensors' state: while on, the idle lock stays away.
    pub(crate) caffeine: bool,
    /// The extra bar surfaces (panel 2 and later) with their own anchor:
    /// the same window→layout mapping the daemon keeps, reachable from
    /// the tick.
    pub(crate) bar_windows: Vec<(iced::window::Id, Option<String>)>,
    /// Whether the glass tweak was applied to the surfaces: a reload
    /// toggles `BlurOptionChange` only on a real change.
    pub(crate) last_glass: bool,
    /// The open power menu surface and its auto-close deadline.
    pub(crate) power_menu: Option<(iced::window::Id, Instant)>,
    /// The open theme picker surface and its auto-close deadline.
    pub(crate) theme_menu: Option<(iced::window::Id, Instant)>,
    /// The open dashboard overlay, `None` while closed.
    pub(crate) dashboard: Option<dashboard::Dashboard>,
    /// The dashboard's snapshot, read only while the card is open.
    pub(crate) dash_probe: Option<dashboard::Probe>,
    /// The profile the next power-profile click selects: shown at once,
    /// cleared when the next probe confirms the real value.
    pub(crate) dash_profile: Option<String>,
    /// Slider targets while dragging (and until the next probe lands).
    pub(crate) dash_volume: Option<f32>,
    pub(crate) dash_brightness: Option<f32>,
    /// A drag is in flight: a probe arriving meanwhile must not snap the
    /// knob back to the old reading.
    pub(crate) dash_volume_drag: bool,
    pub(crate) dash_brightness_drag: bool,
    /// Decoded cover art for `cover_url`.
    pub(crate) cover: Option<iced::widget::image::Handle>,
    /// The art URL the last decode was requested for: a failed or
    /// missing file is not retried on every tick.
    pub(crate) cover_url: Option<String>,
    /// Sections collapsed to their dot handle (runtime only: the dot is
    /// the toggle, and the bar lays out only the dot meanwhile).
    pub(crate) collapsed_sections: std::collections::HashSet<usize>,
    /// The volume/brightness OSD card while it lingers, `None` otherwise.
    pub(crate) osd: Option<osd::Osd>,
    /// Colours derived from the wallpaper (`panel.wallpaper_palette`):
    /// the active dark/light variant the bar draws with. Runtime-only —
    /// never written back to the config file.
    pub(crate) palette: Option<palette::Palette>,
    /// The extracted dark seed: `palette` is its active variant (light
    /// when `panel.theme_mode` asks for it).
    pub(crate) palette_seed: Option<palette::Palette>,
    /// The wallpaper file (path + mtime) the current palette was
    /// extracted from: the extraction re-runs only when this changes.
    pub(crate) palette_key: Option<(String, SystemTime)>,
    /// The last pass's shared compositor snapshot: the layout engine's
    /// save (click) reads the windows from it without forking again.
    pub(crate) hypr: Option<hypr::Snapshot>,
    /// The layout engine's session state (roadmap layout-automatici):
    /// the layout file cache, the window set of the last activation
    /// check, which layouts already applied and the "✓ saved" flash
    /// clock.
    pub(crate) layout_state: layouts::LayoutState,
}

/// The runtime root: one shared `Panel` state (sections, readings, tray,
/// notifications) and one window per configured layout. The layouts only
/// choose output/placement and which named sections each window draws;
/// section definitions stay global, so the same reading feeds every bar.
pub(crate) struct App {
    pub(crate) state: Panel,
    pub(crate) layouts: Vec<config::PanelLayout>,
    /// Logical size of each layout's output (used for popup geometry).
    pub(crate) screens: Vec<(u32, u32)>,
    /// Layershell window → layout index. The initial window (created by
    /// the daemon) is not in the map and defaults to layout 0.
    pub(crate) windows: HashMap<iced::window::Id, usize>,
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
    /// First visible entry; the wheel moves it.
    pub(crate) offset: u32,
    /// Rows the surface shows at once (bounded by the output height).
    pub(crate) visible: usize,
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
    /// Floating/pinned as the snapshot knew them when the menu opened:
    /// Sticky floats a tiled window before pinning it.
    pub(crate) floating: bool,
    pub(crate) pinned: bool,
}

/// The open bar tooltip window: its source (so a stale leave knows
/// whether it still owns the surface) and the fitted text shown.
#[derive(Clone, Debug)]
pub(crate) struct BarTooltip {
    pub(crate) id: String,
    pub(crate) text: String,
    /// When the surface opened: `tooltip::sync` retires it after
    /// `TOOLTIP_TTL`, so a missed mouse exit can never pin it forever.
    pub(crate) opened_at: std::time::Instant,
    /// The sensor whose hover owns this popup (`hover_sync`'s list and
    /// media popups): while it stays hovered the TTL is refreshed, so a
    /// long text is readable; once the hover ends the TTL runs again.
    pub(crate) owner: Option<(usize, usize)>,
    /// Played fraction (0..=1) drawn as a bar under the text; `None` =
    /// text only (every tooltip but the media hover's).
    pub(crate) progress: Option<f32>,
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
