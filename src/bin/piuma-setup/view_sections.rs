//! Sections tab: the accordion of section cards, the per-sensor form with
//! its source-type params and colour rules, and the two defaults cards
//! (per-sensor and per-section).
use iced::widget::text::Style as TextStyle;
use iced::widget::{button, column, container, text, toggler, tooltip, Column, Row};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::config;
use crate::controls::*;
use crate::icons;
use crate::model::*;
use crate::preview;
use crate::style::*;
use crate::model::{Section, Sensor};
use crate::state::{Message, PiumaSetup};

/// The "Sections" block: one card per section, plus the add button.
pub(crate) fn sections_list(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut section_list = Column::new().spacing(15);
    for (i, section) in state.config.sections.iter().enumerate() {
        section_list = section_list.push(section_view(state, i, section));
    }

    let add_section = button(text("+ Add Section").size(BODY))
        .on_press(Message::AddSectionPressed)
        .padding(10);

    column![
        text("Sections").size(16).style(fg_text(pal)),
        section_list,
        add_section,
    ]
    .spacing(10)
    .into()
}

/// Section card (accordion): a compact header — toggle with the section
/// name and a layout/sensor summary, then order and remove — over a body
/// (name, settings, sensors) that collapses to keep the list scannable.
pub(crate) fn section_view<'a>(
    state: &'a PiumaSetup,
    index: usize,
    section: &'a Section,
) -> Element<'a, Message> {
    let pal = palette(&state.config);
    let collapsed = state.sections_collapsed.contains(&index);
    let title = section.name.clone().unwrap_or_else(|| format!("Section {}", index + 1));
    let layout = section.layout.as_deref().unwrap_or(&state.config.section_layout);
    let count = section.sensors.len();
    let summary = format!("{} · {} sensor{}", layout, count, if count == 1 { "" } else { "s" });
    let err_count = state.section_err_count(index);
    let header = Row::new()
        .push(collapsible_header(
            pal,
            title,
            summary,
            !collapsed,
            Message::ToggleSection(index),
        ));
    let badge: Element<'_, Message> = if err_count == 0 {
        text("").size(10).into()
    } else {
        // Badge visible even when the section is collapsed.
        text(format!("⚠ {err_count}"))
            .size(10)
            .style(error_text_style)
            .into()
    };
    let header = header.push(badge)
        .push(move_btn(pal, "▲", index > 0, Message::SectionMoveUp(index)))
        .push(move_btn(
            pal,
            "▼",
            index + 1 < state.config.sections.len(),
            Message::SectionMoveDown(index),
        ))
        .push({
            // The last section keeps its button visible but inert.
            let b = button(text("Remove").size(BODY)).padding(5);
            if state.config.sections.len() > 1 {
                b.on_press(Message::RemoveSectionPressed(index))
            } else {
                b
            }
        })
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if collapsed {
        return card(pal, column![header].width(Length::Fill)).into();
    }

    // Section labels for the sensors' "Move to" picklists: the number
    // prefix keeps duplicates and empty names distinguishable.
    let section_labels: Vec<String> = state
        .config
        .sections
        .iter()
        .enumerate()
        .map(|(n, sec)| format!("{} · {}", n + 1, sec.name.as_deref().unwrap_or("(unnamed)")))
        .collect();

    let name_row = form_cell(
        "Name",
        input(
            "Section name",
            section.name.as_deref().unwrap_or(""),
            move |value| Message::SectionNameChanged(index, value),
        )
        .width(Length::Fill)
        .into(),
    );
    let settings = form_grid(vec![
        ("Layout", invalid_under(
            picklist(
                vec!["horizontal".to_string(), "vertical".to_string()],
                section.layout.as_ref(),
                move |value| Message::SectionLayoutChangedIndex(index, value),
            )
            .width(Length::Fill)
            .into(),
            state.field_err(&format!("panel.sections[{index}].layout")),
        )),
        ("BG", color_control_over(
            section.bg_color.as_deref().unwrap_or(""),
            state.field_err(&format!("panel.sections[{index}].bg_color")),
            state.config.section_defaults.bg_color.as_deref(),
            move |value| Message::SectionBgColorChanged(index, value),
            Some(Message::SectionBgColorChanged(index, String::new())),
        )),
        ("Text", color_control_over(
            section.text_color.as_deref().unwrap_or(""),
            state.field_err(&format!("panel.sections[{index}].text_color")),
            state.config.section_defaults.text_color.as_deref(),
            move |value| Message::SectionTextColorChanged(index, value),
            Some(Message::SectionTextColorChanged(index, String::new())),
        )),
        ("Border", color_control_over(
            section.border_color.as_deref().unwrap_or(""),
            state.field_err(&format!("panel.sections[{index}].border_color")),
            state.config.section_defaults.border_color.as_deref(),
            move |value| Message::SectionStyleChanged(index, "border_color".to_string(), value),
            Some(Message::SectionStyleChanged(index, "border_color".to_string(), String::new())),
        )),
        ("Border Width", slider_control_over(
            -1i32..=10,
            section.border_width.unwrap_or(-1),
            state.field_err(&format!("panel.sections[{index}].border_width")),
            config::style_num(state.config.section_defaults.border_width).map(|n| n as i32),
            move |value| Message::SectionNumberChanged(index, "border_width".to_string(), value),
            Some(Message::SectionNumberChanged(index, "border_width".to_string(), -1)),
        )),
        ("Border Radius", slider_control_over(
            -1i32..=20,
            section.border_radius.unwrap_or(-1),
            state.field_err(&format!("panel.sections[{index}].border_radius")),
            config::style_num(state.config.section_defaults.border_radius).map(|n| n as i32),
            move |value| Message::SectionNumberChanged(index, "border_radius".to_string(), value),
            Some(Message::SectionNumberChanged(index, "border_radius".to_string(), -1)),
        )),
        ("Border Type", invalid_under(
            picklist(
                border_type_options(),
                Some(section.border_type.clone().unwrap_or_else(|| "full".to_string())),
                move |value| Message::SectionStyleChanged(index, "border_type".to_string(), value),
            )
            .width(Length::Fill)
            .into(),
            state.field_err(&format!("panel.sections[{index}].border_type")),
        )),
    ]);

    let mut sensor_list = Column::new().spacing(8).width(Length::Fill);
    for (j, sensor) in section.sensors.iter().enumerate() {
        sensor_list = sensor_list.push(sensor_view(
            pal,
            state,
            sensor_icon_size(&state.config, section, sensor),
            index,
            j,
            section.sensors.len(),
            section,
            sensor,
            state.sensors_open.contains(&(index, j)),
            state.sensor_style_open.contains(&(index, j)),
            section_labels.clone(),
        ));
    }
    sensor_list = sensor_list.push(
        button(text("+ Add Sensor").size(BODY))
            .on_press(Message::AddSensorPressed(index))
            .padding(6),
    );
    let sensors = container(sensor_list)
        .padding(10)
        .width(Length::Fill)
        .style(inset_style(pal));

    let body = card(
        pal,
        column![header, name_row, settings, sensors]
            .spacing(10)
            .width(Length::Fill),
    );
    if err_count == 0 {
        body.into()
    } else {
        // Red frame: the section holding invalid values reads at a glance
        // in the Sections tab, with the fields themselves highlighted.
        container(body)
            .style(move |_t: &Theme| container::Style {
                border: Border {
                    width: 2.0,
                    color: ERROR_RED,
                    radius: pal.radius.into(),
                },
                ..container::Style::default()
            })
            .into()
    }
}


