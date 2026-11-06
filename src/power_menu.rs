//! The power menu: lock/logout/suspend/reboot/shutdown, opened by the
//! `power` sensor's click. One small centered surface in the notification
//! popup's style, dismissed on leaving it or after a choice; every action
//! is a single spawn.

use iced::widget::{button, container, text, Column};
use iced::{Color, Element, Length};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;

use crate::menus::menu_row_h;
use crate::text_util::panel_font;
use crate::{popup_style, text_width, Command, Message, Panel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Lock,
    Logout,
    Suspend,
    Reboot,
    Shutdown,
}

const ROWS: [(Action, &str); 5] = [
    (Action::Lock, "Lock"),
    (Action::Logout, "Log out"),
    (Action::Suspend, "Suspend"),
    (Action::Reboot, "Reboot"),
    (Action::Shutdown, "Shut down"),
];

/// Vertical gap between two rows: tight, the menu is one block.
const GAP: u32 = 2;

/// The menu gives up on its own after this, so a click outside every
/// piuma surface cannot leave it pinned open.
const LINGER: std::time::Duration = std::time::Duration::from_secs(8);

/// The surface height: the rows plus the card padding and the gaps, so
/// the last row never touches the border.
fn menu_height(font_size: u32, padding: u32) -> u32 {
    let rows = ROWS.len() as u32;
    2 * padding + rows * menu_row_h(font_size) + (rows - 1) * GAP
}

/// The surface width: the longest label plus the card padding and a
/// little breathing room — a compact menu, not the notification popup's
/// text width.
fn menu_width(panel: &Panel, style: &crate::PopupStyle, padding: u32) -> u32 {
    let font = panel_font(&panel.config);
    let labels: Vec<String> = ROWS.iter().map(|(_, label)| label.to_string()).collect();
    let text = u32::from(text_width::widest(&font, style.font_size, &labels));
    (text + 2 * padding + 24).clamp(140, 320)
}

/// Open the menu, or close it when the sensor is clicked again.
pub(crate) fn open(panel: &mut Panel) -> Command<Message> {
    if panel.power_menu.is_some() {
        return close(panel);
    }
    let st = popup_style(&panel.config, panel.palette.as_ref());
    // A minimum card inset: the notification padding (often 4 px) is
    // too tight for a menu whose rows hover.
    let padding = st.padding.max(10);
    let height = menu_height(st.font_size, padding);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(menu_width(panel, &st, padding), height),
        layer: Layer::Overlay,
        // Opposite anchors with a fixed size centre the surface.
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: OutputOption::Active,
        events_transparent: false,
        namespace: Some("piuma-power".to_string()),
        blur_option: crate::glass_blur(&panel.config),
    });
    panel.power_menu = Some((id, std::time::Instant::now()));
    open
}

pub(crate) fn close(panel: &mut Panel) -> Command<Message> {
    panel
        .power_menu
        .take()
        .map(|(id, _)| iced::window::close(id))
        .unwrap_or_else(Command::none)
}

pub(crate) fn is_window(panel: &Panel, window: iced::window::Id) -> bool {
    panel.power_menu.is_some_and(|(id, _)| id == window)
}

/// Close the menu once its linger elapsed.
pub(crate) fn sync(panel: &mut Panel) -> Command<Message> {
    if panel.power_menu.is_some_and(|(_, opened)| {
        std::time::Instant::now().saturating_duration_since(opened) >= LINGER
    }) {
        return close(panel);
    }
    Command::none()
}

/// Whether the menu opened long enough ago that a bar click can close
/// it: the click that *opens* it also lands on the bar and must not
/// close it in the same gesture.
pub(crate) fn settled(panel: &Panel) -> bool {
    panel.power_menu.is_some_and(|(_, opened)| {
        std::time::Instant::now().saturating_duration_since(opened)
            > std::time::Duration::from_millis(400)
    })
}

/// The rows in the notification popup's style: same font, colours,
/// border, radius and padding, hover from the popup's hover pair. The
/// destructive entries are no longer tinted — the card reads as one
/// notification card.
pub(crate) fn view(panel: &Panel) -> Element<'_, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let row_h = menu_row_h(st.font_size) as f32;
    let mut list = Column::new().spacing(GAP).width(Length::Fill);
    for (action, label) in ROWS {
        list = list.push(
            button(
                text(label)
                    .size(st.font_size)
                    .font(font)
                    .color(st.text)
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center),
            )
            .on_press(Message::PowerAction(action))
            .height(Length::Fixed(row_h))
            .padding([0.0, st.padding.max(10) as f32])
            .width(Length::Fill)
            .style(move |_t, status| iced::widget::button::Style {
                background: Some(iced::Background::Color(match status {
                    iced::widget::button::Status::Hovered => st.bg_hover,
                    iced::widget::button::Status::Pressed => Color {
                        a: 0.8,
                        ..st.bg_hover
                    },
                    _ => Color::TRANSPARENT,
                })),
                text_color: match status {
                    iced::widget::button::Status::Hovered => st.text_hover,
                    _ => st.text,
                },
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
            background: Some(iced::Background::Color(st.bg)),
            border: iced::Border {
                width: st.border_width as f32,
                color: st.border,
                radius: (st.border_radius as f32).into(),
            },
            ..container::Style::default()
        })
        .into()
}
