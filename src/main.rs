//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.
//!
//! Configuration is loaded from a JSON file (CLI arg or default path).
//! See `src/config.rs` for the config schema.

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

mod config;
mod notifications;
mod sensor_runner;
mod sensors;
mod tray;

// ── globals ────────────────────────────────────────────────────────

static mut CONFIG: Option<config::Config> = None;
static mut CONFIG_PATH: Option<String> = None;
static mut PANEL_SHOULD_CLOSE: bool = false;
static mut SCREEN_WIDTH: u32 = 1920;
static mut SCREEN_HEIGHT: u32 = 1080;

// ── colours / defaults ────────────────────────────────────────────

const DEFAULT_FONT: &str = "Noto Sans";
const DEFAULT_FONT_SIZE: u32 = 14;
const DEFAULT_HEIGHT: u32 = 32;
/// Padding the section content Row/Column adds on every side: used by the
/// layout below and by the tray icon size formula (single source, the
/// formula must match what the view really draws).
const SECTION_CONTENT_PAD: u16 = 4;
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

// ── model ───────────────────────────────────────────────────────────

#[derive(Clone)]
struct SensorValue {
    name: String,
    description: String,
    value: String,
    text_color: Option<iced::Color>,
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
    weight: f32,
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
}

// ── messages ────────────────────────────────────────────────────────

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Tick(Instant),
    IcedEvent(Event),
    /// A sensor entered/exited the hover state: (section_index, sensor_index, hovered).
    SensorHovered(usize, usize, bool),
    /// A sensor was clicked: (section_index, sensor_index, action).
    SensorPressed(usize, usize, Option<String>),
    /// Wheel scrolled over a sensor: (section_index, sensor_index, up);
    /// `None` for scroll events that carry no usable delta.
    SensorScrolled(usize, usize, Option<bool>),
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

// ── helpers ─────────────────────────────────────────────────────────

fn resolve_color<'a>(cfg: Option<Color>, default: Color) -> Color {
    cfg.unwrap_or(default)
}

fn text_color_style(color: Color) -> impl Fn(&iced::Theme) -> ContainerStyle {
    move |_theme: &iced::Theme| ContainerStyle {
        text_color: Some(color),
        ..ContainerStyle::default()
    }
}

fn container_with_bg<'a>(
    child: Element<'a, Message>,
    bg_color: Color,
) -> Element<'a, Message> {
    container(child)
        .width(Length::Shrink)
        .height(Length::Shrink)
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(bg_color)),
            ..ContainerStyle::default()
        })
        .into()
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
    padding: u16,
    width: Length,
    height: Length,
) -> Element<'a, Message> {
    // Only these types bypass the native uniform border.
    let partial = matches!(
        border_type,
        "underline" | "upperline" | "left" | "right" | "corners"
    );

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
                    style.border.width = border_width as f32;
                    style.border.radius = (border_radius as f32).into();
                }
            }
            style
        });

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

    let bw_px = Length::Fixed(bw as f32);

    // Positions a border bar anywhere inside the box bounds.
    let place = |bar: Element<'a, Message>, x: Alignment, y: Alignment| -> Element<'a, Message> {
        container(bar)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(x)
            .align_y(y)
            .into()
    };

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
    let path = unsafe { CONFIG_PATH.clone()? };
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Hot reload of the config file: returns the parsed config only when the
/// file changed on disk since the last attempt (mtime + length check, so an
/// unchanged file is not re-read every tick). The attempt is recorded
/// regardless of the outcome; a parse failure (e.g. a half-written file)
/// logs once per change and keeps the previous config, retrying when the
/// file changes again.
fn maybe_reload(panel: &mut Panel) -> Option<config::Config> {
    let state = config_file_state()?;
    if panel.last_reload == Some(state) {
        return None;
    }
    let path = unsafe { CONFIG_PATH.clone()? };
    panel.last_reload = Some(state);

    let contents = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<config::Config>(&contents) {
        Ok(cfg) => {
            eprintln!("[piuma] config reloaded: {}", path);
            Some(cfg)
        }
        Err(e) => {
            eprintln!("[piuma] config changed but failed to parse ({}), keeping previous", e);
            None
        }
    }
}

// ── update ──────────────────────────────────────────────────────────