/// Compact "type · 'param' · graph" summary of a sensor's collapsed row.
pub(crate) fn sensor_summary(sensor: &Sensor) -> String {
    let cfg = &sensor.source_config;
    let param = cfg
        .device
        .as_deref()
        .or(cfg.interface.as_deref())
        .or(cfg.format.as_deref())
        .or(cfg.command.as_deref())
        .or(cfg.env_var.as_deref())
        .or(cfg.static_text.as_deref())
        .filter(|p| !p.is_empty());
    let mut parts = vec![source_type_display(&sensor.source_type)];
    if let Some(p) = param {
        parts.push(format!("'{p}'"));
    }
    if sensor.draw_graph {
        parts.push("graph".to_string());
    }
    parts.join(" · ")
}

/// Sensor row (accordion): the compact header carries the order, duplicate
/// and remove controls; the form (name, description, source, graph, style)
/// is the body, expanded on demand. `section_labels` feeds the cross-
/// section "Move to" picklist (its number prefix is the 1-based index).
pub(crate) fn sensor_view<'a>(
    pal: Palette,
    state: &'a PiumaSetup,
    icon_size: u16,
    section_index: usize,
    sensor_index: usize,
    sensor_count: usize,
    section: &'a Section,
    sensor: &'a Sensor,
    open: bool,
    style_open: bool,
    section_labels: Vec<String>,
) -> Element<'a, Message> {
    let title = sensor
        .name
        .clone()
        .unwrap_or_else(|| source_label(canonical_type(&sensor.source_type)).to_string());
    // A volume meter shows the same static speaker as the bar when the
    // optional sensor icon is left empty.
    let default_icon = matches!(canonical_type(&sensor.source_type), "volume_meter")
        .then_some(icons::VOLUME_ICON);
    let icon_name = sensor.icon.as_deref().or(default_icon);
    let mut header = Row::new();
    // The sensor's own icon leads the row, at the bar's row-height size.
    if let Some(icon) = resolved_icon(&state.config, icon_size, icon_name) {
        header = header.push(icon);
    }
    let header = header
        .push(collapsible_header(
            pal,
            title,
            sensor_summary(sensor),
            open,
            Message::ToggleSensor(section_index, sensor_index),
        ))
        .push(move_btn(
            pal,
            "▲",
            sensor_index > 0,
            Message::SensorMoveUp(section_index, sensor_index),
        ))
        .push(move_btn(
            pal,
            "▼",
            sensor_index + 1 < sensor_count,
            Message::SensorMoveDown(section_index, sensor_index),
        ))
        .push(
            button(text("⧉").size(BODY))
                .on_press(Message::SensorDuplicated(section_index, sensor_index))
                .padding(5)
                .style(move |t, s| btn_secondary(pal, t, s)),
        )
        .push(
            button(text("Remove").size(BODY))
                .on_press(Message::RemoveSensorPressed(section_index, sensor_index))
                .padding(5),
        )
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if !open {
        return card(pal, column![header].width(Length::Fill))
            .id(sensor_card_id(section_index, sensor_index))
            .into();
    }

    // Identity fields: the name input carries the id that add/duplicate
    // focus, so renaming the fresh row is the next keystroke.
    let fields = form_grid(vec![
        ("Name", input("Name", sensor.name.as_deref().unwrap_or(""), move |value| {
            Message::SensorNameChanged(section_index, sensor_index, value)
        })
        .id(sensor_name_id(section_index, sensor_index))
        .width(Length::Fill)
        .into()),
        ("Description", input(
            "Description",
            sensor.description.as_deref().unwrap_or(""),
            move |value| Message::SensorDescriptionChanged(section_index, sensor_index, value),
        )
        .width(Length::Fill)
        .into()),
    ]);

    // The current type as its real preview card: pressing it reopens the
    // same visual picker the "Add Sensor" flow uses.
    let type_card = button(
        Row::new()
            .push(preview::kind_pill(&state.config, &sensor.source_type))
            .push(text(source_field_display(&sensor.source_type)).size(10))
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .on_press(Message::SensorTypePickerOpened(section_index, sensor_index))
    .padding(4)
    .style(move |t, s| btn_secondary(pal, t, s));
    let type_card = tooltip(
        type_card,
        container(text("Open the visual type picker").size(10))
            .padding(4)
            .style(move |_t: &Theme| container::Style {
                background: Some(Background::Color(pal.inset)),
                border: Border {
                    width: 1.0,
                    color: pal.card_border,
                    radius: 3.0.into(),
                },
                ..container::Style::default()
            }),
        tooltip::Position::Top,
    );

    // Parameter helpers: a pick list of what exists on this machine next
    // to the free-text field (the value is the same config key).
    let combo = |kind: &'static str,
                 placeholder: &'static str,
                 current: &'a str,
                 options: Vec<String>| {
        let selected = if current.is_empty() {
            DEFAULT_OPTION.to_string()
        } else {
            current.to_string()
        };
        let pick = picklist(options, Some(selected), move |value: String| {
            let value = if value == DEFAULT_OPTION { String::new() } else { value };
            Message::SensorSourceTypeConfigChanged(
                section_index,
                sensor_index,
                kind.to_string(),
                value,
            )
        })
        .width(Length::Fixed(170.0));
        let field = input(placeholder, current, move |value| {
            Message::SensorSourceTypeConfigChanged(
                section_index,
                sensor_index,
                kind.to_string(),
                value,
            )
        })
        .width(Length::Fill);
        Row::new()
            .push(pick)
            .push(field)
            .spacing(6)
            .width(Length::Fill)
            .into()
    };

    let source_config: Element<Message> = match canonical_type(&sensor.source_type) {
        "net_state" | "used_netdown" | "used_netup" => {
            let current = sensor.source_config.interface.as_deref().unwrap_or("");
            combo(
                "interface",
                "Interface",
                current,
                options_with_current(network_interfaces(), current),
            )
        }
        "perc_volume" | "volume_meter" => {
            let current = sensor.source_config.device.as_deref().unwrap_or("");
            combo(
                "device",
                "Device",
                current,
                options_with_current(audio_devices(), current),
            )
        }
        "workspace_dots" => slider_control(
            1i32..=50,
            sensor.source_config.count.unwrap_or(10) as i32,
            None,
            move |value| Message::SensorSourceTypeConfigChanged(
                section_index,
                sensor_index,
                "count".to_string(),
                value.to_string(),
            ),
        ),
        "clock" | "calendar" => {
            let current = sensor.source_config.format.as_deref().unwrap_or("");
            let presets = if canonical_type(&sensor.source_type) == "clock" {
                clock_formats()
            } else {
                calendar_formats()
            };
            combo("format", "Format", current, options_with_current(presets, current))
        }
        "command" => {
            column![
                input("Command", sensor.source_config.command.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "command".to_string(), value))
                    .width(Length::Fill),
                input("Interval (s, 0 = every tick)", &sensor.source_config.interval_text, move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "interval".to_string(), value))
                    .width(Length::Fill),
                text("The command's first output line becomes the value, e.g. `date +%H:%M`. An interval keeps expensive commands off the 1 Hz loop.")
                    .size(10)
                    .style(muted_text(pal)),
            ]
            .spacing(2)
            .width(Length::Fill)
            .into()
        }
        "list" => {
            column![
                input("Command", sensor.source_config.command.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "command".to_string(), value))
                    .width(Length::Fill),
                input("Interval (s, 0 = every tick)", &sensor.source_config.interval_text, move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "interval".to_string(), value))
                    .width(Length::Fill),
                slider_control(
                    1i32..=200,
                    sensor.source_config.count.unwrap_or(10) as i32,
                    None,
                    move |value| Message::SensorSourceTypeConfigChanged(
                        section_index,
                        sensor_index,
                        "count".to_string(),
                        value.to_string(),
                    ),
                ),
                text("The sensor shows its name, or the command's first line when the name is empty. Hovering lists up to this many lines of its output.")
                    .size(10)
                    .style(muted_text(pal)),
            ]
            .spacing(2)
            .width(Length::Fill)
            .into()
        }
        "env_var" => input("Env Var", sensor.source_config.env_var.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "env_var".to_string(), value))
            .width(Length::Fill)
            .into(),
        "static_text" => input("Text", sensor.source_config.static_text.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "text".to_string(), value))
            .width(Length::Fill)
            .into(),
        other => Row::new()
            .push(text(format!("{} (no extra config)", source_label(other))).size(BODY))
            .spacing(5)
            .into(),
    };

    let source_path = format!(
        "panel.sections[{section_index}].sensors[{sensor_index}].source"
    );
    // A field-level source error (e.g. the command interval) is shown
    // under the same row as the source itself.
    let source_error = state
        .field_err(&format!("{source_path}.interval"))
        .or_else(|| state.field_err(&source_path));
    let source = form_cell("Source", invalid_under(
        Row::new()
            .push(type_card)
            .push(source_config)
            .spacing(8)
            .width(Length::Fill)
            .into(),
        source_error,
    ));

    // The live preview: the sensor as the bar would draw it, plus a
    // readability warning when the text disappears over its background.
    let hint = preview::contrast_hint(&state.config, Some(section), sensor);
    let preview_line = text(hint.clone().unwrap_or_default())
        .size(10)
        .style(move |_t: &Theme| TextStyle {
            color: hint.as_ref().map(|_| Color::from_rgb(1.0, 0.6, 0.2)),
            ..TextStyle::default()
        });
    let preview_block: Element<Message> = form_cell(
        "Preview",
        column![
            container(preview::sensor_preview(&state.config, Some(section), sensor))
                .padding(2),
            preview_line,
        ]
        .spacing(2)
        .into(),
    )
    .into();

    // Icon name, live preview at the row's real size and a browse
    // modal: the preview resolves exactly like the bar, so it cannot lie.
    let icon_error = state.field_err(&format!(
        "panel.sections[{section_index}].sensors[{sensor_index}].icon"
    ));
    let icon = form_cell("Icon", invalid_under(
        Row::new()
            .push(input("icon name (theme)", sensor.icon.as_deref().unwrap_or(""), move |value| Message::SensorIconChanged(section_index, sensor_index, value))
                .width(Length::Fill))
            .push(button(text("…").size(BODY))
                .on_press(Message::BrowseIconsPressed(section_index, sensor_index))
                .padding(5)
                .style(move |t, s| btn_secondary(pal, t, s)))
            .push(icon_preview(&state.config, icon_size, icon_name))
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into(),
        icon_error,
    ));

    let action = form_cell("Action", input("close_panel / command / ...", sensor.action.as_deref().unwrap_or(""), move |value| Message::SensorActionChanged(section_index, sensor_index, value))
        .width(Length::Fill)
        .into());

    // Hover trend popup; only the numeric sources (perc_/used_/temp_) graph.
    let draw_graph = form_cell("Draw graph", toggler(sensor.draw_graph)
        .on_toggle(move |value| Message::SensorDrawGraphChanged(section_index, sensor_index, value))
        .into());
    // Opt-in for `panel.value_effect`; the panel mode stays the master.
    let animate = form_cell("Animate", toggler(sensor.effect)
        .label("On value change")
        .on_toggle(move |value| Message::SensorEffectChanged(section_index, sensor_index, value))
        .into());

    // Cross-section ordering: only when there is somewhere else to go.
    let move_to = if section_labels.len() > 1 {
        Some(form_cell(
            "Move to",
            picklist(section_labels, None::<String>, move |label: String| {
                // The label prefix is the 1-based number of the target.
                let target = label
                    .split(' ')
                    .next()
                    .and_then(|n| n.parse::<usize>().ok())
                    .map(|n| n - 1)
                    .unwrap_or(section_index);
                Message::SensorMoveTo(section_index, sensor_index, target)
            })
            .width(Length::Fill)
            .into(),
        ))
    } else {
        None
    };

    // Collapsed header shows the overrides as coloured chips (a value is
    // recognisable at a glance, unlike a list of field names).
    let summary = style_chips(
        pal,
        &[
            ("text", &sensor.text_color), ("hover-text", &sensor.text_color_hover),
            ("bg", &sensor.bg_color), ("hover-bg", &sensor.bg_color_hover),
            ("border", &sensor.border_color), ("border-hover", &sensor.border_color_hover),
        ],
        &[
            ("brd", &sensor.border_width), ("rad", &sensor.border_radius), ("pad", &sensor.padding),
        ],
    );
    let style_header = collapsible_header_el(
        pal,
        "Style overrides".to_string(),
        summary,
        style_open,
        Message::ToggleSensorStyle(section_index, sensor_index),
    );

    // The header (title + controls) stays visible in both states, so an
    // expanded sensor can still be collapsed, reordered, copied, removed.
    let mut root = column![header, fields, source, preview_block, icon, action, draw_graph, animate]
        .spacing(6)
        .width(Length::Fill);
    if let Some(move_to) = move_to {
        root = root.push(move_to);
    }
    root = root.push(style_header);

    // Style numbers: -1 = disabled (the defaults apply), 0..=N = enabled.
    if style_open {
        let spath =
            |field: &str| format!("panel.sections[{section_index}].sensors[{sensor_index}].{field}");
        // The inherited value the bar would use: shown grey in empty
        // fields and as "auto N" on disabled sliders, so an override is
        // visible before (and instead of) reading the defaults tab.
        let d = &state.config.sensor_defaults;
        let inh = |value: &Option<i32>| config::style_num(*value).map(|n| n as i32);
        let reset = |message: Message| Some(message);
        let fields = vec![
            // Text colour with the threshold-rules editor right after it:
            // the "…" opens the popup where colours and thresholds live.
            ("Text", Row::new()
                .push(color_control_over(
                    sensor.text_color.as_deref().unwrap_or(""),
                    state.field_err(&spath("text_color")),
                    d.text_color.as_deref(),
                    move |value| Message::SensorTextColorChanged(section_index, sensor_index, value),
                    reset(Message::SensorTextColorChanged(section_index, sensor_index, String::new())),
                ))
                .push(
                    button(text("…").size(BODY))
                        .on_press(Message::ColorRulesPressed(section_index, sensor_index))
                        .padding(5)
                        .style(move |t, s| btn_secondary(pal, t, s)),
                )
                .spacing(8)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .into()),
            ("Hover Text", color_control_over(
                sensor.text_color_hover.as_deref().unwrap_or(""),
                state.field_err(&spath("text_color_hover")),
                d.text_color_hover.as_deref(),
                move |value| Message::SensorTextColorHoverChanged(section_index, sensor_index, value),
                reset(Message::SensorTextColorHoverChanged(section_index, sensor_index, String::new())),
            )),
            ("BG", color_control_over(
                sensor.bg_color.as_deref().unwrap_or(""),
                state.field_err(&spath("bg_color")),
                d.bg_color.as_deref(),
                move |value| Message::SensorBgColorChanged(section_index, sensor_index, value),
                reset(Message::SensorBgColorChanged(section_index, sensor_index, String::new())),
            )),
            ("Hover BG", color_control_over(
                sensor.bg_color_hover.as_deref().unwrap_or(""),
                state.field_err(&spath("bg_color_hover")),
                d.bg_color_hover.as_deref(),
                move |value| Message::SensorBgColorHoverChanged(section_index, sensor_index, value),
                reset(Message::SensorBgColorHoverChanged(section_index, sensor_index, String::new())),
            )),
            ("Border", color_control_over(
                sensor.border_color.as_deref().unwrap_or(""),
                state.field_err(&spath("border_color")),
                d.border_color.as_deref(),
                move |value| Message::SensorBorderColorChanged(section_index, sensor_index, value),
                reset(Message::SensorBorderColorChanged(section_index, sensor_index, String::new())),
            )),
            ("Border Hover", color_control_over(
                sensor.border_color_hover.as_deref().unwrap_or(""),
                state.field_err(&spath("border_color_hover")),
                d.border_color_hover.as_deref(),
                move |value| Message::SensorBorderColorHoverChanged(section_index, sensor_index, value),
                reset(Message::SensorBorderColorHoverChanged(section_index, sensor_index, String::new())),
            )),
            ("Border Width", slider_control_over(
                -1i32..=10,
                sensor.border_width.unwrap_or(-1),
                state.field_err(&spath("border_width")),
                inh(&d.border_width),
                move |value| Message::SensorBorderWidthChanged(section_index, sensor_index, value),
                reset(Message::SensorBorderWidthChanged(section_index, sensor_index, -1)),
            )),
            ("Border Radius", slider_control_over(
                -1i32..=20,
                sensor.border_radius.unwrap_or(-1),
                state.field_err(&spath("border_radius")),
                inh(&d.border_radius),
                move |value| Message::SensorBorderRadiusChanged(section_index, sensor_index, value),
                reset(Message::SensorBorderRadiusChanged(section_index, sensor_index, -1)),
            )),
            ("Padding (h)", slider_control_over(
                -1i32..=20,
                sensor.padding.unwrap_or(-1),
                state.field_err(&spath("padding")),
                inh(&d.padding),
                move |value| Message::SensorPaddingChanged(section_index, sensor_index, value),
                reset(Message::SensorPaddingChanged(section_index, sensor_index, -1)),
            )),
            ("Border Type", invalid_under(
                picklist(
                    border_type_options(),
                    Some(sensor.border_type.clone().unwrap_or_else(|| "full".to_string())),
                    move |value| Message::SensorBorderTypeChanged(section_index, sensor_index, value),
                )
                .width(Length::Fill)
                .into(),
                state.field_err(&spath("border_type")),
            )),
        ];
        root = root.push(
            container(form_grid(fields))
                .padding(10)
                .width(Length::Fill)
                .style(inset_style(pal)),
        );
    }

    card(pal, root)
        .id(sensor_card_id(section_index, sensor_index))
        .into()
}

