//! Panel tab: the panel-wide card (font, palette, wallpaper and autostart
//! shortcuts) and the notifications card styling the popup.
use iced::widget::{button, column, container, scrollable, text, toggler, Column, Row, Slider};
use iced::{Element, Length};

use crate::controls::{
    align_row, anchor_grid, border_type_options, color_control, color_control_over, font_families,
    form_grid, icon_theme_options, input, invalid_under, monitor_outputs, notif_position_grid,
    options_with_current, picklist, slider_control, theme_cell, DEFAULT_OPTION,
    SYSTEM_ICON_THEME,
};
use crate::style::*;
use crate::state::{Message, PiumaSetup};

/// Panel-wide settings as a two-column form grid inside a card.
pub(crate) fn panel_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let font_size = state.config.font_size;
    let thickness = state.config.thickness;
    let anchor = state.config.anchor.clone().unwrap_or_default();
    let align = state.config.align.clone().unwrap_or_default();
    let value_effect = state
        .config
        .value_effect
        .clone()
        .unwrap_or_else(|| "off".to_string());
    // Font Size / Thickness keep their u32 sliders inline: a shared generic
    // slider helper would need `num_traits` bounds, which this binary doesn't depend on.
    let fields = vec![
        (
            "Font",
            font_cell(state.config.font.as_deref().unwrap_or("")),
        ),
        ("Font Size", invalid_under(
            Row::new()
                .push(Slider::new(8..=32, font_size, Message::FontSizeChanged).width(Length::Fill))
                .push(text(format!("{}pt", font_size)).size(BODY).width(40))
                .spacing(8)
                .width(Length::Fill)
                .into(),
            state.field_err("panel.font_size"),
        )),
        ("Bold", toggler(state.config.bold.unwrap_or(false))
            .label("Bold text")
            .on_toggle(Message::BoldChanged)
            .into()),
        // Autostart lives in its own tab now (roadmap2 #2).
        ("App Theme", toggler(state.config.app_theme.unwrap_or(false))
            .label("Generate GTK theme")
            .on_toggle(Message::AppThemeChanged)
            .into()),
        ("Thickness", invalid_under(
            Row::new()
                .push(Slider::new(8..=200, thickness, Message::ThicknessChanged).width(Length::Fill))
                .push(text(format!("{}px", thickness)).size(BODY).width(48))
                .spacing(8)
                .width(Length::Fill)
                .into(),
            state.field_err("panel.thickness"),
        )),
        ("Section Layout", invalid_under(
            picklist(
                vec!["horizontal".to_string(), "vertical".to_string()],
                Some(state.config.section_layout.clone()),
                Message::SectionLayoutChanged,
            )
            .width(Length::Fill)
            .into(),
            state.field_err("panel.section_layout"),
        )),
        ("Section Title", toggler(state.config.section_title.unwrap_or(true))
            .label("Show section titles")
            .on_toggle(Message::SectionTitleChanged)
            .into()),
        ("Value Effect", invalid_under(
            picklist(
                vec!["off".to_string(), "underline".to_string(), "sheen".to_string()],
                Some(value_effect),
                Message::ValueEffectChanged,
            )
            .width(Length::Fill)
            .into(),
            state.field_err("panel.value_effect"),
        )),
        ("Theme", invalid_under(
            theme_control(pal, state.config.theme_mode.as_deref().unwrap_or("dark")),
            state.field_err("panel.theme_mode"),
        )),
        ("Default BG", color_control_over(
            state.config.default_bg_color.as_deref().unwrap_or(""),
            state.field_err("panel.default_bg_color"),
            Some("#1a1b26"),
            Message::DefaultBgColorChanged,
            Some(Message::DefaultBgColorChanged(String::new())),
        )),
        ("Default Text", color_control_over(
            state.config.default_text_color.as_deref().unwrap_or(""),
            state.field_err("panel.default_text_color"),
            Some("#e8e8e8"),
            Message::DefaultTextColorChanged,
            Some(Message::DefaultTextColorChanged(String::new())),
        )),
        ("Anchor", invalid_under(
            anchor_grid(pal, &anchor),
            state.field_err("panel.anchor"),
        )),
        ("Align", invalid_under(
            align_row(pal, &align),
            state.field_err("panel.align"),
        )),
        ("Monitor", monitor_control(state)),
        ("Sections shown", input(
            "unassigned sections",
            &state.config.root_sections_text,
            Message::RootSectionsChanged,
        )
        .width(Length::Fill)
        .into()),
    ];

    card(
        pal,
        column![
            text("Panel Settings").size(16).style(fg_text(pal)),
            form_grid(fields),
            additional_panels(state),
            icon_theme_grid(state),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// Bar/app variant as two explicit choices (dark is the historic
/// default, light is the derived variant).
/// A small caption over one control of an extra-panel row.
fn panel_label<'a>(
    pal: Palette,
    label: &'a str,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        text(label).size(9).style(muted_text(pal)),
        control,
    ]
    .spacing(1)
    .width(Length::Fill)
    .into()
}

