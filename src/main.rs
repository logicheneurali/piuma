//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.
//!
//! Configuration is loaded from a JSON file (CLI arg or default path).
//! See `src/config.rs` for the config schema.

use std::time::{Instant, SystemTime};

use iced::widget::{container, text, Column, Row, MouseArea, Space, Stack};
use iced::{Alignment, Background, Color, Element, Event, Length, Subscription, Task as Command};
use iced::event;
use iced::time;
use iced::font::Font;
use iced::widget::container::Style as ContainerStyle;

use iced_exwlshell::ExWlSettings;
use iced_exwlshell::layershell::timed;
use iced_exwlshell::reexport::{Anchor, Layer};
use iced_exwlshell::settings::{LayerShellSettings, LayerSize, StartMode};
use iced_exwlshell::to_layer_message;

mod config;
mod sensor_runner;
mod sensors;

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
const DEFAULT_BG_COLOR: Color = Color::from_rgba(0.06, 0.06, 0.08, 0.85);
const DEFAULT_TEXT_COLOR: Color = Color::from_rgb(0.88, 0.88, 0.94);

/// Internal action identifiers.
const ACTION_CLOSE_PANEL: &str = "close_panel";

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
}

/// Rendered section data.
#[derive(Clone)]
struct SectionData {
    name: String,
    layout: String,       // "horizontal" or "vertical"
    bg_color: Color,
    text_color: Color,
    title_color: Color,
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
}