/// The "Sensor Defaults" card: global style applied to every sensor,
/// collapsed by default because it only overrides group-wide values.
pub(crate) fn sensor_defaults_view(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let d = &state.config.sensor_defaults;

    let summary = style_chips(
        pal,
        &[
            ("text", &d.text_color), ("hover-text", &d.text_color_hover),
            ("bg", &d.bg_color), ("hover-bg", &d.bg_color_hover),
            ("border", &d.border_color), ("border-hover", &d.border_color_hover),
        ],
        &[
            ("brd", &d.border_width), ("rad", &d.border_radius), ("pad", &d.padding),
        ],
    );
    let header = collapsible_header_el(
        pal,
        "Sensor Defaults".to_string(),
        summary,
        state.defaults_expanded,
        Message::ToggleDefaults,
    );

    let mut root = column![header].spacing(10).width(Length::Fill);
    if state.defaults_expanded {
        root = root.push(
            text("Style applied to every sensor; single sensors override these values.")
                .size(10)
                .style(muted_text(pal)),
        );

        // Style numbers: -1 = disabled (the defaults apply), 0..=N = enabled.
        let fields = vec![
            ("Text", color_control_over(d.text_color.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.text_color"), None, |value| Message::DefaultsStyleChanged("text_color".to_string(), value), Some(Message::DefaultsStyleChanged("text_color".to_string(), String::new())))),
            ("Hover Text", color_control_over(d.text_color_hover.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.text_color_hover"), None, |value| Message::DefaultsStyleChanged("text_color_hover".to_string(), value), Some(Message::DefaultsStyleChanged("text_color_hover".to_string(), String::new())))),
            ("BG", color_control_over(d.bg_color.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.bg_color"), None, |value| Message::DefaultsStyleChanged("bg_color".to_string(), value), Some(Message::DefaultsStyleChanged("bg_color".to_string(), String::new())))),
            ("Hover BG", color_control_over(d.bg_color_hover.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.bg_color_hover"), None, |value| Message::DefaultsStyleChanged("bg_color_hover".to_string(), value), Some(Message::DefaultsStyleChanged("bg_color_hover".to_string(), String::new())))),
            ("Border", color_control_over(d.border_color.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.border_color"), None, |value| Message::DefaultsStyleChanged("border_color".to_string(), value), Some(Message::DefaultsStyleChanged("border_color".to_string(), String::new())))),
            ("Border Hover", color_control_over(d.border_color_hover.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.border_color_hover"), None, |value| Message::DefaultsStyleChanged("border_color_hover".to_string(), value), Some(Message::DefaultsStyleChanged("border_color_hover".to_string(), String::new())))),
            ("Border Width", slider_control_over(-1i32..=10, d.border_width.unwrap_or(-1), state.field_err("panel.sensor_defaults.border_width"), None, |value| Message::DefaultsNumberChanged("border_width".to_string(), value), Some(Message::DefaultsNumberChanged("border_width".to_string(), -1)))),
            ("Border Radius", slider_control_over(-1i32..=20, d.border_radius.unwrap_or(-1), state.field_err("panel.sensor_defaults.border_radius"), None, |value| Message::DefaultsNumberChanged("border_radius".to_string(), value), Some(Message::DefaultsNumberChanged("border_radius".to_string(), -1)))),
            ("Padding (h)", slider_control_over(-1i32..=20, d.padding.unwrap_or(-1), state.field_err("panel.sensor_defaults.padding"), None, |value| Message::DefaultsNumberChanged("padding".to_string(), value), Some(Message::DefaultsNumberChanged("padding".to_string(), -1)))),
            ("Border Type", invalid_under(
                picklist(
                    border_type_options(),
                    Some(d.border_type.clone().unwrap_or_else(|| "full".to_string())),
                    |value| Message::DefaultsStyleChanged("border_type".to_string(), value),
                )
                .width(Length::Fill)
                .into(),
                state.field_err("panel.sensor_defaults.border_type"),
            )),
        ];
        root = root.push(form_grid(fields));
    }

    card(pal, root).into()
}

/// The "Section Defaults" card: style applied to every section, overridden
/// per section (same group rule as the sensor defaults).
pub(crate) fn section_defaults_view(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let d = &state.config.section_defaults;

    let summary = style_chips(
        pal,
        &[
            ("bg", &d.bg_color), ("text", &d.text_color),
            ("border", &d.border_color), ("border-type", &d.border_type),
        ],
        &[("brd", &d.border_width), ("rad", &d.border_radius)],
    );
    let header = collapsible_header_el(
        pal,
        "Section Defaults".to_string(),
        summary,
        state.sec_defaults_expanded,
        Message::ToggleSecDefaults,
    );

    let mut root = column![header].spacing(10).width(Length::Fill);
    if state.sec_defaults_expanded {
        root = root.push(
            text("Style applied to every section; single sections override these values.")
                .size(10)
                .style(muted_text(pal)),
        );

        // Style numbers: -1 = disabled (built-in fallback), 0..=N = enabled.
        let fields = vec![
            ("BG", color_control_over(d.bg_color.as_deref().unwrap_or(""), state.field_err("panel.section_defaults.bg_color"), None, |value| Message::SecDefaultsStyleChanged("bg_color".to_string(), value), Some(Message::SecDefaultsStyleChanged("bg_color".to_string(), String::new())))),
            ("Text", color_control_over(d.text_color.as_deref().unwrap_or(""), state.field_err("panel.section_defaults.text_color"), None, |value| Message::SecDefaultsStyleChanged("text_color".to_string(), value), Some(Message::SecDefaultsStyleChanged("text_color".to_string(), String::new())))),
            ("Border", color_control_over(d.border_color.as_deref().unwrap_or(""), state.field_err("panel.section_defaults.border_color"), None, |value| Message::SecDefaultsStyleChanged("border_color".to_string(), value), Some(Message::SecDefaultsStyleChanged("border_color".to_string(), String::new())))),
            ("Border Width", slider_control_over(-1i32..=10, d.border_width.unwrap_or(-1), state.field_err("panel.section_defaults.border_width"), None, |value| Message::SecDefaultsNumberChanged("border_width".to_string(), value), Some(Message::SecDefaultsNumberChanged("border_width".to_string(), -1)))),
            ("Border Radius", slider_control_over(-1i32..=20, d.border_radius.unwrap_or(-1), state.field_err("panel.section_defaults.border_radius"), None, |value| Message::SecDefaultsNumberChanged("border_radius".to_string(), value), Some(Message::SecDefaultsNumberChanged("border_radius".to_string(), -1)))),
            ("Border Type", invalid_under(
                picklist(
                    border_type_options(),
                    Some(d.border_type.clone().unwrap_or_else(|| "full".to_string())),
                    |value| Message::SecDefaultsStyleChanged("border_type".to_string(), value),
                )
                .width(Length::Fill)
                .into(),
                state.field_err("panel.section_defaults.border_type"),
            )),
            // Roadmap M3: only sides NOT flush with a screen edge get the
            // rounded corners; the flag comes from these defaults alone.
            ("Screen Edges", toggler(d.respect_screen_edges.unwrap_or(false))
                .label("Borders respect screen edges")
                .on_toggle(Message::SecDefaultsRespectEdges)
                .into()),
        ];
        root = root.push(form_grid(fields));
    }

    card(pal, root).into()
}
