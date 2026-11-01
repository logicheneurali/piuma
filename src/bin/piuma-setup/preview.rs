//! WYSIWYG sensor previews shared by the source picker, the sensor form
//! and the Sections/Panel mini-bar. Everything here is static sample data:
//! a preview never runs a sensor, a command or D-Bus.
use iced::widget::{button, column, container, scrollable, text, Row};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::config;
use crate::controls::{resolved_icon, sensor_icon_size, sensor_padding};
use crate::icons;
use crate::model::{canonical_type, source_label, PiumaConfig, Section, Sensor};
use crate::state::{Message, PiumaSetup};
use crate::style::{btn_ghost, inset_style, muted_text, palette, SETUP_DEFAULT_FG};

/// A representative reading per source type, so a preview shows the
/// sensor's shape instead of the type key.
pub(crate) fn sample_value(source_type: &str) -> &'static str {
    match canonical_type(source_type) {
        "perc_cpu" => "12%",
        "perc_gpu" => "34%",
        "perc_mem" => "48%",
        "perc_swap" => "3%",
        "perc_disk" => "61%",
        "perc_battery" => "82%",
        "perc_backlight" => "70%",
        "perc_volume" => "40%",
        "used_mem" => "5.2 GiB",
        "used_swap" => "512 MiB",
        "used_disk" => "120 GiB",
        "used_battery" => "18.4 Wh",
        "used_netdown" => "1.4 MiB/s",
        "used_netup" => "320 KiB/s",
        "total_mem" => "16 GiB",
        "total_swap" => "8 GiB",
        "total_disk" => "512 GiB",
        "total_battery" => "52 Wh",
        "detail_mem" => "5.2/16 GiB",
        "detail_swap" => "0.5/8 GiB",
        "detail_disk" => "120/512 GiB",
        "temp_cpu" => "48°C",
        "net_state" => "up",
        "uptime" => "3d 4h",
        "clock" => "14:05",
        "calendar" => "Sat 31 Oct",
        "window" => "Firefox — Piuma",
        "workspace" => "2",
        "workspace_dots" => "",
        "media" => "⏵  Song — Artist",
        "tray" => "",
        "notifications" => "3 new",
        "taskbar" => "",
        "layout" => "Work",
        "command" => "42",
        "env_var" => "/home/user",
        "static_text" => "Text",
        "volume_meter" => "",
        _ => "value",
    }
}

/// A short sample name per type, shown in the picker cells (the friendly
/// label sits under the pill, so it is deliberately terse).
pub(crate) fn sample_name(source_type: &str) -> &'static str {
    match canonical_type(source_type) {
        "perc_cpu" => "CPU",
        "perc_gpu" => "GPU",
        "perc_mem" | "used_mem" | "total_mem" | "detail_mem" => "MEM",
        "perc_swap" | "used_swap" | "total_swap" | "detail_swap" => "SWAP",
        "perc_disk" | "used_disk" | "total_disk" | "detail_disk" => "DISK",
        "perc_battery" | "used_battery" | "total_battery" => "BAT",
        "perc_backlight" => "LIGHT",
        "perc_volume" | "volume_meter" => "VOL",
        "used_netdown" | "used_netup" => "NET",
        "net_state" => "NET",
        "temp_cpu" => "TEMP",
        "uptime" => "UP",
        "clock" => "CLOCK",
        "calendar" => "CAL",
        "window" => "WINDOW",
        "workspace" | "workspace_dots" => "WS",
        "tray" => "TRAY",
        "taskbar" => "TASKBAR",
        "media" => "MEDIA",
        "notifications" => "NOTIF",
        "layout" => "LAYOUT",
        "command" => "CMD",
        "env_var" => "ENV",
        "static_text" => "",
        _ => "",
    }
}

/// The value a form preview shows: static_text previews its own text,
/// everything else the table above.
fn preview_value(sensor: &Sensor) -> String {
    if canonical_type(&sensor.source_type) == "static_text" {
        sensor
            .source_config
            .static_text
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| sample_value("static_text").to_string())
    } else {
        sample_value(&sensor.source_type).to_string()
    }
}

/// The name the bar draws: sensors whose content carries it (text/icon,
/// tray, workspace buttons) show no separate name.
fn display_name(sensor: &Sensor) -> String {
    if matches!(
        canonical_type(&sensor.source_type),
        "static_text" | "tray" | "workspace"
    ) {
        return String::new();
    }
    sensor
        .name
        .clone()
        .unwrap_or_else(|| source_label(canonical_type(&sensor.source_type)).to_string())
}