/// Extra panels: placement plus the comma-separated section names each
/// one shows. The first panel is the settings above; sections are shared.
fn additional_panels(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut list = Column::new().spacing(6).width(Length::Fill);
    if state.config.panels.is_empty() {
        list = list.push(
            text("No extra panels — the bar uses the settings above.")
                .size(10)
                .style(muted_text(pal)),
        );
    }
    for (index, panel) in state.config.panels.iter().enumerate() {
        let current_output = panel.output.clone().unwrap_or_else(|| "auto".to_string());
        let mut outputs = vec!["auto".to_string()];
        let detected = monitor_outputs();
        if current_output != "auto" && !detected.iter().any(|name| name == &current_output) {
            outputs.push(current_output.clone());
        }
        outputs.extend(detected);
        let sections = &panel.sections_text;
        let sections_error = state.field_err(&format!("panels[{index}].sections"));
        let row = Row::new()
            .push(panel_label(
                pal,
                "Output",
                picklist(outputs, Some(current_output), move |value: String| {
                    Message::PanelOutputChanged(index, value)
                })
                .width(Length::Fill)
                .into(),
            ))
            .push(panel_label(
                pal,
                "Anchor",
                picklist(
                    vec![
                        "default".to_string(),
                        "top".to_string(),
                        "bottom".to_string(),
                        "left".to_string(),
                        "right".to_string(),
                        "top-left".to_string(),
                        "top-right".to_string(),
                        "bottom-left".to_string(),
                        "bottom-right".to_string(),
                    ],
                    Some(panel.anchor.clone().unwrap_or_else(|| "default".to_string())),
                    move |value: String| Message::PanelAnchorChanged(index, value),
                )
                .width(Length::Fill)
                .into(),
            ))
            .push(panel_label(
                pal,
                "Align",
                picklist(
                    vec![
                        "default".to_string(),
                        "left".to_string(),
                        "center".to_string(),
                        "right".to_string(),
                        "fill".to_string(),
                    ],
                    Some(panel.align.clone().unwrap_or_else(|| "default".to_string())),
                    move |value: String| Message::PanelAlignChanged(index, value),
                )
                .width(Length::Fill)
                .into(),
            ))
            .push(
                button(text("Remove").size(BODY))
                    .on_press(Message::PanelRemoved(index))
                    .padding(5),
            )
            .spacing(6)
            .align_y(iced::Alignment::End);
        list = list.push(
            container(
                column![
                    row,
                    invalid_under(
                        input("required: section names", sections, move |value| {
                            Message::PanelSectionsChanged(index, value)
                        })
                        .width(Length::Fill)
                        .into(),
                        sections_error,
                    ),
                ]
                .spacing(4)
                .width(Length::Fill),
            )
            .padding(6)
            .width(Length::Fill)
            .style(inset_style(pal)),
        );
    }
    list = list.push(
        button(text("+ Add panel").size(BODY))
            .on_press(Message::PanelAdded)
            .padding(6),
    );
    column![
        text("Additional panels").size(12).style(fg_text(pal)),
        text("Sections are assigned exclusively: the main panel shows the ones not listed here; a secondary panel needs at least one. Comma-separated names; placement applies at the next piuma start (or via Reload Piuma in the footer). A panel anchored left or right is a vertical side bar.")
            .size(10)
            .style(muted_text(pal)),
        list,
    ]
    .spacing(6)
    .width(Length::Fill)
    .into()
}

