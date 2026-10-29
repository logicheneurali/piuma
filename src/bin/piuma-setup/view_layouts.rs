//! Layouts tab: the saved window layouts of the bar (piuma-layouts.json)
//! with a per-row delete and a delete-all — both behind the shared
//! confirmation modals (modal.rs).
use iced::widget::{button, column, container, text, Row};
use iced::{Alignment, Element, Length};

use crate::model::SavedLayout;
use crate::state::{Message, PiumaSetup};
use crate::style::*;

/// The "Layouts" block: one row per saved layout (label, window count,
/// delete) over the delete-all button; both open a confirmation modal.
pub(crate) fn layouts_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut list = column![].spacing(10);
    for layout in &state.layouts {
        list = list.push(layout_row(layout, pal));
    }
    let body: Element<Message> = if state.layouts.is_empty() {
        text("No saved layouts yet — click a layout sensor on the bar to save one.")
            .size(BODY)
            .style(muted_text(pal))
            .into()
    } else {
        column![
            list,
            button(text("Delete all").size(BODY))
                .on_press(Message::LayoutsClearPressed)
                .padding(10),
        ]
        .spacing(10)
        .into()
    };
    column![
        text("Layouts").size(16).style(fg_text(pal)),
        text("Saved window arrangements of the bar (piuma-layouts.json). A deletion reaches the running bar on its next restart.")
            .size(BODY)
            .style(muted_text(pal)),
        body,
    ]
    .spacing(10)
    .into()
}

/// One saved layout: a compact class summary, then one inset card per
/// class with its window titles. Classes stack vertically (the page
/// scrolls), so the list never turns into an unreadable side-by-side
/// line, however many classes a layout has.
fn layout_row(layout: &SavedLayout, pal: Palette) -> Element<'_, Message> {
    let summary = layout.class_summary();
    let title = if summary.is_empty() { "Layout".to_string() } else { summary };
    let header = Row::new()
        .push(
            column![
                text(title)
                    .size(BODY)
                    .style(fg_text(pal))
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::Word),
                text(format!(
                    "{} window{}",
                    layout.windows,
                    if layout.windows == 1 { "" } else { "s" }
                ))
                .size(BODY)
                .style(muted_text(pal)),
            ]
            .spacing(2)
            .width(Length::Fill),
        )
        .push(
            button(text("Delete").size(BODY))
                .on_press(Message::LayoutDeletePressed(layout.key.clone()))
                .padding(5),
        )
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let mut body = column![].spacing(6).width(Length::Fill);
    if layout.classes.is_empty() {
        body = body.push(text("No window metadata.").size(BODY).style(muted_text(pal)));
    }
    for group in &layout.classes {
        let mut titles = column![].spacing(2).width(Length::Fill);
        for title in &group.titles {
            titles = titles.push(
                text(if title.is_empty() { "(untitled)" } else { title.as_str() })
                    .size(BODY)
                    .style(muted_text(pal))
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::Word),
            );
        }
        body = body.push(
            container(
                column![
                    Row::new()
                        .push(
                            text(&group.class)
                                .size(BODY)
                                .style(fg_text(pal))
                                .width(Length::Fill)
                                .wrapping(iced::widget::text::Wrapping::Word),
                        )
                        .push(
                            text(format!("×{}", group.titles.len()))
                                .size(BODY)
                                .style(muted_text(pal)),
                        )
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .width(Length::Fill),
                    titles,
                ]
                .spacing(4)
                .width(Length::Fill),
            )
            .padding(8)
            .width(Length::Fill)
            .style(inset_style(pal)),
        );
    }

    card(
        pal,
        column![header, body].spacing(8).width(Length::Fill),
    )
    .into()
}
