//! The theme picker: one row per embedded theme plus "Custom", opened by
//! the `theme` sensor's click. Choosing a row writes `theme.json` (never
//! `piuma.json`) and applies the colours to the live bar; the menu closes
//! on picking, on leaving it, or after its linger.

use iced::widget::{button, container, text, Column, Row, Space};
use iced::{Background, Color, Element, Length};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;

use crate::menus::menu_row_h;
use crate::text_util::panel_font;
use crate::{config, popup_style, text_width, themes, Command, Message, Panel};

/// Gap between two rows and the swatch's size.
const GAP: u32 = 2;
const SWATCH: (f32, f32) = (18.0, 9.0);

/// The menu gives up after this, like the power menu.
const LINGER: std::time::Duration = std::time::Duration::from_secs(12);

/// One row: `None` is "Custom" (the user's own colours).
fn rows() -> Vec<(Option<&'static str>, String, Color, Color)> {
    let mut rows = vec![(
        None,
        "Custom".to_string(),
        Color::from_rgb(0.16, 0.16, 0.16),
        Color::from_rgb(0.85, 0.85, 0.85),
    )];
    rows.extend(themes::all().iter().map(|theme| {
        let colour = |value: &Option<String>| {
            value
                .as_deref()
                .and_then(|v| config::parse_color(v).ok())
                .unwrap_or(Color::BLACK)
        };
        (
            Some(theme.slug),
            theme.name.clone(),
            colour(&theme.panel.default_bg_color),
            colour(&theme.panel.default_text_color),
        )
    }));
    rows
}

fn menu_height(font_size: u32, padding: u32) -> u32 {
    let rows = rows().len() as u32;
    2 * padding + rows * menu_row_h(font_size) + (rows - 1) * GAP
}

fn menu_width(panel: &Panel, style: &crate::PopupStyle, padding: u32) -> u32 {
    let font = panel_font(&panel.config);
    let labels: Vec<String> = rows().into_iter().map(|(_, name, _, _)| name).collect();
    let text = u32::from(text_width::widest(&font, style.font_size, &labels));
    (text + SWATCH.0 as u32 + 12 + 2 * padding + 24).clamp(160, 340)
}

/// Open the picker, or close it when the sensor is clicked again.
pub(crate) fn open(panel: &mut Panel) -> Command<Message> {
    if panel.theme_menu.is_some() {
        return close(panel);
    }
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let padding = st.padding.max(10);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(
            menu_width(panel, &st, padding),
            menu_height(st.font_size, padding),
        ),
        layer: Layer::Overlay,
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: OutputOption::Active,
        events_transparent: false,
        namespace: Some("piuma-theme".to_string()),
        blur_option: crate::glass_blur(&panel.config),
    });
    panel.theme_menu = Some((id, std::time::Instant::now()));
    open
}

pub(crate) fn close(panel: &mut Panel) -> Command<Message> {
    panel
        .theme_menu
        .take()
        .map(|(id, _)| iced::window::close(id))
        .unwrap_or_else(Command::none)
}

pub(crate) fn is_window(panel: &Panel, window: iced::window::Id) -> bool {
    panel.theme_menu.is_some_and(|(id, _)| id == window)
}

/// Close the menu once its linger elapsed.
pub(crate) fn sync(panel: &mut Panel) -> Command<Message> {
    if panel.theme_menu.is_some_and(|(_, opened)| {
        std::time::Instant::now().saturating_duration_since(opened) >= LINGER
    }) {
        return close(panel);
    }
    Command::none()
}

/// Whether the menu opened long enough ago that a bar click can close it
/// (the opening click also lands on the bar).
pub(crate) fn settled(panel: &Panel) -> bool {
    panel.theme_menu.is_some_and(|(_, opened)| {
        std::time::Instant::now().saturating_duration_since(opened)
            > std::time::Duration::from_millis(400)
    })
}

pub(crate) fn view(panel: &Panel) -> Element<'_, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let row_h = menu_row_h(st.font_size) as f32;
    let mut list = Column::new().spacing(GAP).width(Length::Fill);
    for (slug, name, bg, fg) in rows() {
        let active = slug == panel.active_theme;
        let label = if active {
            format!("✓ {name}")
        } else {
            name
        };
        let swatch: Element<'_, Message> = container(Space::new())
            .width(Length::Fixed(SWATCH.0))
            .height(Length::Fixed(SWATCH.1))
            .style(move |_t| container::Style {
                background: Some(Background::Color(bg)),
                border: iced::Border {
                    width: 1.0,
                    color: fg,
                    radius: 3.0.into(),
                },
                ..container::Style::default()
            })
            .into();
        let content: Element<'_, Message> = Row::new()
            .push(swatch)
            .push(
                text(label)
                    .size(st.font_size)
                    .font(font)
                    .color(if active { st.text_hover } else { st.text }),
            )
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into();
        list = list.push(
            button(content)
                .on_press(Message::ThemeSelected(slug.map(str::to_string)))
                .height(Length::Fixed(row_h))
                .padding([0.0, st.padding.max(10) as f32])
                .width(Length::Fill)
                .style(move |_t, status| iced::widget::button::Style {
                    background: Some(Background::Color(match status {
                        iced::widget::button::Status::Hovered => st.bg_hover,
                        iced::widget::button::Status::Pressed => Color {
                            a: 0.8,
                            ..st.bg_hover
                        },
                        _ => Color::TRANSPARENT,
                    })),
                    text_color: st.text,
                    border: iced::Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    container(list)
        .padding(st.padding.max(10) as f32)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(st.bg)),
            border: iced::Border {
                width: st.border_width as f32,
                color: st.border,
                radius: (st.border_radius as f32).into(),
            },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One Custom row plus one per embedded theme, each slug resolvable
    /// and its colours readable: the picker can never offer a theme the
    /// bar would reject.
    #[test]
    fn rows_cover_custom_and_every_theme() {
        let rows = rows();
        assert_eq!(rows.len(), themes::all().len() + 1);
        assert!(rows[0].0.is_none(), "Custom comes first");
        for (slug, name, _, _) in &rows[1..] {
            let theme = themes::find(slug.expect("theme slug")).expect("embedded theme");
            assert_eq!(&theme.name, name);
        }
    }
}