/// The bar's output: `auto` (the focused monitor) or a detected name.
/// Applied when piuma starts; a layer surface cannot move outputs.
fn monitor_control(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let current = state
        .config
        .output
        .clone()
        .unwrap_or_else(|| "auto".to_string());
    let mut options = vec!["auto".to_string(), "all".to_string()];
    let detected = monitor_outputs();
    if current != "auto" && !detected.iter().any(|name| name == &current) {
        options.push(current.clone());
    }
    options.extend(detected);
    column![
        picklist(options, Some(current), Message::OutputChanged)
            .width(Length::Fill),
        text("auto = focused · all = every monitor · restart to apply")
            .size(10)
            .style(muted_text(pal)),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()
}

fn theme_control<'a>(pal: Palette, current: &str) -> Element<'a, Message> {
    let mut row = Row::new().spacing(4);
    for (label, value) in [("Dark", "dark"), ("Light", "light")] {
        let selected = current == value;
        row = row.push(
            button(text(label).size(BODY))
                .on_press(Message::ThemeModeChanged(value.to_string()))
                .padding(6)
                .style(move |t, s| {
                    if selected {
                        btn_primary(pal, t, s)
                    } else {
                        btn_secondary(pal, t, s)
                    }
                }),
        );
    }
    row.into()
}

/// Icon themes as a 3×3-visible grid of cells previewing three sample
/// icons in that theme; the rest scrolls.
fn icon_theme_grid(state: &PiumaSetup) -> Element<'_, Message> {
    use crate::icons;

    let pal = palette(&state.config);
    let current = state.config.icon_theme.as_deref().unwrap_or("");
    let mut options: Vec<Option<String>> = vec![None];
    options.extend(
        icon_theme_options(state.config.icon_theme.as_deref())
            .into_iter()
            .filter(|theme| theme != SYSTEM_ICON_THEME)
            .map(Some),
    );
    let mut grid = column![].spacing(6).width(Length::Fill);
    let mut row = Row::new().spacing(6).width(Length::Fill);
    let mut count = 0;
    for option in options {
        let label = option.clone().unwrap_or_else(|| {
            format!("{SYSTEM_ICON_THEME} ({})", icons::icon_theme())
        });
        let selected = option.as_deref().unwrap_or("") == current;
        row = row.push(theme_cell(pal, option.as_deref(), &label, selected));
        count += 1;
        if count % 3 == 0 {
            grid = grid.push(row);
            row = Row::new().spacing(6).width(Length::Fill);
        }
    }
    if count % 3 != 0 {
        grid = grid.push(row);
    }
    column![
        text("Icon Theme").size(12).style(fg_text(pal)),
        text("Three sample icons per theme, three rows visible — scroll for the rest.")
            .size(10)
            .style(muted_text(pal)),
        scrollable(grid)
            .height(Length::Fixed(200.0))
            .width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// The `panel.notifications` editor: the enable toggle is the block's
/// presence (it turns piuma's D-Bus server on/off), the three fields
/// are the block's settings; they only exist while the block does.
pub(crate) fn notifications_settings<'a>(state: &'a PiumaSetup) -> Element<'a, Message> {
    let pal = palette(&state.config);
    let enabled = state.config.notifications.is_some();
    let mut body = column![
        toggler(enabled)
            .label("Enable desktop notifications")
            .on_toggle(Message::NotificationsEnabled)
    ]
    .spacing(10);

    if let Some(n) = &state.config.notifications {
        let (timeout, history, replace) =
            (n.default_timeout_ms, n.history_size, n.replace_existing);
        body = body.push(form_grid(vec![
            ("Default Timeout", Row::new()
                .push(Slider::new(500..=60_000u32, timeout, Message::NotificationsTimeoutChanged).width(Length::Fill))
                .push(text(format!("{}ms", timeout)).size(BODY).width(64))
                .spacing(8)
                .width(Length::Fill)
                .into()),
            ("History Size", Row::new()
                .push(Slider::new(1..=100u32, history, Message::NotificationsHistoryChanged).width(Length::Fill))
                .push(text(format!("{}", history)).size(BODY).width(64))
                .spacing(8)
                .width(Length::Fill)
                .into()),
            ("App Updates", toggler(replace)
                .label("Update the existing card (no duplicates)")
                .on_toggle(Message::NotificationsReplaceChanged)
                .into()),
        ]));

        // Popup placement: where the cards appear on screen (roadmap M4) —
        // position picklist plus the two screen-edge margins.
        body = body
            .push(
                text("Popup placement — where the cards appear on screen")
                    .size(10)
                    .style(muted_text(pal)),
            )
            .push(form_grid(vec![
                ("Position", notif_position_grid(pal, &n.popup_position)),
                ("Margin H", slider_control(0..=100, n.popup_margin_h, state.field_err("panel.notifications.popup_margin_h"), |value| Message::NotifNumberChanged("popup_margin_h".to_string(), value))),
                ("Margin V", slider_control(0..=100, n.popup_margin_v, state.field_err("panel.notifications.popup_margin_v"), |value| Message::NotifNumberChanged("popup_margin_v".to_string(), value))),
            ]));

        // Popup style: the same controls as Sensor Defaults (colors, border,
        // padding) plus the font size; the defaults are white text on black
        // with a grey border and every key is written out explicitly, like
        // the server keys above.
        body = body
            .push(
                text("Popup style — how the notification cards look")
                    .size(10)
                    .style(muted_text(pal)),
            )
            .push(form_grid(vec![
                ("Font Size", slider_control(8..=32, n.font_size as i32, state.field_err("panel.notifications.font_size"), |value| Message::NotifNumberChanged("font_size".to_string(), value))),
                ("Text", color_control(&n.text_color, state.field_err("panel.notifications.text_color"), |value| Message::NotifStyleChanged("text_color".to_string(), value))),
                ("Hover Text", color_control(&n.text_color_hover, state.field_err("panel.notifications.text_color_hover"), |value| Message::NotifStyleChanged("text_color_hover".to_string(), value))),
                ("BG", color_control(&n.bg_color, state.field_err("panel.notifications.bg_color"), |value| Message::NotifStyleChanged("bg_color".to_string(), value))),
                ("Hover BG", color_control(&n.bg_color_hover, state.field_err("panel.notifications.bg_color_hover"), |value| Message::NotifStyleChanged("bg_color_hover".to_string(), value))),
                ("Border", color_control(&n.border_color, state.field_err("panel.notifications.border_color"), |value| Message::NotifStyleChanged("border_color".to_string(), value))),
                ("Border Hover", color_control(&n.border_color_hover, state.field_err("panel.notifications.border_color_hover"), |value| Message::NotifStyleChanged("border_color_hover".to_string(), value))),
                ("Border Width", slider_control(0..=10, n.border_width, state.field_err("panel.notifications.border_width"), |value| Message::NotifNumberChanged("border_width".to_string(), value))),
                ("Border Radius", slider_control(0..=20, n.border_radius, state.field_err("panel.notifications.border_radius"), |value| Message::NotifNumberChanged("border_radius".to_string(), value))),
                ("Padding", slider_control(0..=24, n.padding, state.field_err("panel.notifications.padding"), |value| Message::NotifNumberChanged("padding".to_string(), value))),
                ("Border Type", invalid_under(
                    picklist(
                        border_type_options(),
                        Some(n.border_type.clone()),
                        |value| Message::NotifStyleChanged("border_type".to_string(), value),
                    )
                    .width(Length::Fill)
                    .into(),
                    state.field_err("panel.notifications.border_type"),
                )),
            ]));
    }

    card(
        pal,
        column![
            text("Desktop Notifications").size(16).style(fg_text(pal)),
            body,
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// The Font cell: the installed families (the renderer's own list) next
/// to the free-text field, like the sensor-source combos. The sentinel
/// clears the key, so the bar's default family applies.
fn font_cell(current: &str) -> Element<'_, Message> {
    let selected = if current.is_empty() {
        DEFAULT_OPTION.to_string()
    } else {
        current.to_string()
    };
    let pick = picklist(
        options_with_current(font_families().to_vec(), current),
        Some(selected),
        |value: String| {
            Message::FontChanged(if value == DEFAULT_OPTION {
                String::new()
            } else {
                value
            })
        },
    )
    .width(Length::Fixed(170.0));
    Row::new()
        .push(pick)
        .push(input("Font name", current, Message::FontChanged).width(Length::Fill))
        .spacing(6)
        .width(Length::Fill)
        .into()
}
