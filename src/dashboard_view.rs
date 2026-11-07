//! The dashboard layout: a two-column card over a full-screen scrim.
//! Left: clock is in the header, then media, the stateful controls and
//! the sliders. Right: the numeric sensors as tiles plus a scrollable
//! notification list. Every control shows its current state; nothing
//! here runs while the card is closed (see `dashboard`).

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::widget::{button, column, container, image, row, scrollable, text, Column, MouseArea, Row, Slider, Space};
use iced::{Background, Border, Color, Element, Length};

use crate::dashboard::{self, Action};
use crate::text_util::panel_font;
use crate::{notifications, popup_style, Message, Panel};

/// The overlay's full-screen scrim: dark enough to separate the card,
/// light enough to keep the desktop behind visible.
const SCRIM: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.35,
};

const LEFT_WIDTH: f32 = 360.0;
const MAX_SYSTEM_TILES: usize = 8;
const MAX_NOTIFICATIONS: usize = 20;
const TILE_H: f32 = 58.0;
const SYSTEM_TILE_H: f32 = 66.0;

/// The full-screen surface: the scrim is clickable everywhere outside
/// the card.
pub(crate) fn view(panel: &Panel) -> Element<'_, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let (clock, date) = dashboard::clock_and_date();
    let card_w = (unsafe { crate::SCREEN_WIDTH } as f32 - 120.0).clamp(720.0, 980.0);
    let card_h = (unsafe { crate::SCREEN_HEIGHT } as f32 - 160.0).clamp(480.0, 620.0);

    let scrim = || -> Element<'_, Message> {
        MouseArea::new(Space::new().width(Length::Fill).height(Length::Fill))
            .on_press(Message::DashboardClose)
            .into()
    };

    let card = container(
        column![
            header(panel, &st, font, clock, date),
            Row::new()
                .push(left_column(panel, &st, font))
                .push(right_column(panel, &st, font))
                .spacing(18)
                .height(Length::Fill)
                .width(Length::Fill),
        ]
        .spacing(12)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .padding(16)
    .width(Length::Fixed(card_w))
    .height(Length::Fixed(card_h))
    .style(move |_t| container::Style {
        background: Some(Background::Color(Color { a: 0.98, ..st.bg })),
        border: Border {
            width: st.border_width as f32,
            color: st.border,
            radius: (st.border_radius as f32).into(),
        },
        ..container::Style::default()
    });

    // Top / bottom / left / right strips: each closes on click, and the
    // card's own controls keep their clicks.
    container(
        column![
            scrim(),
            row![scrim(), card, scrim()]
                .height(Length::Fixed(card_h))
                .width(Length::Fill),
            scrim(),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(SCRIM)),
        ..container::Style::default()
    })
    .into()
}

