//! Panel tab: the panel-wide card (font, palette, wallpaper and autostart
//! shortcuts) and the notifications card styling the popup.
use iced::widget::{column, text, toggler, Row, Slider};
use iced::{Element, Length};

use crate::controls::{
    border_type_options, color_control, form_grid, icon_theme_picker, input, invalid_under, picklist,
    slider_control,
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
    // Absent corners block = the bar's own fallback radius (config.rs).
    let corners_radius = state.config.screen_corners.as_ref().map_or(32, |c| c.radius);

    // Font Size / Thickness keep their u32 sliders inline: a shared generic
    // slider helper would need `num_traits` bounds, which this binary doesn't depend on.
    let fields = vec![
        ("Font", input("Font name", state.config.font.as_deref().unwrap_or(""), Message::FontChanged)
            .width(Length::Fill)
            .into()),
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
        ("Icon Theme", icon_theme_picker(state).into()),
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
        ("Default BG", color_control(
            state.config.default_bg_color.as_deref().unwrap_or(""),
            state.field_err("panel.default_bg_color"),
            Message::DefaultBgColorChanged,
        )),
        ("Default Text", color_control(
            state.config.default_text_color.as_deref().unwrap_or(""),
            state.field_err("panel.default_text_color"),
            Message::DefaultTextColorChanged,
        )),
        ("Anchor", invalid_under(
            picklist(
                vec!["top".to_string(), "bottom".to_string(), "left".to_string(), "right".to_string(),
                     "top-left".to_string(), "top-right".to_string(), "bottom-left".to_string(), "bottom-right".to_string()],
                Some(anchor),
                Message::AnchorChanged,
            )
            .width(Length::Fill)
            .into(),
            state.field_err("panel.anchor"),
        )),
        ("Align", invalid_under(
            picklist(
                vec!["left".to_string(), "center".to_string(), "right".to_string(), "fill".to_string()],
                Some(align),
                Message::AlignChanged,
            )
            .width(Length::Fill)
            .into(),
            state.field_err("panel.align"),
        )),
        // Rounded screen corners (roadmap angoli-schermo): an absent
        // block reads as "off" with the bar's own fallback values.
        ("Screen Corners", toggler(
            state.config.screen_corners.as_ref().map_or(false, |c| c.enable))
            .label("Round the screen corners")
            .on_toggle(Message::CornersEnabled)
            .into()),
        ("Corner Color", color_control(
            state.config.screen_corners.as_ref().map_or("", |c| c.color.as_str()),
            state.field_err("panel.screen_corners.color"),
            Message::CornersColorChanged,
        )),
        ("Corner Radius", invalid_under(
            Row::new()
                .push(Slider::new(1..=256, corners_radius, Message::CornersRadiusChanged).width(Length::Fill))
                .push(text(format!("{}px", corners_radius)).size(BODY).width(48))
                .spacing(8)
                .width(Length::Fill)
                .into(),
            state.field_err("panel.screen_corners.radius"),
        )),
    ];

    card(
        pal,
        column![
            text("Panel Settings").size(16).style(fg_text(pal)),
            form_grid(fields),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
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
                ("Position", picklist(
                    vec![
                        "auto".to_string(),
                        "top-left".to_string(),
                        "top-center".to_string(),
                        "top-right".to_string(),
                        "bottom-left".to_string(),
                        "bottom-center".to_string(),
                        "bottom-right".to_string(),
                    ],
                    Some(n.popup_position.clone()),
                    |value| Message::NotifPositionChanged(value),
                ).width(Length::Fill).into()),
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
