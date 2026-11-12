//! Themes tab: the ten embedded colour themes plus "Custom". Applying one
//! writes `~/.config/piuma/theme.json` — never `piuma.json`, whose colours
//! stay the user's own and come back with "Custom".

use iced::widget::{button, column, container, text, Column, Row};
use iced::{Background, Border, Color, Element, Length};

use crate::state::{Message, PiumaSetup};
use crate::style::*;
use crate::themes;

/// One card's colours: the bar background, the sensor pill and its text.
#[derive(Clone, Copy)]
struct Swatch {
    bg: Color,
    text: Color,
    pill_bg: Color,
    pill_text: Color,
}

fn parse(value: &Option<String>, fallback: Color) -> Color {
    value
        .as_deref()
        .and_then(|v| crate::config::parse_color(v).ok())
        .unwrap_or(fallback)
}

fn theme_swatch(theme: &themes::Theme) -> Swatch {
    let panel = &theme.panel;
    Swatch {
        bg: parse(&panel.default_bg_color, Color::BLACK),
        text: parse(&panel.default_text_color, Color::WHITE),
        pill_bg: parse(&panel.sensor_defaults.bg_color, Color::from_rgb(0.3, 0.3, 0.3)),
        pill_text: parse(&panel.sensor_defaults.text_color, Color::WHITE),
    }
}

/// The tab: a caption (the file it writes) and one row per theme, two
/// cards per line, each with a live mini-bar in its own colours.
pub(crate) fn themes_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut rows: Vec<Element<'_, Message>> = Vec::new();
    let mut line = Row::new().spacing(12);
    let mut cells = 0;
    for theme in themes::all() {
        line = line.push(cell(
            state,
            Some(theme.slug),
            &theme.name,
            theme_swatch(theme),
        ));
        cells += 1;
        if cells == 2 {
            rows.push(line.into());
            line = Row::new().spacing(12);
            cells = 0;
        }
    }
    let custom = Swatch {
        bg: pal.bg,
        text: pal.fg,
        pill_bg: pal.accent,
        pill_text: pal.accent_text,
    };
    line = line.push(cell(state, None, "Custom", custom));
    rows.push(line.into());

    let mut list = Column::new().spacing(12).width(Length::Fill);
    for row in rows {
        list = list.push(row);
    }

    column![
        text(
            "The active theme is saved in ~/.config/piuma/theme.json. Piuma reads it \
             at startup and live while running; piuma.json is never rewritten and \
             \"Custom\" restores your own colours."
        )
        .size(BODY)
        .style(muted_text(pal)),
        card(pal, list),
    ]
    .spacing(12)
    .width(Length::Fill)
    .into()
}

/// One theme card: name, Apply (or the active marker) and a mini-bar.
fn cell<'a>(
    state: &PiumaSetup,
    slug: Option<&'static str>,
    name: &'a str,
    swatch: Swatch,
) -> Element<'a, Message> {
    let pal = palette(&state.config);
    let active = state.active_theme == slug;
    let action: Element<'a, Message> = if active {
        text("✓ active").size(BODY).style(muted_text(pal)).into()
    } else {
        button(text("Apply").size(BODY))
            .on_press(Message::ThemeApplied(slug.map(str::to_string)))
            .padding(4)
            .style(move |t, s| btn_secondary(pal, t, s))
            .into()
    };
    let bar = container(
        Row::new()
            .push(
                container(text("Aa").size(11).color(swatch.pill_text))
                    .padding([1, 6])
                    .style(move |_t| container::Style {
                        background: Some(Background::Color(swatch.pill_bg)),
                        border: Border {
                            radius: 4.0.into(),
                            ..Default::default()
                        },
                        ..container::Style::default()
                    }),
            )
            .push(text("bar").size(12).color(swatch.text))
            .spacing(8)
            .align_y(iced::Alignment::Center),
    )
    .padding([0, 8])
    .width(Length::Fill)
    .height(Length::Fixed(28.0))
    .style(move |_t| container::Style {
        background: Some(Background::Color(swatch.bg)),
        border: Border {
            width: 1.0,
            color: Color { a: 0.35, ..swatch.text },
            radius: 6.0.into(),
        },
        ..container::Style::default()
    });

    container(
        column![
            Row::new()
                .push(text(name).size(BODY).width(Length::Fill))
                .push(action)
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .width(Length::Fill),
            bar,
        ]
        .spacing(6)
        .width(Length::Fill),
    )
    .padding(8)
    .width(Length::Fill)
    .style(move |_t| container::Style {
        background: Some(Background::Color(pal.chrome)),
        border: Border {
            width: 1.0,
            color: pal.card_border,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
