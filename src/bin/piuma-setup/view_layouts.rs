//! Layouts tab: the saved window layouts of the bar (piuma-layouts.json)
//! with a per-row delete and a delete-all — both behind the shared
//! confirmation modals (modal.rs).
use iced::widget::{button, column, text, Row};
use iced::{Alignment, Element, Length};

use crate::model::SavedLayout;
use crate::state::{Message, PiumaSetup};
use crate::style::*;

/// The "Layouts" block: one row per saved layout (label, window count,
/// delete) over the delete-all button; both open a confirmation modal.
pub(crate) fn layouts_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut list = column![].spacing(8);
    for layout in &state.layouts {
        list = list.push(layout_row(layout));
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

/// One saved layout: its windows as "class: title", the window count
/// and the delete button opening the confirmation modal.
fn layout_row(layout: &SavedLayout) -> Element<'_, Message> {
    Row::new()
        .push(
            text(format!(
                "{}  ·  {} window{}",
                layout.label,
                layout.windows,
                if layout.windows == 1 { "" } else { "s" }
            ))
            .size(BODY)
            .width(Length::Fill),
        )
        .push(
            button(text("Delete").size(BODY))
                .on_press(Message::LayoutDeletePressed(layout.key.clone()))
                .padding(5),
        )
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}
