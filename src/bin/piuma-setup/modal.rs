//! Modal dialogs stacked over the main view: save/load confirmations, the
//! section/sensor editors, the colour-rule list and the icon browser.
use iced::widget::text::Style as TextStyle;
use iced::widget::{button, column, container, scrollable, text, Row};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};
use crate::config;
use crate::icons;
use crate::controls::*;
use crate::model::*;
use crate::style::*;
use crate::state::{Message, PiumaSetup};

/// One editable threshold rule inside the `ColorRules` modal: strings
/// while typing (a keystroke never errors), parsed when Apply is pressed.
#[derive(Debug, Clone)]
pub struct RuleDraft {
    pub min: String,
    pub color: String,
}

/// Modal dialogs stacked over the main view (see `view`).
#[derive(Debug, Clone)]
pub enum Modal {
    /// Overwrite `current_file` with the modified configuration.
    ConfirmSave,
    /// Save-as form: typed file name and whether it already exists.
    SaveAs { name: String, exists: bool },
    /// Extra confirmation: the save-as target file already exists.
    ConfirmSaveAs(String),
    /// Load picker: selected file among the *.json of the config directory.
    LoadFile { selected: String, files: Vec<String> },
    /// New-section form: optional name (an empty one keeps the default).
    AddSection { name: String },
    /// New-sensor form: target section, picked type, optional name.
    AddSensor { section_index: usize, kind: String, name: String },
    /// Removal confirmation for one section (and its sensors).
    ConfirmRemoveSection(usize),
    /// Removal confirmation for one sensor.
    ConfirmRemoveSensor(usize, usize),
    /// Removal confirmation for one saved window layout (its key).
    ConfirmRemoveLayout(String),
    /// Removal confirmation for every saved window layout (the file).
    ConfirmClearLayouts,
    /// Icon browser: target sensor and the query filtering theme names.
    BrowseIcons { section_index: usize, sensor_index: usize, query: String },
    /// Threshold colour-rules editor: target sensor and the draft rows
    /// (parsed into `Sensor::color_rules` when the modal is confirmed).
    ColorRules { section_index: usize, sensor_index: usize, rules: Vec<RuleDraft> },
    /// Save refused: one human-readable line per invalid value (the
    /// fields they belong to are highlighted in red behind the modal).
    InvalidFields { errors: Vec<String> },
}

/// Dimmed full-window layer with the active modal card centered on top.
pub(crate) fn modal_overlay(state: &PiumaSetup) -> Element<'_, Message> {
    container(modal_card(state))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 })),
            ..container::Style::default()
        })
        .into()
}

