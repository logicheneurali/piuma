//! Piuma - System info panel for Hyprland (LayerShell/Wayland)
//!
//! Uses iced + iced_exwlshell for continuous rendering on the desktop.
//!
//! Configuration is loaded from a JSON file (CLI arg or default path).
//! See `src/config.rs` for the config schema.

use std::time::Instant;

use iced::widget::{container, text, Column, Row, MouseArea};
use iced::{Alignment, Background, Color, Element, Event, Length, Subscription, Task as Command};
use iced::event;
use iced::time;
use iced::font::Font;
use iced::widget::container::Style as ContainerStyle;

use iced_exwlshell::ExWlSettings;
use iced_exwlshell::layershell::timed;
use iced_exwlshell::reexport::{Anchor, Layer};
use iced_exwlshell::settings::{LayerShellSettings, StartMode};
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
const DEFAULT_EXCLUSIVE_ZONE: i32 = 32;
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

// ── update ──────────────────────────────────────────────────────────

fn update(panel: &mut Panel, message: Message, _time: Instant) -> Command<Message> {
    match message {
        Message::Tick(_) => {
            // Reload config from file so runtime edits take effect.
            unsafe {
                let cfg_opt = CONFIG_PATH.as_ref();
                if let Some(cfg_path) = cfg_opt {
                    if let Ok(contents) = std::fs::read_to_string(cfg_path) {
                        if let Ok(reloaded) = serde_json::from_str::<config::Config>(&contents) {
                            panel.config = reloaded;
                        }
                    }
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
                        let sensor_tc = s.text_color.as_ref()
                            .and_then(|sc| config::parse_color(sc).ok())
                            .unwrap_or(section_text_color);
                        let sensor_bg: Option<Color> = match s.bg_color {
                            Some(ref sc) => match config::parse_color(sc) {
                                Ok(c) => Some(c),
                                Err(_) => None,
                            },
                            None => None,
                        };
                        let sensor_bg_hover: Option<Color> = match s.bg_color_hover {
                            Some(ref sc) => match config::parse_color(sc) {
                                Ok(c) => Some(c),
                                Err(_) => None,
                            },
                            None => None,
                        };
                        let sensor_tc_hover: Option<iced::Color> = match s.text_color_hover {
                            Some(ref sc) => config::parse_color(sc).ok(),
                            None => None,
                        };
                        let sensor_border: Option<Color> = match s.border_color {
                            Some(ref sc) => config::parse_color(sc).ok(),
                            None => None,
                        };
                        SensorValue {
                            name: s.name.clone(),
                            description: s.description.as_ref().map(|d| d.to_string()).unwrap_or_default(),
                            value,
                            text_color: Some(sensor_tc),
                            bg_color: sensor_bg,
                            text_color_hover: sensor_tc_hover,
                            bg_color_hover: sensor_bg_hover,
                            border_color: sensor_border,
                            border_width: s.border_width.unwrap_or(0) as u16,
                            border_radius: s.border_radius.unwrap_or(0) as u16,
                            padding: s.padding.unwrap_or(0) as u16,
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
            Command::none()
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

    // Build sections.
    let mut section_elements: Vec<Element<'a, Message>> = Vec::new();

    for (section_idx, section) in panel.sections.iter().enumerate() {
        let section_bg = section.bg_color;
        let section_text = section.text_color;
        let has_title = !section.name.is_empty();
        let title_color = section.title_color;
        let font_obj = Font::with_name(DEFAULT_FONT);

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

            let row = Row::new()
                .push(label_widget)
                .push(text(": ").color(text_color).size(font_size).font(font_obj))
                .push(value_widget)
                .spacing(2);

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

            let styled_container = container(content)
                .padding(pad)
                .style(move |_theme: &iced::Theme| {
                    let mut style = ContainerStyle::default();
                    if let Some(bg) = bg_color {
                        style.background = Some(Background::Color(bg));
                    }
                    if let Some(border) = border_color {
                        style.border.color = border;
                        style.border.width = s.border_width as f32;
                        style.border.radius = (s.border_radius as f32).into();
                    }
                    style
                });

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
            if has_title {
                Column::new()
                    .push(title_el)
                    .push(sensor_col)
                    .into()
            } else {
                sensor_col.into()
            }
        } else {
            Row::new()
                .extend(sensor_widgets)
                .spacing(8)
                .padding(4)
                .into()
        };

        // Wrap in container with section background.
        let section_element: Element<'a, Message> = container(sensors_inner)
            .style(move |_theme: &iced::Theme| ContainerStyle {
                background: Some(Background::Color(section_bg)),
                ..ContainerStyle::default()
            })
            .into();

        section_elements.push(section_element);
    }

    // Build the full panel layout.
    let main_layout: Element<'a, Message> = if panel.config.panel.section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal") == "vertical"
    {
        Column::new()
            .extend(section_elements)
            .width(Length::Fill)
            .into()
    } else {
        Row::new()
            .extend(section_elements)
            .width(Length::Fill)
            .into()
    };

    let panel_bg = panel.config.panel.default_bg_color
        .as_ref()
        .and_then(|s| config::parse_color(s).ok())
        .unwrap_or(DEFAULT_BG_COLOR);

    // Position the real panel within the surface according to `align`.
    // The surface is screen-sized on the cross axis and sized to the panel's
    // own content on the main axis; the inner container is shrink-wrapped to
    // the content (so the panel is as big as its sensors) and aligned on the
    // main axis.
    let vertical = panel
        .config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal") == "vertical";
    let align = panel
        .config
        .panel
        .align
        .as_deref()
        .unwrap_or("center");

    let main_layout = container(main_layout)
        .width(Length::Shrink)
        .height(Length::Shrink)
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(panel_bg)),
            ..ContainerStyle::default()
        });

    let main_layout = if vertical {
        // Vertical panel: main axis is Y. Fill the cross axis (X) so the
        // background spans the surface, then align along Y.
        main_layout
            .width(Length::Fill)
            .align_y(match align {
                "top" => Alignment::Start,
                "bottom" => Alignment::End,
                _ => Alignment::Center,
            })
    } else {
        // Horizontal panel: main axis is X. Fill the cross axis (Y) so the
        // background spans the surface, then align along X.
        main_layout
            .height(Length::Fill)
            .align_x(match align {
                "left" => Alignment::Start,
                "right" => Alignment::End,
                _ => Alignment::Center,
            })
    };

    main_layout.into()
}

// ── positioning helpers ─────────────────────────────────────────────

/// Panel size (as the LayerSurface requests it).
///
/// The surface spans the whole screen on the axis *perpendicular* to the
/// panel orientation, so the inner panel can be positioned freely inside it.
/// The main axis (the direction the panel extends) is sized to the panel's
/// own content — there is no `percentage` anymore; the panel is as big as its
/// sensors.
///
/// * horizontal panel  -> `screen_width  x panel_thickness`
/// * vertical panel    -> `panel_thickness x screen_height`
fn panel_size(config: &config::Config, screen_width: u32, screen_height: u32) -> iced_exwlshell::settings::LayerSize {
    let vertical = config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal") == "vertical";

    // Cross-axis extent: the whole screen, so the surface reaches the edges
    // along the perpendicular direction.
    let cross = if vertical {
        iced_exwlshell::settings::Extent::px(screen_width)
    } else {
        iced_exwlshell::settings::Extent::px(screen_height)
    };

    // Main axis is content-sized, so leave it to the compositor (Fill).
    iced_exwlshell::settings::LayerSize {
        width: if vertical { cross } else { iced_exwlshell::settings::Extent::Fill },
        height: if vertical { iced_exwlshell::settings::Extent::Fill } else { cross },
    }
}

/// Anchor set for the surface.
///
/// The surface spans the screen on the axis *perpendicular* to the panel
/// orientation (so the inner panel can be positioned freely), and is pinned to
/// the screen edge on its own axis:
///
/// * horizontal, top   -> `Top  | Left  | Right`
/// * horizontal, bottom-> `Bottom| Left  | Right`
/// * vertical,   left  -> `Left  | Top   | Bottom`
/// * vertical,   right -> `Right | Top   | Bottom`
/// * center / anything -> pinned to the nearest edge (top / left).
fn panel_anchor(config: &config::Config) -> Anchor {
    let vertical = config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal") == "vertical";

    if vertical {
        match config.panel.align.as_deref().unwrap_or("center") {
            "bottom" => Anchor::Bottom | Anchor::Left | Anchor::Right,
            "right" => Anchor::Right | Anchor::Top | Anchor::Bottom,
            _ => Anchor::Top | Anchor::Left | Anchor::Bottom,
        }
    } else {
        match config.panel.align.as_deref().unwrap_or("center") {
            "right" => Anchor::Right | Anchor::Top | Anchor::Bottom,
            _ => Anchor::Top | Anchor::Left | Anchor::Right,
        }
    }
}

// ── style ───────────────────────────────────────────────────────────

fn style(_panel: &Panel, theme: &iced::Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: DEFAULT_BG_COLOR,
        text_color: theme.palette().text,
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

    // Load configuration once at startup.
    let config = config::load_config(config_path.clone());

    eprintln!("[piuma] Config loaded: {} section(s)", config.panel.sections.len());

    // Best-effort primary monitor dimensions for screen-wide surface sizing.
    unsafe {
        SCREEN_WIDTH = active_screen_width().unwrap_or(1920);
        SCREEN_HEIGHT = active_screen_height().unwrap_or(1080);
    }

    unsafe { CONFIG = Some(config.clone()); }
    unsafe { CONFIG_PATH = config_path.clone(); }

    let panel_config = unsafe { CONFIG.as_ref().cloned().unwrap_or_else(|| config::default_config()) };

    // Run the iced app on a dedicated thread so the close action can be
    // observed from the main thread without blocking the event loop.
    let handle = std::thread::Builder::new()
        .name("piuma-panel".to_string())
        .spawn(move || {
            timed(
                || {
                    let cfg = unsafe { CONFIG.as_ref().map(|c| c.clone()).unwrap_or_else(|| config::default_config()) };
                    Panel {
                        config: cfg,
                        sections: Vec::new(),
                        hovered: None,
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
                    exclusive_zone: DEFAULT_EXCLUSIVE_ZONE,
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