/// Clock and date on the left, the session chips and the ✕ on the right.
fn header<'a>(
    panel: &'a Panel,
    st: &crate::PopupStyle,
    font: iced::Font,
    clock: String,
    date: String,
) -> Element<'a, Message> {
    let probe = panel.dash_probe.as_ref();
    let mut chips = Row::new().spacing(14).align_y(iced::Alignment::Center);
    if let Some(battery) = probe.and_then(|p| p.battery.as_deref()) {
        chips = chips.push(chip("Battery", battery, st, font));
    }
    if let Some(probe) = probe {
        chips = chips.push(chip("Uptime", &probe.uptime, st, font));
    }
    row![
        column![
            text(clock).size(34).font(font).color(st.text_hover),
            text(date).size(12).font(font).color(st.text),
        ]
        .spacing(2),
        Space::new().width(Length::Fill),
        chips,
        button(text("✕").size(st.font_size).font(font).color(st.text))
            .on_press(Message::DashboardClose)
            .padding(4)
            .style(button_style(st)),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn chip<'a>(label: &'a str, value: &'a str, st: &crate::PopupStyle, font: iced::Font) -> Element<'a, Message> {
    column![
        text(label).size(9).font(font).color(st.text),
        text(value).size(11).font(font).color(st.text_hover),
    ]
    .spacing(1)
    .align_x(iced::Alignment::End)
    .into()
}

/// Media, the stateful controls, the sliders and the session line.
fn left_column<'a>(
    panel: &'a Panel,
    st: &crate::PopupStyle,
    font: iced::Font,
) -> Element<'a, Message> {
    let probe = panel.dash_probe.as_ref();
    let profile = probe
        .map(|p| {
            panel
                .dash_profile
                .as_deref()
                .unwrap_or(p.power_profile.as_str())
        })
        .unwrap_or("N/A");
    let bluetooth = truncate(probe.map(|p| p.bluetooth.as_str()).unwrap_or("N/A"), 20);
    let controls = column![
        row![
            tile("Power profile", profile, false, Action::PowerProfile, st, font),
            tile(
                "Caffeine",
                if panel.caffeine { "On" } else { "Off" },
                panel.caffeine,
                Action::Caffeine,
                st,
                font,
            ),
        ]
        .spacing(8),
        row![
            tile(
                "Bluetooth",
                &bluetooth,
                probe.is_some_and(|p| p.bluetooth.starts_with("On")),
                Action::Bluetooth,
                st,
                font,
            ),
            tile("Lock", "Lock now", false, Action::Lock, st, font),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .width(Length::Fill);

    let volume = probe.and_then(|p| p.volume);
    let brightness = probe.and_then(|p| p.brightness);
    let slider_st = SliderStyle {
        track: Color { a: 0.22, ..st.text_hover },
        active: st.text_hover,
        knob: st.text_hover,
    };

    let mut sliders = Column::new().spacing(8).width(Length::Fill);
    if let Some(volume) = volume {
        sliders = sliders.push(slider_row(
            "Volume",
            panel.dash_volume.unwrap_or(volume),
            slider_st,
            Message::DashboardVolume,
            Message::DashboardVolumeRelease,
            st,
            font,
        ));
    }
    if let (Some(brightness), true) = (brightness, probe.is_some_and(|p| p.can_set_brightness)) {
        sliders = sliders.push(slider_row(
            "Brightness",
            panel.dash_brightness.unwrap_or(brightness),
            slider_st,
            Message::DashboardBrightness,
            Message::DashboardBrightnessRelease,
            st,
            font,
        ));
    }

    let session = probe.map(|p| format!("{}  ·  {}", p.net, p.uptime));

    column![
        media_block(panel, st, font),
        Space::new().height(Length::Fixed(14.0)),
        text("CONTROLS").size(10).font(font).color(st.text),
        controls,
        Space::new().height(Length::Fixed(10.0)),
        sliders,
        Space::new().height(Length::Fill),
        text(session.unwrap_or_default())
            .size(10)
            .font(font)
            .color(st.text),
    ]
    .spacing(8)
    .width(Length::Fixed(LEFT_WIDTH))
    .height(Length::Fill)
    .into()
}

/// Cover, track, transport and progress.
fn media_block<'a>(panel: &'a Panel, st: &crate::PopupStyle, font: iced::Font) -> Element<'a, Message> {
    let Some(now) = panel.dash_probe.as_ref().and_then(|p| p.media.as_ref()) else {
        return container(
            text("No player")
                .size(12)
                .font(font)
                .color(st.text),
        )
        .height(Length::Fixed(140.0))
        .width(Length::Fill)
        .align_y(iced::Alignment::Center)
        .into();
    };
    let placeholder_bg = Color { a: 0.6, ..st.bg_hover };
    let art: Element<'a, Message> = match panel.cover.clone() {
        Some(handle) => image(handle)
            .width(Length::Fixed(dashboard::COVER))
            .height(Length::Fixed(dashboard::COVER))
            .content_fit(iced::ContentFit::Cover)
            .border_radius(8.0)
            .into(),
        None => container(Space::new())
            .width(Length::Fixed(dashboard::COVER))
            .height(Length::Fixed(dashboard::COVER))
            .style(move |_t| container::Style {
                background: Some(Background::Color(placeholder_bg)),
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..container::Style::default()
            })
            .into(),
    };
    let title = if now.title.is_empty() {
        "Unknown track".to_string()
    } else {
        truncate(&now.title, 40)
    };
    let status = if now.status == "Playing" {
        "Playing"
    } else {
        "Paused"
    };

    let play = if now.status == "Playing" { "⏸" } else { "⏵" };
    let transport = row![
        transport_button("⏮", Action::Previous, st, font),
        transport_button(play, Action::PlayPause, st, font),
        transport_button("⏭", Action::Next, st, font),
    ]
    .spacing(6);

    let mut progress = Column::new().spacing(2).width(Length::Fill);
    if let (Some(position), Some(length)) = (now.position_ms, now.length_ms) {
        if length > 0 {
            let fraction = (position as f32 / length as f32).clamp(0.0, 1.0);
            let filled = (fraction * 100.0).round().clamp(1.0, 100.0) as u16;
            let (done, track) = (st.text_hover, Color { a: 0.25, ..st.text_hover });
            progress = progress
                .push(
                    row![
                        container(Space::new())
                            .width(Length::FillPortion(filled))
                            .height(Length::Fixed(3.0))
                            .style(move |_t| container::Style {
                                background: Some(Background::Color(done)),
                                border: Border {
                                    radius: 2.0.into(),
                                    ..Default::default()
                                },
                                ..container::Style::default()
                            }),
                        container(Space::new())
                            .width(Length::FillPortion(100 - filled))
                            .height(Length::Fixed(3.0))
                            .style(move |_t| container::Style {
                                background: Some(Background::Color(track)),
                                ..container::Style::default()
                            }),
                    ]
                    .width(Length::Fill),
                )
                .push(
                    row![
                        text(dashboard::duration(position)).size(9).font(font).color(st.text),
                        Space::new().width(Length::Fill),
                        text(dashboard::duration(length)).size(9).font(font).color(st.text),
                    ]
                    .width(Length::Fill),
                );
        }
    }

    column![
        row![
            art,
            column![
                text(title).size(15).font(font).color(st.text),
                text(truncate(&now.artist, 32)).size(12).font(font).color(st.text_hover),
                text(status).size(10).font(font).color(st.text),
            ]
            .spacing(3)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        transport,
        progress,
    ]
    .spacing(8)
    .width(Length::Fill)
    .into()
}