/// The card of the currently open modal (an empty element when none).
pub(crate) fn modal_card(state: &PiumaSetup) -> Element<'_, Message> {
    // Errors are rendered inside the modal so they are visible over the overlay.
    let error = if state.status_is_error {
        state.status_message.clone()
    } else {
        None
    };
    let pal = palette(&state.config);

    match &state.modal {
        Some(Modal::ConfirmSave) => dialog_frame(
            pal,
            "Unsaved changes",
            text(format!("Overwrite {} with the current configuration?", state.current_file)).size(BODY).into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Overwrite"),
        ),
        Some(Modal::SaveAs { name, exists }) => {
            let mut body = column![
                input("my-styles.json", name, Message::ModalTextChanged)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .width(Length::Fill);
            if *exists {
                body = body.push(
                    text("File exists: it will be overwritten").size(10)
                        .style(|_t: &Theme| TextStyle { color: Some(Color::from_rgb(1.0, 0.6, 0.2)), ..TextStyle::default() }),
                );
            }
            dialog_frame(
                pal,
                "Save as",
                body.into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Save"),
            )
        }
        Some(Modal::ConfirmSaveAs(file)) => dialog_frame(
            pal,
            "File exists",
            text(format!("The file {} already exists. Overwrite it?", file)).size(BODY).into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Overwrite"),
        ),
        Some(Modal::LoadFile { selected, files }) => {
            // Files as buttons (not a pick list): every choice stays visible
            // and no dropdown overlay can swallow clicks around the dialog.
            let mut list = column![].spacing(6).width(Length::Fill);
            for f in files {
                let is_selected = *selected == *f;
                list = list.push(
                    button(text(f.clone()).size(BODY))
                        .on_press(Message::ModalTextChanged(f.clone()))
                        .padding(8)
                        .width(Length::Fill)
                        .style(move |t, s| {
                            if is_selected {
                                btn_primary(pal, t, s)
                            } else {
                                btn_secondary(pal, t, s)
                            }
                        }),
                );
            }
            let body: Element<Message> = if files.is_empty() {
                text("No .json files found in the config directory").size(BODY).into()
            } else {
                list.into()
            };
            dialog_frame(
                pal,
                "Load configuration",
                body,
                error,
                dialog_actions(pal, Message::ModalConfirm, "Load"),
            )
        }
        Some(Modal::AddSection { name }) => dialog_frame(
            pal,
            "New section",
            input("Name (optional)", name, Message::ModalTextChanged)
                .width(Length::Fill)
                .into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Add"),
        ),
        Some(Modal::AddSensor { kind, name, .. }) => {
            // Type combo of the canonical list, plus an optional name; the
            // muted line under the fields explains what the type does.
            let body = column![
                form_cell("Type", picklist(
                    available_source_types(kind),
                    Some(kind.clone()),
                    Message::ModalTypeChanged,
                ).width(Length::Fill).into()),
                form_cell("Name", input("optional", name, Message::ModalTextChanged)
                    .width(Length::Fill)
                    .into()),
                text(source_label(canonical_type(kind))).size(10).style(muted_text(pal)),
            ]
            .spacing(8)
            .width(Length::Fill);
            dialog_frame(
                pal,
                "New sensor",
                body.into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Add"),
            )
        }
        Some(Modal::ConfirmRemoveSection(index)) => {
            let section = &state.config.sections[*index];
            let name = section.name.as_deref().unwrap_or("(unnamed)");
            let count = section.sensors.len();
            dialog_frame(
                pal,
                "Remove section",
                text(format!(
                    "Remove \"{name}\" and its {count} sensor(s)? Changes apply on Save."
                ))
                .size(BODY)
                .into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Remove"),
            )
        }
        Some(Modal::ConfirmRemoveSensor(section_index, sensor_index)) => {
            let sensor = &state.config.sections[*section_index].sensors[*sensor_index];
            let name = sensor
                .name
                .clone()
                .unwrap_or_else(|| source_label(canonical_type(&sensor.source_type)).to_string());
            dialog_frame(
                pal,
                "Remove sensor",
                text(format!("Remove \"{name}\"? Changes apply on Save."))
                    .size(BODY)
                    .into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Remove"),
            )
        }
        Some(Modal::ConfirmRemoveLayout(key)) => {
            let label = state
                .layouts
                .iter()
                .find(|l| l.key == *key)
                .map(|l| l.label.clone())
                .unwrap_or_else(|| key.clone());
            dialog_frame(
                pal,
                "Delete layout",
                text(format!(
                    "Delete \"{label}\"? The running bar keeps it until restart."
                ))
                .size(BODY)
                .into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Delete"),
            )
        }
        Some(Modal::ConfirmClearLayouts) => dialog_frame(
            pal,
            "Delete all layouts",
            text("Delete every saved layout (the whole piuma-layouts.json)? The layout sensor turns off until the next save on the bar.")
                .size(BODY)
                .into(),
            error,
            dialog_actions(pal, Message::ModalConfirm, "Delete all"),
        ),
        Some(Modal::BrowseIcons { section_index, sensor_index, query }) => {
            // Theme names as a grid of live previews: type to narrow the
            // list, press a cell to pick; the directory walk is cached
            // per theme, so only filtering happens per keystroke.
            let theme = state
                .config
                .icon_theme
                .clone()
                .unwrap_or_else(|| icons::icon_theme().to_string());
            let names = theme_icon_names(&theme);
            let hits = filter_icon_names(&names, query);
            // Cells preview the sensor being edited: its row's icon size,
            // so the browser shows exactly what the bar will draw (the
            // indices were checked when the modal opened; 16 only keeps
            // the cells sane if that ever stops holding).
            let icon_size = state
                .config
                .sections
                .get(*section_index)
                .and_then(|sec| {
                    sec.sensors
                        .get(*sensor_index)
                        .map(|s| sensor_icon_size(&state.config, sec, s))
                })
                .unwrap_or(16);
            let mut grid = column![].spacing(8).width(Length::Fill);
            let mut cells = Row::new().spacing(8);
            for (i, name) in hits.iter().enumerate() {
                cells = cells.push(icon_grid_cell(
                    pal,
                    &state.config,
                    icon_size,
                    name,
                    *section_index,
                    *sensor_index,
                ));
                if (i + 1) % ICONS_PER_ROW == 0 {
                    grid = grid.push(cells);
                    cells = Row::new().spacing(8);
                }
            }
            if hits.len() % ICONS_PER_ROW != 0 {
                grid = grid.push(cells);
            }
            let results: Element<Message> = if hits.is_empty() {
                text("no match").size(BODY).into()
            } else {
                scrollable(grid.width(Length::Fill))
                    .height(Length::Fixed(320.0))
                    .width(Length::Fill)
                    .into()
            };
            let body = column![
                input("search (substring)", query, Message::ModalTextChanged).width(Length::Fill),
                text(format!(
                    "{} icons available · {} match",
                    names.len(),
                    count_icon_matches(&names, query)
                ))
                .size(10)
                .style(muted_text(pal)),
                results,
            ]
            .spacing(8)
            .width(Length::Fill);
            dialog_frame(
                pal,
                "Pick icon",
                body.into(),
                error,
                // A single close button: a picked cell already applies.
                Row::new().push(
                    button(text("Close").size(BODY))
                        .on_press(Message::ModalCancel)
                        .padding(8)
                        .style(move |t, s| btn_primary(pal, t, s)),
                ),
            )
        }
        Some(Modal::ColorRules { rules, .. }) => {
            // One row per rule: threshold `min`, colour with its live
            // swatch (same control as the Text field) and a remove button.
            // The hint spells the bar's rule so the editor cannot mislead.
            let mut rows = column![].spacing(8).width(Length::Fill);
            if rules.is_empty() {
                rows = rows.push(
                    text("No rules yet: the value keeps the sensor text colour.")
                        .size(10)
                        .style(muted_text(pal)),
                );
            }
            for (i, rule) in rules.iter().enumerate() {
                rows = rows.push(
                    Row::new()
                        .push(input("min", &rule.min, move |value| Message::ColorRuleMinChanged(i, value))
                            .width(Length::Fixed(90.0)))
                        .push(color_control(
                            &rule.color,
                            // Draft rows are not in the config yet: the same
                            // parser names the bad value live in the popup.
                            if config::parse_color(&rule.color).is_err() {
                                Some("not a valid colour (#rgb, #rrggbb, #rrggbbaa)")
                            } else {
                                None
                            },
                            move |value| Message::ColorRuleColorChanged(i, value),
                        ))
                        .push(
                            button(text("✕").size(BODY))
                                .on_press(Message::ColorRuleRemove(i))
                                .padding(5)
                                .style(move |t, s| btn_secondary(pal, t, s)),
                        )
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .width(Length::Fill),
                );
            }
            let body = column![
                text("While the value reaches `min` the text takes `color`: the highest rule wins, hover still overrides.")
                    .size(10)
                    .style(muted_text(pal)),
                rows,
                button(text("Add rule").size(BODY))
                    .on_press(Message::ColorRuleAdd)
                    .padding(8)
                    .style(move |t, s| btn_secondary(pal, t, s)),
            ]
            .spacing(10)
            .width(Length::Fill);
            dialog_frame(
                pal,
                "Threshold colours",
                body.into(),
                error,
                dialog_actions(pal, Message::ModalConfirm, "Apply"),
            )
        }
        Some(Modal::InvalidFields { errors }) => {
            // One red line per invalid value, scrollable when a hand-edited
            // file made many fields wrong: this list is the refusal reason.
            let mut list = column![].spacing(4).width(Length::Fill);
            for line in errors {
                list = list.push(
                    text(line.clone()).size(BODY).style(error_text_style),
                );
            }
            dialog_frame(
                pal,
                "Save blocked: invalid values",
                container(scrollable(list))
                    .max_height(280.0)
                    .into(),
                None,
                Row::new().push(
                    button(text("Close").size(BODY))
                        .on_press(Message::ModalCancel)
                        .padding(8)
                        .style(move |t, s| btn_secondary(pal, t, s)),
                ),
            )
        }
        None => text("").into(),
    }
}