// ── messages ────────────────────────────────────────────────────────

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {
    Tick(Instant),
    IcedEvent(Event),
    /// A sensor entered/exited the hover state: (section_index, sensor_index, hovered).
    SensorHovered(usize, usize, bool),
    /// A sensor was clicked: (section_index, sensor_index, action).
    SensorPressed(usize, usize, Option<String>),
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

/// Build the sensor box: background plus its border, drawn according to
/// `border_type`:
///
/// * `full` (or any unknown value) -> native container border, which also
///   honors `border_radius`;
/// * `underline` / `upperline` / `left` / `right` -> a single colored bar on
///   that side, overlaid through a `Stack` so the sensor keeps exactly the
///   size it would have without a border;
/// * `corners` -> four L-shaped brackets (each arm covers one quarter of
///   its side, so every side keeps a visible middle gap).
///
/// The border is drawn only when both a color and a positive width are set,
/// which is the rule the native border always applied.
fn sensor_box<'a>(
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

fn update(panel: &mut Panel, message: Message, _time: Instant) -> Command<Message> {
    match message {
        Message::Tick(_) => {
            // Hot reload: pick up config file changes (content and, when
            // layout/align differ from what is applied, surface geometry).
            let mut geometry_changed = false;
            let mut zone_changed = false;
            if let Some(reloaded) = maybe_reload(panel) {
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

                let section_text_color = section_cfg.text_color.as_ref()
                    .and_then(|s| config::parse_color(s).ok())
                    .unwrap_or(section_default_txt);

                let sensors: Vec<SensorValue> = section_cfg
                    .sensors
                    .iter()
                    .map(|s| {
                        let value = sensor_runner::run(&s.source);
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
                            name: s.name.clone(),
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
                        }
                    })
                    .collect();

                let default_bg = panel.config.panel.default_bg_color
                    .as_ref()
                    .map(|s| config::parse_color(s).ok())
                    .unwrap_or(Some(DEFAULT_BG_COLOR))
                    .unwrap_or(DEFAULT_BG_COLOR);

                sections.push(SectionData {
                    name: section_cfg.name.clone(),
                    layout: section_cfg.layout.as_ref().map(|s| s.to_string()).unwrap_or("horizontal".to_string()),
                    bg_color: resolve_color(
                        section_cfg.bg_color.as_ref().and_then(|s| config::parse_color(s).ok()),
                        default_bg,
                    ),
                    text_color: section_text_color,
                    title_color: {
                        let tc = section_cfg.text_color.as_ref()
                            .map(|sc| config::parse_color(sc).ok())
                            .flatten()
                            .or_else(|| {
                                panel.config.panel.default_text_color
                                    .as_ref()
                                    .map(|sc| config::parse_color(sc).ok())
                                    .flatten()
                                    .or(Some(DEFAULT_TEXT_COLOR))
                            })
                            .unwrap_or(DEFAULT_TEXT_COLOR);
                        tc.scale_alpha(0.8)
                    },
                    weight: panel.config.panel.default_section_weight.as_ref().map(|w| (*w as u32) as f32).unwrap_or(1.0),
                    sensors,
                });
            }
            panel.sections = sections;
            let mut cmds: Vec<Command<Message>> = Vec::new();
            if geometry_changed {
                // Resize/re-anchor the layer surface at runtime; the view
                // already uses the new config for content and alignment.
                cmds.push(Command::done(Message::LayoutChange {
                    anchor: panel.last_anchor,
                    size: panel.last_size,
                }));
            }
            if zone_changed {
                // Keep hyprland's reserved space in step with the thickness.
                cmds.push(Command::done(Message::ExclusiveZoneChange(panel.last_exclusive_zone)));
            }
            if cmds.is_empty() { Command::none() } else { Command::batch(cmds) }
        }
        Message::IcedEvent(e) => {
            // Mouse enter/leave/press are handled per-sensor by MouseArea,
            // which emits Message::SensorHovered / SensorPressed. Nothing else here.
            let _ = e;
            Command::none()
        }
        Message::SensorPressed(_section_index, _sensor_index, action) => {
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
                None => {}
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

fn subscription(_panel: &Panel) -> Subscription<Message> {
    Subscription::batch([
        time::every(std::time::Duration::from_secs(1)).map(|now| Message::Tick(now)),
        event::listen().map(Message::IcedEvent),
    ])
}

// ── view ────────────────────────────────────────────────────────────

fn view<'a>(panel: &'a Panel) -> Element<'a, Message> {
    let font_name = panel.config.panel.font.as_ref().map(|s| s.to_string()).unwrap_or(DEFAULT_FONT.to_string());
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let _ = &font_name;

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
        let font_obj = Font::with_name(DEFAULT_FONT);

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

        // Build sensor widgets with optional per-sensor background.
        let sensor_widgets: Vec<Element<'a, Message>> = section.sensors.iter().enumerate().map(|(sensor_idx, s)| {
            let sensor_text_color = s.text_color.as_ref().map(|c| *c).unwrap_or(section_text);

            // Text color: hover variant when the cursor is over this sensor.
            let text_color = if panel.hovered == Some((section_idx, sensor_idx)) {
                s.text_color_hover.unwrap_or_else(|| s.text_color.unwrap_or(section_text))
            } else {
                sensor_text_color
            };

            let label_widget = text(&s.name)
                .color(text_color)
                .size(font_size)
                .font(font_obj);

            let value_widget = text(&s.value)
                .color(text_color)
                .size(font_size)
                .font(font_obj);

            // Name and value side by side, separated by a small gap (the
            // colon separator was removed on request).
            let row = Row::new()
                .push(label_widget)
                .push(value_widget)
                .spacing(6);

            let desc_str: String = s.description.clone();
            let has_desc = !desc_str.is_empty();
            let desc_value = desc_str.clone();

            let content: Element<'a, Message> = if has_desc {
                let desc_widget = text(desc_value)
                    .color(Color::from_rgba(0.5, 0.5, 0.5, 0.7))
                    .size(font_size.saturating_sub(2))
                    .font(font_obj);
                Column::new().push(row).push(desc_widget).into()
            } else {
                row.into()
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

            let styled_container = sensor_box(
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

            // Wrap in MouseArea to deliver hover + click for this specific sensor.
            MouseArea::new(styled_container)
                .on_enter(Message::SensorHovered(section_idx, sensor_idx, true))
                .on_exit(Message::SensorHovered(section_idx, sensor_idx, false))
                .on_press(Message::SensorPressed(
                    section_idx,
                    sensor_idx,
                    s.action.clone(),
                ))
                .interaction(iced::mouse::Interaction::Pointer)
                .into()
        }).collect();

        // Optionally prepend a section title.
        let sensors_inner: Element<'a, Message> = if section.layout == "vertical" {
            let title_widget = container(
                text(section.name.clone())
                    .color(title_color)
                    .size(font_size.saturating_add(2))
                    .font(font_obj)
            ).align_x(Alignment::Center);
            let title_el: Element<'a, Message> = title_widget.padding(4).into();
            let sensor_col = Column::new()
                .extend(sensor_widgets)
                .padding(4);
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
                .padding(4);
            // `stretch` here means: horizontal section in a horizontal panel ->
            // fill the thickness (height) so the boxes stretch.
            let row = if stretch { row.height(Length::Fill) } else { row };
            row.into()
        };

        // Wrap in container with section background. It fills the panel's cross
        // (thickness) axis and centres the section content on that axis, so
        // `thickness` reaches the sections: stretched boxes fill it, stacked
        // groups are centred within it.
        let section_element: Element<'a, Message> = container(sensors_inner)
            .width(if vertical { Length::Fill } else { Length::Shrink })
            .height(if vertical { Length::Shrink } else { Length::Fill })
            .align_x(if vertical { Alignment::Center } else { Alignment::Start })
            .align_y(if vertical { Alignment::Start } else { Alignment::Center })
            .style(move |_theme: &iced::Theme| ContainerStyle {
                background: Some(Background::Color(section_bg)),
                ..ContainerStyle::default()
            })
            .into();

        section_elements.push(section_element);
    }

    // Build the full panel layout. The main axis shrinks to the content (so
    // `align` can place the panel within the surface); the cross axis fills
    // the surface thickness, which is what makes `thickness` reach the
    // sections and the sensor boxes.
    let main_layout: Element<'a, Message> = if vertical {
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
    // (`vertical` is computed at the top of the function.)
    let align = panel
        .config
        .panel
        .align
        .as_deref()
        .unwrap_or("center");

    let panel_container = container(main_layout)
        .width(if vertical { Length::Fill } else { Length::Shrink })
        .height(if vertical { Length::Shrink } else { Length::Fill })
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(panel_bg)),
            ..ContainerStyle::default()
        });

    // Outer container fills the surface on BOTH axes, so `align` has room to
    // move the (shrink) panel on its main axis: left / center / right
    // (horizontal) or top / center / bottom (vertical).
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

/// Panel thickness on the cross axis, in pixels: `panel.thickness` from the
/// config (default 32px, minimum 1px). Single source of truth for the surface
/// size and for the exclusive zone hyprland reserves.
fn panel_thickness(config: &config::Config) -> u32 {
    config.panel.thickness.unwrap_or(DEFAULT_HEIGHT).max(1)
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
/// single edge that sticks the panel to the screen border:
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

    let align = config.panel.align.as_deref().unwrap_or("center");

    if vertical {
        match align {
            "bottom" => Anchor::Bottom,
            "right" => Anchor::Right,
            _ => Anchor::Left,
        }
    } else {
        match align {
            "bottom" => Anchor::Bottom,
            _ => Anchor::Top,
        }
    }
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
            timed(
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
                    }
                },
                namespace,
                update,
                subscription,
                view,
            )
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