/// Applies a message to the panel. `timed`'s `Instant` parameter was dropped
/// in the migration to the `daemon` build pattern (multi-surface): each
/// `Tick` already carries its own `Instant`.
fn update(panel: &mut Panel, message: Message) -> Command<Message> {
    match message {
        Message::Tick(_) => {
            // Hot reload: pick up config file changes (content and, when
            // layout/align differ from what is applied, surface geometry).
            let mut geometry_changed = false;
            let mut zone_changed = false;
            // Any reload re-anchors/resizes the popup too: the popup style
            // (font/padding) lives in the config without affecting the bar's
            // own geometry, so its reposition rides on this flag.
            let mut config_reloaded = false;
            if let Some(reloaded) = maybe_reload(panel) {
                config_reloaded = true;
                panel.config = reloaded;
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

            let mut sections: Vec<SectionData> = Vec::new();
            for section_cfg in panel.config.panel.sections.iter() {
                let section_default_txt = panel.config.panel.default_text_color
                    .as_ref()
                    .map(|s| config::parse_color(s).ok())
                    .unwrap_or(Some(DEFAULT_TEXT_COLOR))
                    .unwrap_or(DEFAULT_TEXT_COLOR);

                // Text colour chain: section > section_defaults > panel > built-in.
                let section_text_color = section_cfg.text_color.as_ref()
                    .and_then(|s| config::parse_color(s).ok())
                    .or_else(|| {
                        panel.config.panel.section_defaults.as_ref()
                            .and_then(|sd| sd.text_color.as_ref())
                            .and_then(|s| config::parse_color(s).ok())
                    })
                    .unwrap_or(section_default_txt);

                let sensors: Vec<SensorValue> = section_cfg
                    .sensors
                    .iter()
                    .map(|s| {
                        let value = sensor_runner::run(&s.source);
                        // Tray sensors render icons from the shared tray state
                        // (tray.rs) instead of text.
                        let tray_icons =
                            matches!(s.source, config::SensorSource::Tray {}).then(tray::items);
                        // Style precedence, per style GROUP: a sensor takes a
                        // group over only when it defines the group's key
                        // colour (`border_color`, `bg_color`, `text_color`).
                        // When that key is null on the sensor, every override
                        // of that group on the sensor is ignored and
                        // `panel.sensor_defaults` applies as a whole. When the
                        // key is set, the sensor's own fields win and the
                        // remaining ones fall back to the defaults.
                        // `padding` is a single value: sensor when set,
                        // otherwise the default.
                        let defaults = panel.config.panel.sensor_defaults.as_ref();
                        let own_border = s.border_color.is_some();
                        let own_bg = s.bg_color.is_some();
                        let own_text = s.text_color.is_some();
                        // Key fields: per-sensor, then defaults, then section/built-in.
                        let tc_str = s.text_color.as_deref().or(defaults.and_then(|sd| sd.text_color.as_deref()));
                        let bg_str = s.bg_color.as_deref().or(defaults.and_then(|sd| sd.bg_color.as_deref()));
                        let bc_str = s.border_color.as_deref().or(defaults.and_then(|sd| sd.border_color.as_deref()));
                        // Secondary fields only count when the sensor owns the group.
                        let bg_h_str = (if own_bg { s.bg_color_hover.as_deref() } else { None })
                            .or(defaults.and_then(|sd| sd.bg_color_hover.as_deref()));
                        let tc_h_str = (if own_text { s.text_color_hover.as_deref() } else { None })
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
                        SensorValue {
                            // static_text and tray sensors render only their
                            // content: the name stays in the config (setup
                            // keeps editing it) but never appears on the bar.
                            name: if matches!(
                                s.source,
                                config::SensorSource::StaticText { .. }
                                    | config::SensorSource::Tray {}
                            ) {
                                String::new()
                            } else {
                                s.name.clone()
                            },
                            description: s.description.as_ref().map(|d| d.to_string()).unwrap_or_default(),
                            value,
                            text_color: Some(sensor_tc),
                            bg_color: sensor_bg,
                            text_color_hover: sensor_tc_hover,
                            bg_color_hover: sensor_bg_hover,
                            border_color: sensor_border,
                            border_width: bw_val as u16,
                            border_radius: br_val as u16,
                            border_type: bt_str.to_string(),
                            padding: config::style_num(s.padding)
                                .or(defaults.and_then(|sd| config::style_num(sd.padding)))
                                .unwrap_or(0) as u16,
                            action: s.action.clone(),
                            tray: tray_icons,
                        }
                    })
                    .collect();

                let default_bg = panel.config.panel.default_bg_color
                    .as_ref()
                    .map(|s| config::parse_color(s).ok())
                    .unwrap_or(Some(DEFAULT_BG_COLOR))
                    .unwrap_or(DEFAULT_BG_COLOR);

                // Section style precedence, per style GROUP (the same rule as
                // `panel.sensor_defaults`): the border group belongs to the
                // section only when it sets `border_color`; otherwise the
                // whole group comes from `panel.section_defaults`. bg/text
                // are single keys: section value when set, else the default.
                let sdefaults = panel.config.panel.section_defaults.as_ref();
                let own_border = section_cfg.border_color.is_some();
                let bg_str = section_cfg.bg_color.as_deref()
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
                    weight: panel.config.panel.default_section_weight.as_ref().map(|w| (*w as u32) as f32).unwrap_or(1.0),
                    sensors,
                });
            }
            panel.sections = sections;
            // Icons size to the space their sensor box really has; both the
            // view and the icon resolution read tray::icon_target.
            update_tray_icon_target(panel);
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
            // The popup follows the active set (and re-anchors with the
            // bar on a geometry change) — one place drives its window.
            if let Some(cmd) = sync_popup(panel, geometry_changed || config_reloaded) {
                cmds.push(cmd);
            }
            if cmds.is_empty() { Command::none() } else { Command::batch(cmds) }
        }
        Message::IcedEvent(e) => {
            // Mouse enter/leave/press are handled per-sensor by MouseArea,
            // which emits Message::SensorHovered / SensorPressed. The cursor
            // position itself is remembered for tray activation hints.
            if let Event::Mouse(iced::mouse::Event::CursorMoved { position }) = e {
                panel.last_cursor = Some((position.x, position.y));
            }
            Command::none()
        }
        Message::TrayItemsChanged => {
            // An item appeared/vanished/was updated: rebuild the sections at
            // once instead of waiting for the next tick (same full path).
            update(panel, Message::Tick(Instant::now()))
        }
        Message::NotificationsChanged => {
            // A notification arrived, was replaced, expired or closed:
            // rebuild at once (same full path as the tray poke).
            update(panel, Message::Tick(Instant::now()))
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
            // Foreground thread only queues the D-Bus call (tray.rs spawns
            // its own thread), so the UI never blocks on the item.
            if let Some((x, y)) = tray::hint_point(panel) {
                tray::spawn_action(&bus, &path, button, x, y);
            }
            Command::none()
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
                    // click runs the shown entry's spec `default` action
                    // (emitted as `ActionInvoked`, other sensors ignore it).
                    let is_notifications = panel
                        .config
                        .panel
                        .sections
                        .get(section_index)
                        .and_then(|sec| sec.sensors.get(sensor_index))
                        .is_some_and(|s| matches!(s.source, config::SensorSource::Notifications {}));
                    if is_notifications {
                        notifications::invoke_default();
                    }
                }
            }
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
        Message::SensorHovered(section_index, sensor_index, hovered) => {
            if hovered {
                panel.hovered = Some((section_index, sensor_index));
            } else if panel.hovered == Some((section_index, sensor_index)) {
                panel.hovered = None;
            }
            Command::none()
        }
        _ => Command::none(),
    }
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
            time::every(std::time::Duration::from_secs(1)).map(|now| Message::Tick(now)),
            event::listen().map(Message::IcedEvent),
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