/// The resolved pill style: sensor value → sensor defaults → section →
/// section defaults (the bar's group chain, flattened for a preview).
struct Paint {
    text: Color,
    bg: Option<Color>,
    border: Option<Color>,
    width: f32,
    radius: f32,
    padding: f32,
}

fn parse(value: Option<&String>) -> Option<Color> {
    value.and_then(|s| config::parse_color(s).ok())
}

fn paint(cfg: &PiumaConfig, section: Option<&Section>, sensor: &Sensor) -> Paint {
    let defaults = &cfg.sensor_defaults;
    let sec = section;
    let light = cfg.theme_mode.as_deref() == Some("light");
    let mut text = parse(sensor.text_color.as_ref())
        .or_else(|| parse(defaults.text_color.as_ref()))
        .or_else(|| sec.and_then(|s| parse(s.text_color.as_ref())))
        .or_else(|| parse(cfg.section_defaults.text_color.as_ref()))
        .or_else(|| parse(cfg.default_text_color.as_ref()))
        .unwrap_or(SETUP_DEFAULT_FG);
    let mut bg = parse(sensor.bg_color.as_ref())
        .or_else(|| parse(defaults.bg_color.as_ref()))
        .or_else(|| sec.and_then(|s| parse(s.bg_color.as_ref())))
        .or_else(|| parse(cfg.section_defaults.bg_color.as_ref()));
    if light {
        // Explicit sensor AND section colours are user accents and stay
        // as they are; inherited/default surfaces follow the variant.
        let explicit_text = sensor.text_color.is_some()
            || sec.and_then(|s| s.text_color.as_ref()).is_some();
        let explicit_bg = sensor.bg_color.is_some()
            || sec.and_then(|s| s.bg_color.as_ref()).is_some();
        if !explicit_text {
            text = crate::style::light_text(text);
        }
        if !explicit_bg {
            bg = bg.map(|c| crate::style::light_surface(c, 0.90));
        }
    }
    let border = parse(sensor.border_color.as_ref())
        .or_else(|| parse(defaults.border_color.as_ref()))
        .or_else(|| sec.and_then(|s| parse(s.border_color.as_ref())))
        .or_else(|| parse(cfg.section_defaults.border_color.as_ref()));
    let number = |own: Option<i32>, default: Option<i32>, section_value: Option<i32>| {
        config::style_num(own)
            .or_else(|| config::style_num(default))
            .or_else(|| config::style_num(section_value))
            .unwrap_or(0)
    };
    Paint {
        text,
        bg,
        border,
        width: number(
            sensor.border_width,
            defaults.border_width,
            sec.and_then(|s| s.border_width),
        ) as f32,
        radius: number(
            sensor.border_radius,
            defaults.border_radius,
            sec.and_then(|s| s.border_radius),
        ) as f32,
        padding: sensor_padding(cfg, sensor) as f32,
    }
}

fn icon_size(cfg: &PiumaConfig, section: Option<&Section>, sensor: &Sensor) -> u16 {
    match section {
        Some(section) => sensor_icon_size(cfg, section, sensor),
        None => {
            let thickness = if cfg.thickness == 0 {
                config::DEFAULT_HEIGHT
            } else {
                cfg.thickness
            };
            icons::row_icon_target(
                thickness,
                0,
                sensor_padding(cfg, sensor),
            )
        }
    }
}

/// The sensor's effective icon name (volume meter's speaker included).
fn icon_name(sensor: &Sensor) -> Option<&str> {
    sensor.icon.as_deref().or_else(|| {
        matches!(canonical_type(&sensor.source_type), "volume_meter")
            .then_some(icons::VOLUME_ICON)
    })
}

