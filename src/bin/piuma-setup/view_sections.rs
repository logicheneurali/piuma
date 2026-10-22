//! Sections tab: the accordion of section cards, the per-sensor form with
//! its source-type params and colour rules, and the two defaults cards
//! (per-sensor and per-section).
use iced::widget::{button, column, container, text, toggler, Column, Row};
use iced::{Alignment, Border, Element, Length, Theme};

use crate::controls::*;
use crate::model::*;
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
        ("BG", color_control(section.bg_color.as_deref().unwrap_or(""), state.field_err(&format!("panel.sections[{index}].bg_color")), move |value| Message::SectionBgColorChanged(index, value))),
        ("Text", color_control(section.text_color.as_deref().unwrap_or(""), state.field_err(&format!("panel.sections[{index}].text_color")), move |value| Message::SectionTextColorChanged(index, value))),
        ("Border", color_control(section.border_color.as_deref().unwrap_or(""), state.field_err(&format!("panel.sections[{index}].border_color")), move |value| Message::SectionStyleChanged(index, "border_color".to_string(), value))),
        ("Border Width", slider_control(-1i32..=10, section.border_width.unwrap_or(-1), state.field_err(&format!("panel.sections[{index}].border_width")), move |value| Message::SectionNumberChanged(index, "border_width".to_string(), value))),
        ("Border Radius", slider_control(-1i32..=20, section.border_radius.unwrap_or(-1), state.field_err(&format!("panel.sections[{index}].border_radius")), move |value| Message::SectionNumberChanged(index, "border_radius".to_string(), value))),
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
    let mut parts = vec![canonical_type(&sensor.source_type).to_string()];
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
    sensor: &'a Sensor,
    open: bool,
    style_open: bool,
    section_labels: Vec<String>,
) -> Element<'a, Message> {
    let title = sensor
        .name
        .clone()
        .unwrap_or_else(|| source_label(canonical_type(&sensor.source_type)).to_string());
    let mut header = Row::new();
    // The sensor's own icon leads the row, at the bar's row-height size.
    if let Some(icon) = resolved_icon(&state.config, icon_size, sensor.icon.as_deref()) {
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
        return card(pal, column![header].width(Length::Fill)).into();
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

    // Options = the canonical types, alphabetical (roadmap2 #6); an
    // unknown/typo'd type from the file joins the list — still sorted —
    // so the current value never falls out of the options.
    let source_types = available_source_types(&sensor.source_type);
    let source_type_picklist = picklist(
        source_types,
        Some(&sensor.source_type),
        move |value| Message::SensorSourceTypeChanged(section_index, sensor_index, value),
    ).width(130);

    let source_config: Element<Message> = match canonical_type(&sensor.source_type) {
        "net_state" | "used_netdown" | "used_netup" => input("Interface", sensor.source_config.interface.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "interface".to_string(), value))
            .width(Length::Fill)
            .into(),
        "perc_volume" => input("Device", sensor.source_config.device.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "device".to_string(), value))
            .width(Length::Fill)
            .into(),
        "clock" | "calendar" => input("Format", sensor.source_config.format.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "format".to_string(), value))
            .width(Length::Fill)
            .into(),
        "command" => input("Command", sensor.source_config.command.as_deref().unwrap_or(""), move |value| Message::SensorSourceTypeConfigChanged(section_index, sensor_index, "command".to_string(), value))
            .width(Length::Fill)
            .into(),
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

    let source = form_cell("Source", Row::new()
        .push(source_type_picklist)
        .push(source_config)
        .spacing(8)
        .width(Length::Fill)
        .into());

    // Icon name, live preview at the row's real size and a browse
    // modal: the preview resolves exactly like the bar, so it cannot lie.
    let icon = form_cell("Icon", Row::new()
        .push(input("icon name (theme)", sensor.icon.as_deref().unwrap_or(""), move |value| Message::SensorIconChanged(section_index, sensor_index, value))
            .width(Length::Fill))
        .push(button(text("…").size(BODY))
            .on_press(Message::BrowseIconsPressed(section_index, sensor_index))
            .padding(5)
            .style(move |t, s| btn_secondary(pal, t, s)))
        .push(icon_preview(&state.config, icon_size, sensor.icon.as_deref()))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into());

    let action = form_cell("Action", input("close_panel / command / ...", sensor.action.as_deref().unwrap_or(""), move |value| Message::SensorActionChanged(section_index, sensor_index, value))
        .width(Length::Fill)
        .into());

    // Hover trend popup; only the numeric sources (perc_/used_/temp_) graph.
    let draw_graph = form_cell("Draw graph", toggler(sensor.draw_graph)
        .on_toggle(move |value| Message::SensorDrawGraphChanged(section_index, sensor_index, value))
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

    // Collapsed header shows which overrides are actually set.
    let summary = style_summary(
        &[
            ("text", &sensor.text_color), ("hover-text", &sensor.text_color_hover),
            ("bg", &sensor.bg_color), ("hover-bg", &sensor.bg_color_hover),
            ("border", &sensor.border_color), ("border-hover", &sensor.border_color_hover),
            ("border-type", &sensor.border_type),
        ],
        &[
            ("brd", &sensor.border_width), ("rad", &sensor.border_radius), ("pad", &sensor.padding),
        ],
    );
    let style_header = collapsible_header(
        pal,
        "Style overrides".to_string(),
        summary,
        style_open,
        Message::ToggleSensorStyle(section_index, sensor_index),
    );

    // The header (title + controls) stays visible in both states, so an
    // expanded sensor can still be collapsed, reordered, copied, removed.
    let mut root = column![header, fields, source, icon, action, draw_graph]
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
        let fields = vec![
            // Text colour with the threshold-rules editor right after it:
            // the "…" opens the popup where colours and thresholds live.
            ("Text", Row::new()
                .push(color_control(sensor.text_color.as_deref().unwrap_or(""), state.field_err(&spath("text_color")), move |value| Message::SensorTextColorChanged(section_index, sensor_index, value)))
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
            ("Hover Text", color_control(sensor.text_color_hover.as_deref().unwrap_or(""), state.field_err(&spath("text_color_hover")), move |value| Message::SensorTextColorHoverChanged(section_index, sensor_index, value))),
            ("BG", color_control(sensor.bg_color.as_deref().unwrap_or(""), state.field_err(&spath("bg_color")), move |value| Message::SensorBgColorChanged(section_index, sensor_index, value))),
            ("Hover BG", color_control(sensor.bg_color_hover.as_deref().unwrap_or(""), state.field_err(&spath("bg_color_hover")), move |value| Message::SensorBgColorHoverChanged(section_index, sensor_index, value))),
            ("Border", color_control(sensor.border_color.as_deref().unwrap_or(""), state.field_err(&spath("border_color")), move |value| Message::SensorBorderColorChanged(section_index, sensor_index, value))),
            ("Border Hover", color_control(sensor.border_color_hover.as_deref().unwrap_or(""), state.field_err(&spath("border_color_hover")), move |value| Message::SensorBorderColorHoverChanged(section_index, sensor_index, value))),
            ("Border Width", slider_control(-1i32..=10, sensor.border_width.unwrap_or(-1), state.field_err(&spath("border_width")), move |value| Message::SensorBorderWidthChanged(section_index, sensor_index, value))),
            ("Border Radius", slider_control(-1i32..=20, sensor.border_radius.unwrap_or(-1), state.field_err(&spath("border_radius")), move |value| Message::SensorBorderRadiusChanged(section_index, sensor_index, value))),
            ("Padding", slider_control(-1i32..=20, sensor.padding.unwrap_or(-1), state.field_err(&spath("padding")), move |value| Message::SensorPaddingChanged(section_index, sensor_index, value))),
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

    card(pal, root).into()
}

/// The "Sensor Defaults" card: global style applied to every sensor,
/// collapsed by default because it only overrides group-wide values.
pub(crate) fn sensor_defaults_view(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let d = &state.config.sensor_defaults;

    let summary = style_summary(
        &[
            ("text", &d.text_color), ("hover-text", &d.text_color_hover),
            ("bg", &d.bg_color), ("hover-bg", &d.bg_color_hover),
            ("border", &d.border_color), ("border-hover", &d.border_color_hover),
            ("border-type", &d.border_type),
        ],
        &[
            ("brd", &d.border_width), ("rad", &d.border_radius), ("pad", &d.padding),
        ],
    );
    let header = collapsible_header(
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
            ("Text", color_control(d.text_color.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.text_color"), |value| Message::DefaultsStyleChanged("text_color".to_string(), value))),
            ("Hover Text", color_control(d.text_color_hover.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.text_color_hover"), |value| Message::DefaultsStyleChanged("text_color_hover".to_string(), value))),
            ("BG", color_control(d.bg_color.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.bg_color"), |value| Message::DefaultsStyleChanged("bg_color".to_string(), value))),
            ("Hover BG", color_control(d.bg_color_hover.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.bg_color_hover"), |value| Message::DefaultsStyleChanged("bg_color_hover".to_string(), value))),
            ("Border", color_control(d.border_color.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.border_color"), |value| Message::DefaultsStyleChanged("border_color".to_string(), value))),
            ("Border Hover", color_control(d.border_color_hover.as_deref().unwrap_or(""), state.field_err("panel.sensor_defaults.border_color_hover"), |value| Message::DefaultsStyleChanged("border_color_hover".to_string(), value))),
            ("Border Width", slider_control(-1i32..=10, d.border_width.unwrap_or(-1), state.field_err("panel.sensor_defaults.border_width"), |value| Message::DefaultsNumberChanged("border_width".to_string(), value))),
            ("Border Radius", slider_control(-1i32..=20, d.border_radius.unwrap_or(-1), state.field_err("panel.sensor_defaults.border_radius"), |value| Message::DefaultsNumberChanged("border_radius".to_string(), value))),
            ("Padding", slider_control(-1i32..=20, d.padding.unwrap_or(-1), state.field_err("panel.sensor_defaults.padding"), |value| Message::DefaultsNumberChanged("padding".to_string(), value))),
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

    let summary = style_summary(
        &[
            ("bg", &d.bg_color), ("text", &d.text_color),
            ("border", &d.border_color), ("border-type", &d.border_type),
        ],
        &[("brd", &d.border_width), ("rad", &d.border_radius)],
    );
    let header = collapsible_header(
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
            ("BG", color_control(d.bg_color.as_deref().unwrap_or(""), state.field_err("panel.section_defaults.bg_color"), |value| Message::SecDefaultsStyleChanged("bg_color".to_string(), value))),
            ("Text", color_control(d.text_color.as_deref().unwrap_or(""), state.field_err("panel.section_defaults.text_color"), |value| Message::SecDefaultsStyleChanged("text_color".to_string(), value))),
            ("Border", color_control(d.border_color.as_deref().unwrap_or(""), state.field_err("panel.section_defaults.border_color"), |value| Message::SecDefaultsStyleChanged("border_color".to_string(), value))),
            ("Border Width", slider_control(-1i32..=10, d.border_width.unwrap_or(-1), state.field_err("panel.section_defaults.border_width"), |value| Message::SecDefaultsNumberChanged("border_width".to_string(), value))),
            ("Border Radius", slider_control(-1i32..=20, d.border_radius.unwrap_or(-1), state.field_err("panel.section_defaults.border_radius"), |value| Message::SecDefaultsNumberChanged("border_radius".to_string(), value))),
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