/// Configured family name kept alive for [`Font::Name`]'s `'static` bound:
/// leaked once per name actually used, so reloads don't accumulate leaks.
static FONT_NAME: std::sync::Mutex<Option<(&'static str, String)>> =
    std::sync::Mutex::new(None);

/// The `'static` family name of the configured font (roadmap M2).
fn font_name_static(config: &config::Config) -> &'static str {
    let name = config.panel.font.as_deref().unwrap_or(DEFAULT_FONT);
    let mut slot = FONT_NAME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((leaked, original)) = slot.as_ref() {
        if original == name {
            return *leaked;
        }
    }
    let leaked = Box::leak(name.to_string().into_boxed_str());
    *slot = Some((leaked, name.to_string()));
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
    Font { weight, ..Font::with_name(font_name_static(config)) }
}

fn view<'a>(panel: &'a Panel, window: iced::window::Id) -> Element<'a, Message> {
    // The popup window renders its own tree; only bar calls feed BAR_ID.
    if panel.popup == Some(window) {
        return popup_view(panel);
    }
    let _ = BAR_ID.set(window);
    let font_obj = panel_font(&panel.config);
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
        // sensor with no items renders nothing at all (no empty box).
        let sensor_widgets: Vec<Element<'a, Message>> = section.sensors.iter().enumerate().filter_map(|(sensor_idx, s)| {
            if s.tray.as_ref().is_some_and(|icons| icons.is_empty()) {
                return None;
            }
            let sensor_text_color = s.text_color.as_ref().map(|c| *c).unwrap_or(section_text);

            // Text color: hover variant when the cursor is over this sensor.
            let text_color = if panel.hovered == Some((section_idx, sensor_idx)) {
                s.text_color_hover.unwrap_or_else(|| s.text_color.unwrap_or(section_text))
            } else {
                sensor_text_color
            };

            let value_widget = text(&s.value)
                .color(text_color)
                .size(font_size)
                .font(font_obj);

            // Name and value side by side, separated by a small gap (the
            // colon separator was removed on request); an empty name (e.g.
            // static_text sensors) renders the value alone, with no gap.
            // Tray sensors skip the text row: `tray::row` builds their
            // icon row with clicks, wheel and tooltip included. The style
            // passed here drives the tooltip, which mirrors the sensor box;
            // a transparent sensor shows the section behind it on the bar,
            // so the tooltip borrows that colour to stay legible anywhere.
            let main: Element<'a, Message> = if let Some(icons) = &s.tray {
                tray::row(
                    icons,
                    font_size,
                    font_obj,
                    &tray::Style {
                        bg_color: s.bg_color.or(Some(section_bg)),
                        text_color: s.text_color,
                        border_color: s.border_color,
                        border_width: s.border_width,
                        border_radius: s.border_radius,
                        border_type: s.border_type.clone(),
                        padding: s.padding,
                    },
                )
            } else if s.name.is_empty() {
                Row::new().push(value_widget).into()
            } else {
                Row::new()
                    .push(text(&s.name).color(text_color).size(font_size).font(font_obj))
                    .push(value_widget)
                    .spacing(6)
                    .into()
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
            let pad = s.padding;

            let styled_container = styled_box(
                content,
                bg_color,
                border_color,
                s.border_width,
                s.border_radius,
                &s.border_type,
                pad,
                box_w,
                box_h,
            );

            // Tray icons carry their own MouseAreas (press/wheel/tooltip);
            // wrapping them in the sensor MouseArea as well would handle the
            // same events twice, so a tray sensor stays bare.
            if s.tray.is_some() {
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

    let panel_bg = panel.config.panel.default_bg_color
        .as_ref()
        .and_then(|s| config::parse_color(s).ok())
        .unwrap_or(DEFAULT_BG_COLOR);

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
    config.panel.thickness.unwrap_or(DEFAULT_HEIGHT).max(1)
}

/// Icon edge (px) that fits inside a tray sensor box: panel thickness minus
/// everything the layout draws on the thickness axis inside it — section
/// border and content padding on both sides, the section title when sensors
/// are stacked vertically (`title_extra`), and the sensor's own padding and
/// border — clamped to a usable range so a hairline bar and a very thick
/// bar both stay sane.
fn tray_icon_target(
    thickness: u32,
    section_border: u32,
    title_extra: u32,
    padding: u32,
    border: u32,
) -> u16 {
    let chrome =
        2 * (section_border + u32::from(SECTION_CONTENT_PAD) + padding + border) + title_extra;
    thickness.saturating_sub(chrome).clamp(4, 48) as u16
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
        // Title height as the view draws it: text (font + 2) in a box
        // padded SECTION_CONTENT_PAD on both sides.
        let title_extra = if section.layout == "vertical" && !section.name.is_empty() {
            font + 2 + 2 * u32::from(SECTION_CONTENT_PAD)
        } else {
            0
        };
        for sensor in &section.sensors {
            if sensor.tray.is_some() {
                let target = tray_icon_target(
                    thickness,
                    u32::from(section.border_width),
                    title_extra,
                    u32::from(sensor.padding),
                    u32::from(sensor.border_width),
                );
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
/// `border_type` to `full` (the `styled_box` rule).
fn popup_style(config: &config::Config) -> PopupStyle {
    let d = PopupStyle::default();
    let Some(n) = config.panel.notifications.as_ref() else {
        return d;
    };
    let color = |key: &Option<String>, fallback: Color| {
        key.as_deref()
            .and_then(|s| config::parse_color(s).ok())
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
        text: color(&n.text_color, d.text),
        text_hover: color(&n.text_color_hover, d.text),
        bg: color(&n.bg_color, d.bg),
        bg_hover: color(&n.bg_color_hover, d.bg),
        border: color(&n.border_color, d.border),
        border_hover: color(&n.border_color_hover, d.border),
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

fn sync_popup(panel: &mut Panel, reposition: bool) -> Option<Command<Message>> {
    let enabled = panel.config.panel.notifications.is_some();
    let cards = if enabled {
        notifications::active_cards(POPUP_MAX_CARDS).len()
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
            let (id, open) = Message::layershell_open(NewLayerShellSettings {
                size: popup_size(panel, n),
                layer: Layer::Overlay,
                anchor: popup_anchor(&panel.config),
                exclusive_zone: None,
                margin: Some(Margin {
                    top: POPUP_MARGIN,
                    right: POPUP_MARGIN,
                    bottom: 0,
                    left: 0,
                }),
                keyboard_interactivity: KeyboardInteractivity::None,
                output_option: OutputOption::LastOutput,
                namespace: Some("piuma-notifications".to_string()),
                ..Default::default()
            });
            panel.popup = Some(id);
            panel.popup_cards = n;
            Some(open)
        }
        (Some(id), n) if reposition || n != panel.popup_cards => {
            panel.popup_cards = n;
            Some(Command::done(Message::LayoutChange {
                id,
                anchor: popup_anchor(&panel.config),
                size: popup_size(panel, n),
            }))
        }
        _ => None,
    }
}

/// Surface size for a stack of `cards`: exact, so there is no dead
/// (click-blocking) surface below the last card. The card height comes
/// from the configured popup style (see `popup_card_h`).
fn popup_size(panel: &Panel, cards: usize) -> LayerSize {
    let n = cards as u32;
    let h = popup_card_h(&popup_style(&panel.config));
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
    let st = popup_style(&panel.config);
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
        // Icon: image-data wins, a real file path follows; anything else
        // (e.g. a themed icon name) renders without an icon.
        let icon: Option<Image> = match &card.icon {
            Some(notifications::CardIcon::Rgba { width, height, data }) => Some(
                Image::new(iced::widget::image::Handle::from_rgba(
                    *width as u32,
                    *height as u32,
                    data.clone(),
                ))
                .width(32)
                .height(32)
                .content_fit(iced::ContentFit::Contain),
            ),
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
    let _ = simple_logger::SimpleLogger::from_env()
        .with_utc_timestamps()
        .init();

    eprintln!("[piuma] Starting …");

    // Load configuration once at startup; remember the file it came from so
    // it can be watched for hot reload.
    let (config, resolved_config_path) = config::load_config(config_path.clone());

    eprintln!(
        "[piuma] Config loaded: {} section(s) from {}",
        config.panel.sections.len(),
        resolved_config_path.display()
    );

    // Best-effort primary monitor dimensions for screen-wide surface sizing.
    unsafe {
        SCREEN_WIDTH = active_screen_width().unwrap_or(1920);
        SCREEN_HEIGHT = active_screen_height().unwrap_or(1080);
    }

    unsafe { CONFIG = Some(config.clone()); }
    unsafe { CONFIG_PATH = Some(resolved_config_path.to_string_lossy().into_owned()); }

    let panel_config = unsafe { CONFIG.as_ref().cloned().unwrap_or_else(|| config::default_config()) };

    // Run the iced app on a dedicated thread so the close action can be
    // observed from the main thread without blocking the event loop.
    let handle = std::thread::Builder::new()
        .name("piuma-panel".to_string())
        .spawn(move || {
            daemon(
                || {
                    let cfg = unsafe { CONFIG.as_ref().map(|c| c.clone()).unwrap_or_else(|| config::default_config()) };
                    // Initial geometry (size, anchor, exclusive zone) must
                    // match what wl_settings applies at startup, so reloads
                    // only emit LayoutChange/ExclusiveZoneChange on change.
                    let size = panel_size(&cfg, unsafe { SCREEN_WIDTH }, unsafe { SCREEN_HEIGHT });
                    let anchor = panel_anchor(&cfg);
                    let zone = panel_thickness(&cfg) as i32;
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
            .run()
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
    use super::{popup_card_h, tray_icon_target, PopupStyle};

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

    /// A bare 26px bar: only the section content padding (4px a side).
    #[test]
    fn tray_icon_target_subtracts_section_padding() {
        assert_eq!(tray_icon_target(26, 0, 0, 0, 0), 18);
    }

    /// Section border + sensor padding/border, both sides, subtracted too.
    #[test]
    fn tray_icon_target_subtracts_chrome() {
        assert_eq!(tray_icon_target(44, 1, 0, 6, 2), 44 - 2 - 8 - 12 - 4);
    }

    /// A stacked section title eats thickness as well (font 14 → 14+2+8).
    #[test]
    fn tray_icon_target_subtracts_stacked_title() {
        assert_eq!(tray_icon_target(44, 0, 24, 0, 0), 44 - 8 - 24);
    }

    /// Clamped: never smaller than 4px, never larger than 48px.
    #[test]
    fn tray_icon_target_clamps() {
        assert_eq!(tray_icon_target(10, 0, 0, 0, 0), 4);
        assert_eq!(tray_icon_target(400, 0, 0, 0, 0), 48);
    }
}