/// One sensor pill with the sensor's real colours, padding and border:
/// the unit shared by the form preview, the type picker and the
/// mini-bar. Text sizes follow `panel.font_size`.
pub(crate) fn pill(
    cfg: &PiumaConfig,
    section: Option<&Section>,
    sensor: &Sensor,
    name: &str,
    value: &str,
) -> Element<'static, Message> {
    let style = paint(cfg, section, sensor);
    let font = cfg.font_size.clamp(9, 22);
    let mut row = Row::new().spacing(6).align_y(Alignment::Center);
    if let Some(icon) = resolved_icon(cfg, icon_size(cfg, section, sensor), icon_name(sensor)) {
        row = row.push(icon);
    }
    if !name.is_empty() {
        row = row.push(text(name.to_string()).size(font).color(style.text));
    }
    if !value.is_empty() {
        row = row.push(text(value.to_string()).size(font).color(style.text));
    }
    container(row)
        .padding(style.padding)
        .style(move |_t: &Theme| container::Style {
            background: style.bg.map(Background::Color),
            border: Border {
                width: if style.border.is_some() {
                    style.width
                } else {
                    0.0
                },
                color: style.border.unwrap_or(Color::TRANSPARENT),
                radius: style.radius.min(16.0).into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// The preview pill of a bare source type, for the picker cells.
pub(crate) fn kind_pill(cfg: &PiumaConfig, kind: &str) -> Element<'static, Message> {
    let sensor = Sensor {
        source_type: canonical_type(kind).to_string(),
        ..Sensor::default()
    };
    pill(cfg, None, &sensor, sample_name(kind), sample_value(kind))
}

/// A live preview of one sensor as currently edited.
pub(crate) fn sensor_preview(
    cfg: &PiumaConfig,
    section: Option<&Section>,
    sensor: &Sensor,
) -> Element<'static, Message> {
    let value = preview_value(sensor);
    pill(cfg, section, sensor, &display_name(sensor), &value)
}

/// WCAG relative-luminance contrast ratio between two colours.
pub(crate) fn contrast_ratio(a: Color, b: Color) -> f32 {
    fn channel(c: f32) -> f32 {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    let lum = |c: Color| 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Readability warning for the preview: the text over the resolved
/// background drops below 3:1 (transparent pills use the window card).
pub(crate) fn contrast_hint(
    cfg: &PiumaConfig,
    section: Option<&Section>,
    sensor: &Sensor,
) -> Option<String> {
    let style = paint(cfg, section, sensor);
    let bg = style.bg.unwrap_or_else(|| palette(cfg).card);
    let ratio = contrast_ratio(style.text, bg);
    (ratio < 3.0).then(|| format!("⚠ low text/bg contrast ({ratio:.1}:1)"))
}

/// The Sections/Panel mini-bar: every section and sensor in bar order,
/// each a clickable pill with a sample reading; pressing one jumps to
/// its card in the Sections tab.
pub(crate) fn mini_bar(state: &PiumaSetup) -> Element<'static, Message> {
    let pal = palette(&state.config);
    let mut strip = Row::new().spacing(6).align_y(Alignment::Center);
    for (section_index, section) in state.config.sections.iter().enumerate() {
        if section_index > 0 {
            strip = strip.push(
                text("│")
                    .size(14)
                    .style(muted_text(pal)),
            );
        }
        if let Some(name) = &section.name {
            strip = strip.push(text(name.clone()).size(10).style(muted_text(pal)));
        }
        for (sensor_index, sensor) in section.sensors.iter().enumerate() {
            // The sensor whose card is open carries an accent ring, so
            // the strip also shows where the editor currently is.
            let pill = sensor_preview(&state.config, Some(section), sensor);
            let pill: Element<'static, Message> = if state
                .sensors_open
                .contains(&(section_index, sensor_index))
            {
                container(pill)
                    .padding(1)
                    .style(move |_t: &Theme| container::Style {
                        border: Border {
                            width: 1.0,
                            color: pal.accent,
                            radius: 6.0.into(),
                        },
                        ..container::Style::default()
                    })
                    .into()
            } else {
                pill
            };
            strip = strip.push(
                button(pill)
                    .on_press(Message::SensorSelected(section_index, sensor_index))
                    .padding(0)
                    .style(|_t: &Theme, _s| btn_ghost()),
            );
        }
    }
    let empty = column![text("No sensors yet.").size(10).style(muted_text(pal))];
    let body: Element<'static, Message> = if state
        .config
        .sections
        .iter()
        .all(|section| section.sensors.is_empty())
    {
        empty.into()
    } else {
        // Real scrollbar, embedded with a gap instead of floating over
        // the strip: it stays draggable and never covers a pill.
        scrollable(strip)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .spacing(6.0)
            .width(Length::Fill)
            .into()
    };
    container(body)
        .padding(8)
        .width(Length::Fill)
        .style(inset_style(pal))
        .into()
}