/// Modal card frame: title, body, optional error hint, action buttons.
pub(crate) fn dialog_frame<'a>(
    pal: Palette,
    title: &'a str,
    body: Element<'a, Message>,
    error: Option<String>,
    actions: Row<'a, Message>,
) -> Element<'a, Message> {
    let mut col = column![text(title).size(16)]
        .spacing(10)
        .width(Length::Fixed(420.0));
    col = col.push(body);
    if let Some(e) = error {
        col = col.push(text(e).size(10)
            .style(error_text_style));
    }
    col = col.push(actions);

    container(col)
        .padding(16)
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(pal.card)),
            border: Border {
                width: 1.0,
                color: pal.card_border,
                radius: pal.radius.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Cancel + primary action button row shared by all modals.
pub(crate) fn dialog_actions<'a>(pal: Palette, action: Message, label: &'a str) -> Row<'a, Message> {
    Row::new()
        .push(
            button(text("Cancel").size(BODY))
                .on_press(Message::ModalCancel)
                .padding(8)
                .style(move |t, s| btn_secondary(pal, t, s)),
        )
        .push(
            button(text(label).size(BODY))
                .on_press(action)
                .padding(8)
                .style(move |t, s| btn_primary(pal, t, s)),
        )
        .spacing(10)
        .align_y(Alignment::Center)
}
