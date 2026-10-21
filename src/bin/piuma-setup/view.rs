//! The root view: tab rail, active tab's card, footer with status and
//! actions, and the modal layer stacked on top.
use iced::widget::text::Style as TextStyle;
use iced::widget::{button, column, container, scrollable, stack, text, Column, Row};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Theme};

use crate::controls::content_scroll_id;
use crate::modal::modal_overlay;
use crate::style::*;
use crate::view_panel::*;
use crate::view_sections::*;
use crate::view_wallpaper::*;
use crate::state::{Message, PiumaSetup, Tab};

/// Root view: the scrollable column of nested cards with the action bar
/// underneath; a modal, when open, is stacked over everything on a dimmed layer.
pub(crate) fn view(state: &PiumaSetup) -> Element<'_, Message> {
    // Roadmap N4: a left tab rail picks the category and only that
    // category's card renders — the header (file info) and the footer
    // (Load/Save/status) stay visible on every tab.
    let pal = palette(&state.config);
    let content = column![config_header(state)]
        .push(active_card(state))
        .padding(20)
        .spacing(15)
        .width(Length::Fill);

    let base = container(
        column![
            Row::new()
                .push(sidebar(state))
                .push(
                    container(scrollable(content).id(content_scroll_id()).width(Length::Fill).height(Length::Fill))
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill),
            footer(state),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_t: &Theme| container::Style {
        background: Some(Background::Color(pal.bg)),
        ..container::Style::default()
    });

    // One stable root type: swapping Stack↔Container when a modal opens or
    // closes makes Tree::diff replace the whole widget tree (tag mismatch),
    // dropping the content scrollable's state — the page jumped back to the
    // top after every modal, e.g. after picking an icon. Keeping `base` at
    // stack index 0 in both states preserves the scroll position.
    let mut root = stack![base];
    if state.modal.is_some() {
        root = root.push(modal_overlay(state));
    }
    root.into()
}

/// The one card belonging to the active tab (roadmap N4).
fn active_card(state: &PiumaSetup) -> Element<'_, Message> {
    match state.active_tab {
        Tab::Panel => panel_settings(state),
        Tab::Notifications => notifications_settings(state),
        Tab::SensorDefaults => sensor_defaults_view(state),
        Tab::SectionDefaults => section_defaults_view(state),
        Tab::Sections => sections_list(state),
        Tab::Wallpaper => wallpaper_settings(state),
        Tab::AutostartApps => autostart_apps_settings(state),
    }
}

/// Left tab rail (roadmap N4): one button per category, the active one
/// highlighted; edits live in `state.config`, so switching tabs never
/// loses unsaved work.
fn sidebar(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut rail = Column::new().spacing(4).padding(Padding {
        top: 20.0,
        right: 10.0,
        bottom: 10.0,
        left: 10.0,
    });
    for tab in Tab::ALL {
        let selected = state.active_tab == tab;
        rail = rail.push(
            button(text(tab.label()).size(BODY))
                .on_press(Message::TabSelected(tab))
                .padding(8)
                .width(Length::Fill)
                .style(move |t, s| {
                    if selected {
                        btn_primary(pal, t, s)
                    } else {
                        btn_secondary(pal, t, s)
                    }
                }),
        );
    }
    container(rail)
        .width(Length::Fixed(170.0))
        .height(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(pal.chrome)),
            border: Border {
                width: 1.0,
                color: pal.card_border,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Fixed bottom bar: edited file + status on the left, actions on the right.
fn footer(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let info = column![
        text(format!("editing: {}", state.current_file))
            .size(BODY)
            .style(muted_text(pal)),
        status_bar(state),
    ]
    .spacing(2);

    let bar = Row::new()
        .push(container(info).width(Length::Fill))
        .push(action_buttons(pal))
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    container(bar)
        .padding(Padding { top: 10.0, right: 20.0, bottom: 10.0, left: 20.0 })
        .width(Length::Fill)
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(pal.chrome)),
            border: Border {
                width: 1.0,
                color: pal.card_border,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}


fn config_header(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let title = text("Piuma Configuration Setup")
        .size(22)
        .style(fg_text(pal));
    let subtitle = text("Configure your Piuma status bar")
        .size(14)
        .style(muted_text(pal));

    column![text(""), title, subtitle, text("")]
        .spacing(10)
        .align_x(Alignment::Center)
        .into()
}




/// Load / Save / Save As / Reset actions for the fixed bottom bar.
fn action_buttons(pal: Palette) -> Element<'static, Message> {
    let load_btn = button(text("Load…").size(BODY))
        .on_press(Message::LoadConfig)
        .padding(10)
        .style(move |t, s| btn_secondary(pal, t, s));
    let save_btn = button(text("Save").size(BODY))
        .on_press(Message::SaveConfig)
        .padding(10)
        .style(move |t, s| btn_primary(pal, t, s));
    let save_as_btn = button(text("Save As…").size(BODY))
        .on_press(Message::SaveAs)
        .padding(10)
        .style(move |t, s| btn_secondary(pal, t, s));
    let reset_btn = button(text("Reset").size(BODY))
        .on_press(Message::ResetConfig)
        .padding(10);

    Row::new()
        .push(load_btn)
        .push(save_btn)
        .push(save_as_btn)
        .push(reset_btn)
        .spacing(10)
        .into()
}

fn status_bar(state: &PiumaSetup) -> Element<'_, Message> {
    match &state.status_message {
        Some(msg) => {
            let is_error = state.status_is_error;
            text(msg).size(10).style(move |_t: &Theme| TextStyle { color: Some(if is_error { ERROR_RED } else { Color::from_rgb(0.3, 0.8, 0.4) }), ..TextStyle::default() }).into()
        }
        None => text("").into(),
    }
}