fn transport_button<'a>(
    glyph: &'a str,
    action: Action,
    st: &crate::PopupStyle,
    font: iced::Font,
) -> Element<'a, Message> {
    button(text(glyph).size(14).font(font).color(st.text))
        .on_press(Message::DashboardAction(action))
        .padding([3, 10])
        .style(button_style(st))
        .into()
}

/// A stateful control: label on top, the current value under it. The
/// active tile reads as "on" (accent surface), not as a plain button.
fn tile<'a>(
    label: &str,
    value: &str,
    active: bool,
    action: Action,
    st: &crate::PopupStyle,
    font: iced::Font,
) -> Element<'a, Message> {
    let (label, value) = (label.to_string(), value.to_string());
    let surface = blend(st.bg, st.text, 0.08);
    let (bg, hover, border, text_color) = if active {
        (st.bg_hover, blend(st.bg, st.text, 0.22), st.text_hover, st.text_hover)
    } else {
        (surface, blend(st.bg, st.text, 0.18), Color { a: 0.35, ..st.text }, st.text)
    };
    button(
        column![
            text(label).size(9).font(font).color(text_color),
            text(value).size(12).font(font).color(text_color),
        ]
        .spacing(2)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::Alignment::Start),
    )
    .on_press(Message::DashboardAction(action))
    .padding([6, 9])
    .width(Length::Fill)
    .height(Length::Fixed(TILE_H))
    .style(move |_t, status| button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => hover,
            _ => bg,
        })),
        text_color,
        border: Border {
            width: 1.0,
            color: border,
            radius: 7.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The right column: the numeric sensors as two-column tiles and the
/// scrollable notification list.
fn right_column<'a>(
    panel: &'a Panel,
    st: &crate::PopupStyle,
    font: iced::Font,
) -> Element<'a, Message> {
    let cards = notifications::active_cards(MAX_NOTIFICATIONS);
    // With no notifications the system tiles take the free height (bigger
    // sparklines); with some, the list keeps its half of the column.
    let stretch = cards.is_empty();
    let mut grid = Column::new()
        .spacing(8)
        .width(Length::Fill)
        .height(if stretch { Length::Fill } else { Length::Shrink });
    let mut pair = Row::new().spacing(8).width(Length::Fill);
    let mut counted = 0;
    for (section_index, section) in panel.config.panel.sections.iter().enumerate() {
        for (sensor_index, sensor) in section.sensors.iter().enumerate() {
            if counted >= MAX_SYSTEM_TILES {
                break;
            }
            if crate::graphs::scale_of(&sensor.source).is_none()
                || matches!(sensor.source, crate::config::SensorSource::Wifi {})
            {
                continue;
            }
            let Some(value) = panel
                .sections
                .get(section_index)
                .and_then(|sec| sec.sensors.get(sensor_index))
            else {
                continue;
            };
            if value.value.is_empty() {
                continue;
            }
            pair = pair.push(system_tile(
                panel,
                section_index,
                sensor_index,
                &value.name,
                &value.value,
                stretch,
                st,
                font,
            ));
            counted += 1;
            if counted % 2 == 0 {
                grid = grid.push(pair.height(tile_row_height(stretch)));
                pair = Row::new().spacing(8).width(Length::Fill);
            }
        }
    }
    if counted % 2 == 1 {
        grid = grid.push(
            pair.push(Space::new().width(Length::Fill))
                .height(tile_row_height(stretch)),
        );
    }
    if counted == 0 {
        grid = grid.push(
            text("No numeric sensors with draw_graph")
                .size(11)
                .font(font)
                .color(st.text),
        );
    }

    let notification_surface = blend(st.bg, st.text, 0.08);
    let mut list = Column::new().spacing(6).width(Length::Fill);
    if cards.is_empty() {
        list = list.push(text("Nothing new").size(11).font(font).color(st.text));
    }
    for card in cards {
        let icon: Element<'a, Message> = match notifications::icon_handle(card.id) {
            Some(handle) => image(handle)
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .into(),
            None => Space::new().width(Length::Fixed(18.0)).into(),
        };
        list = list.push(
            container(
                row![
                    icon,
                    column![
                        text(truncate(&card.summary, 46))
                            .size(11)
                            .font(font)
                            .color(st.text_hover),
                        text(truncate(&card.body, 70)).size(10).font(font).color(st.text),
                    ]
                    .spacing(1)
                    .width(Length::Fill),
                ]
                .spacing(8)
                .width(Length::Fill),
            )
            .padding([4, 6])
            .width(Length::Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(notification_surface)),
                border: Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..container::Style::default()
            }),
        );
    }

    column![
        text("SYSTEM").size(10).font(font).color(st.text),
        grid,
        Space::new().height(Length::Fixed(8.0)),
        text("NOTIFICATIONS").size(10).font(font).color(st.text),
        scrollable(list)
            .height(if stretch { Length::Shrink } else { Length::Fill })
            .width(Length::Fill),
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// One system tile: name and value on top, the sparkline under them.
fn system_tile<'a>(
    panel: &'a Panel,
    section: usize,
    sensor: usize,
    name: &'a str,
    value: &'a str,
    stretch: bool,
    st: &crate::PopupStyle,
    font: iced::Font,
) -> Element<'a, Message> {
    let (color, surface, border_color, text_color, value_color) = (
        st.text_hover,
        blend(st.bg, st.text, 0.08),
        Color { a: 0.18, ..st.text },
        st.text,
        st.text_hover,
    );
    let (name, value) = (name.to_string(), value.to_string());
    let graph: Element<'a, Message> = match panel.graphs.window(section, sensor) {
        Some((scale, samples)) => container(
            Canvas::new(Sparkline {
                samples,
                scale,
                color,
            })
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .height(if stretch { Length::Fill } else { Length::Fixed(26.0) })
        .width(Length::Fill)
        .into(),
        None => Space::new()
            .height(if stretch { Length::Fill } else { Length::Fixed(26.0) })
            .into(),
    };
    container(
        column![
            row![
                text(name).size(10).font(font).color(text_color),
                Space::new().width(Length::Fill),
                text(value)
                    .size(if stretch { 17 } else { 13 })
                    .font(font)
                    .color(value_color),
            ]
            .width(Length::Fill),
            graph,
        ]
        .spacing(2)
        .width(Length::Fill),
    )
    .padding([6, 9])
    .width(Length::Fill)
    .height(tile_height(stretch))
    .style(move |_t| container::Style {
        background: Some(Background::Color(surface)),
        border: Border {
            width: 1.0,
            color: border_color,
            radius: 7.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// Tile heights: the fixed two-row layout with notifications under it,
/// or stretched rows when the list is empty (no dead space).
fn tile_row_height(stretch: bool) -> Length {
    if stretch {
        Length::Fill
    } else {
        Length::Fixed(SYSTEM_TILE_H)
    }
}

fn tile_height(stretch: bool) -> Length {
    if stretch {
        Length::Fill
    } else {
        Length::Fixed(SYSTEM_TILE_H)
    }
}

/// One labelled slider with its value; `on_release` commits the drag.
fn slider_row(
    label: &str,
    value: f32,
    style: SliderStyle,
    on_input: fn(f32) -> Message,
    on_release: Message,
    st: &crate::PopupStyle,
    font: iced::Font,
) -> Element<'static, Message> {
    let (label, font) = (label.to_string(), font);
    let (text_color, value_color) = (st.text, st.text_hover);
    column![
        row![
            text(label).size(10).font(font).color(text_color),
            Space::new().width(Length::Fill),
            text(format!("{value:.0}%")).size(10).font(font).color(value_color),
        ]
        .width(Length::Fill),
        Slider::new(0.0..=100.0, value, on_input)
            .on_release(on_release)
            .width(Length::Fill)
            .style(move |_t, status| slider_style(style, status)),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()
}

#[derive(Clone, Copy)]
struct SliderStyle {
    track: Color,
    active: Color,
    knob: Color,
}

fn slider_style(style: SliderStyle, status: iced::widget::slider::Status) -> iced::widget::slider::Style {
    let _ = status;
    iced::widget::slider::Style {
        rail: iced::widget::slider::Rail {
            backgrounds: (
                Background::Color(style.track),
                Background::Color(style.active),
            ),
            width: 3.0,
            border: Border {
                radius: 2.0.into(),
                ..Default::default()
            },
        },
        handle: iced::widget::slider::Handle {
            shape: iced::widget::slider::HandleShape::Circle { radius: 6.0 },
            background: Background::Color(style.knob),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// A plain trend line: the full graph widget (grid, axis labels,
/// gradient) is unreadable at 26 px.
struct Sparkline {
    samples: Vec<f32>,
    scale: crate::graphs::Scale,
    color: Color,
}

impl<Message> canvas::Program<Message> for Sparkline {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        if self.samples.len() < 2 || bounds.width <= 0.0 || bounds.height <= 0.0 {
            return vec![frame.into_geometry()];
        }
        let (low, high) = crate::graphs::y_bounds(self.scale, &self.samples);
        let span = (high - low).max(f32::EPSILON);
        let (w, h) = (bounds.width, bounds.height);
        let point = |index: usize, value: f32| {
            let x = index as f32 / (self.samples.len() - 1) as f32 * w;
            let y = h - ((value - low) / span).clamp(0.0, 1.0) * (h - 2.0) - 1.0;
            iced::Point::new(x, y)
        };
        let line = Path::new(|path| {
            path.move_to(point(0, self.samples[0]));
            for (index, value) in self.samples.iter().enumerate().skip(1) {
                path.line_to(point(index, *value));
            }
        });
        frame.stroke(
            &line,
            Stroke::default().with_width(1.5).with_color(self.color),
        );
        vec![frame.into_geometry()]
    }
}

fn button_style(st: &crate::PopupStyle) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let (hover, text_color, surface) = (
        blend(st.bg, st.text, 0.18),
        st.text_hover,
        blend(st.bg, st.text, 0.08),
    );
    move |_t, status| button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => hover,
            _ => surface,
        })),
        text_color,
        border: Border {
            width: 1.0,
            color: Color { a: 0.25, ..text_color },
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

/// Linear blend `a → b` by `t` (the card's raised surfaces).
fn blend(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: (a.a + (b.a - a.a) * t).max(a.a),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_the_limit() {
        assert_eq!(truncate("short", 10), "short");
        let out = truncate("0123456789abcdef", 8);
        assert_eq!(out.chars().count(), 8);
        assert!(out.ends_with('…'));
    }
}
