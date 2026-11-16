//! The bar's widget tree: the bar and its sensors, sections and layout.
//!
//! This is the view half of the panel; `main.rs` keeps the update loop and
//! the daemon, and every helper it calls lives in the modules it needs.


use iced::widget::{container, text, Column, MouseArea, Row, Space};
use iced::{Alignment, Background, Color, Element, Length};
use iced::widget::container::Style as ContainerStyle;

use crate::corners::corners_view;
use crate::dashboard;
use crate::media_ui;
use crate::fill_sections::FillSections;
use crate::styles::panel_background;
use crate::geometry::{panel_vertical, section_touch_for};
use crate::icons::{SECTION_CONTENT_PAD, SECTION_CONTENT_PAD_NARROW};
use crate::hover::sensor_box_id;
use crate::popup_calendar::calendar_popup_view;
use crate::popup_graph::graph_popup_view;
use crate::popup_notifications::{notif_list_view, popup_view};
use crate::text_util::{panel_font, value_font};
use crate::tooltip::tooltip_window_view;
use crate::wallpaper_ui::wallpaper_view;
use crate::{
    bar_line_height, config, dashboard_view, effects, icons, osd,
    panel_anchor_for, panel_thickness, popup_anim, power_menu, screensaver, styled_box,
    sensors, taskbar_menu_view, text_width, theme_menu, tray, tray_menu_view, volume_ui, window_menu,
    windows, workspace,
    Message, Panel, SectionData, App, DEFAULT_FONT_SIZE,
    SCREEN_HEIGHT, SCREEN_WIDTH, SENSOR_SPACING,
};

/// The section name as the title-fit math sees it: empty when the
/// title is hidden by `panel.section_title` — no title means no
/// chrome on the thickness axis (and no drawn title at all).
fn title_fit_name<'a>(panel: &'a Panel, section: &'a SectionData) -> &'a str {
    if panel.config.panel.section_title.unwrap_or(true) {
        section.name.as_str()
    } else {
        ""
    }
}

/// Root view: pick the layout for this window, publish its logical size
/// (popup geometry reads the globals) and render the shared panel state.
pub(crate) fn view<'a>(app: &'a App, window: iced::window::Id) -> Element<'a, Message> {
    let index = app.windows.get(&window).copied().unwrap_or(0);
    let layout = app.layouts.get(index).unwrap_or(&app.layouts[0]);
    let (width, height) = app.screens.get(index).copied().unwrap_or((1920, 1080));
    unsafe {
        SCREEN_WIDTH = width;
        SCREEN_HEIGHT = height;
    }
    bar_view(&app.state, window, layout)
}

fn bar_view<'a>(
    panel: &'a Panel,
    window: iced::window::Id,
    layout: &'a config::PanelLayout,
) -> Element<'a, Message> {
    // Side windows render their own trees; only bar calls feed BAR_ID.
    if panel.wallpaper.iter().any(|(_, id)| *id == window) {
        return wallpaper_view(panel, window);
    }
    if panel.corners.iter().any(|(_, id)| *id == window) {
        return corners_view(panel, window);
    }
    if let Some((_, ghost)) = panel.closing.iter().find(|(id, _)| *id == window) {
        return popup_anim::ghost_view(panel, ghost);
    }
    if let Some((id, _)) = panel.tray_menu.as_ref() {
        if *id == window {
            return tray_menu_view(panel);
        }
    }
    if let Some((id, _)) = panel.notif_list.as_ref() {
        if *id == window {
            return notif_list_view(panel);
        }
    }
    if let Some((id, _)) = panel.taskbar_menu.as_ref() {
        if *id == window {
            return taskbar_menu_view(panel);
        }
    }
    if let Some(menu) = panel.window_menu.as_ref() {
        if menu.id == window {
            return window_menu::view(panel, menu);
        }
    }
    if let Some((id, _)) = panel.bar_tooltip.as_ref() {
        if *id == window {
            return tooltip_window_view(panel);
        }
    }
    if let Some(gp) = panel.graph_popup.as_ref() {
        if gp.id == window {
            return graph_popup_view(panel, gp);
        }
    }
    if let Some(cp) = panel.calendar.as_ref() {
        if cp.id == window {
            return calendar_popup_view(panel, cp);
        }
    }
    if power_menu::is_window(panel, window) {
        return power_menu::view(panel);
    }
    if theme_menu::is_window(panel, window) {
        return theme_menu::view(panel);
    }
    if dashboard::is_window(panel, window) {
        return dashboard_view::view(panel);
    }
    if osd::is_window(panel, window) {
        return osd::view(panel);
    }
    if screensaver::is_window(panel, window) {
        return screensaver::view(panel, window);
    }
    if panel.popup == Some(window) {
        return popup_view(panel);
    }
    let _ = BAR_ID.set(window);
    let font_obj = panel_font(&panel.config);
    // Value texts can ride a separate (typically monospaced) family.
    let value_font_obj = value_font(&panel.config);
    let font_size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);

    // Panel orientation: the `thickness` axis (the "cross" axis of the panel)
    // is the height for a horizontal panel and the width for a vertical one.
    let vertical = panel_vertical(&panel.config, layout.anchor.as_deref());
    // A side bar is narrow: spend less of its width on the content inset
    // so the icons keep their size (styles.rs sizes them to match).
    let content_pad = if vertical {
        SECTION_CONTENT_PAD_NARROW
    } else {
        SECTION_CONTENT_PAD
    };

    // Build sections.
    let mut section_elements: Vec<Element<'a, Message>> = Vec::new();

    let value_effect = panel
        .config
        .panel
        .value_effect
        .as_deref()
        .unwrap_or("off");
    // The sections assigned to this panel, in bar order. An empty list is
    // an empty panel (secondary panels must name at least one section).
    let visible: Vec<usize> = panel
        .sections
        .iter()
        .enumerate()
        .filter_map(|(index, section)| match &layout.sections {
            Some(names) => names.iter().any(|name| name == &section.name).then_some(index),
            None => Some(index),
        })
        .collect();
    // Screen-edge rounding follows THIS panel's anchor/align, not the
    // root panel's: a section moved to a bottom panel must square its
    // bottom (and only its bottom) side.
    let respect_edges = panel
        .config
        .panel
        .section_defaults
        .as_ref()
        .and_then(|defaults| defaults.respect_screen_edges)
        .unwrap_or(false);
    let touch_anchor = panel_anchor_for(&panel.config, layout.anchor.as_deref());
    let touch_align = layout.align.as_deref().unwrap_or("center");
    for (position, section_idx) in visible.iter().copied().enumerate() {
        let section = &panel.sections[section_idx];
        let touch = if respect_edges {
            section_touch_for(
                touch_anchor,
                touch_align,
                position,
                visible.len(),
            )
        } else {
            [false; 4]
        };
        let section_bg = section.bg_color;
        let section_text = section.text_color;
        // The `panel.section_title` toggle can hide the title entirely.
        let has_title = !title_fit_name(panel, section).is_empty();
        let title_color = section.title_color;

        // A vertical panel stacks every section's sensors down its height,
        // whatever the section's own layout says (a row would squeeze them
        // across the thickness): `stack_sensors` is that effective axis.
        let stack_sensors = vertical || section.layout == "vertical";
        // Sensor boxes fill the `thickness` axis only when the sensors run
        // along the main axis (row across a horizontal panel, column down a
        // vertical one); stacked along the thickness they stay content-sized
        // (filling would split the thickness between them) and the group is
        // centred by the container.
        let stretch = vertical || !stack_sensors;
        let (box_w, box_h) = match (vertical, stretch) {
            (false, true) => (Length::Shrink, Length::Fill),
            (true, true) => (Length::Fill, Length::Shrink),
            _ => (Length::Shrink, Length::Shrink),
        };

        // Build sensor widgets with optional per-sensor background. A tray
        // sensor with no items renders nothing at all (no empty box); the
        // notifications bell always shows (its counter is empty until the
        // next delivery).
        let sensor_widgets: Vec<Element<'a, Message>> = section.sensors.iter().enumerate().filter_map(|(sensor_idx, s)| {
            if s.tray.as_ref().is_some_and(|icons| icons.is_empty()) {
                return None;
            }
            let is_wifi = panel
                .config
                .panel
                .sections
                .get(section_idx)
                .and_then(|sec| sec.sensors.get(sensor_idx))
                .is_some_and(|cfg_s| matches!(cfg_s.source, config::SensorSource::Wifi {}));
            let is_media = panel
                .config
                .panel
                .sections
                .get(section_idx)
                .and_then(|sec| sec.sensors.get(sensor_idx))
                .is_some_and(|cfg_s| matches!(cfg_s.source, config::SensorSource::Media {}));
            // A list sensor shows the name when it has one, the first
            // output line otherwise — never both.
            let is_list = panel
                .config
                .panel
                .sections
                .get(section_idx)
                .and_then(|sec| sec.sensors.get(sensor_idx))
                .is_some_and(|cfg_s| matches!(cfg_s.source, config::SensorSource::List { .. }));
            if is_media && s.media.is_none() {
                return None;
            }
            // A workspace sensor with no buttons (hyprctl unreachable)
            // hides like the tray does (roadmap2 #4).
            if s.workspaces.as_ref().is_some_and(|b| b.is_empty()) {
                return None;
            }
            // Same rule for a taskbar whose client list came back empty
            // (hyprctl unreachable): no empty icon strip.
            if s.taskbar.as_ref().is_some_and(|slots| slots.is_empty()) {
                return None;
            }
            let sensor_text_color = s.text_color.as_ref().map(|c| *c).unwrap_or(section_text);

            // Text colour: a threshold rule the reading reaches (the
            // highest `min` ≤ value) tints the value; the hover variant
            // still wins while the cursor is over the sensor.
            let ruled = sensors::leading_number(&s.value)
                .and_then(|v| {
                    s.color_rules
                        .iter()
                        .rev()
                        .find(|(min, _)| v >= *min)
                        .map(|(_, c)| *c)
                })
                .unwrap_or(sensor_text_color);
            let is_hovered = panel.hovered == Some((section_idx, sensor_idx));
            let text_color = if is_hovered {
                s.text_color_hover.unwrap_or(ruled)
            } else {
                ruled
            };

            // The value text of net rates pins its box width (roadmap M5):
            // the container carries the fixed width so the text itself never
            // wraps, whatever the content; the text is centred inside it so
            // the value lands like every other sensor's value (no leading
            // padding shifts it off the line).
            let effect_progress = panel
                .value_effects
                .get(&(section_idx, sensor_idx))
                .and_then(|start| effects::progress(*start));
            let value_color = effect_progress
                .map(|t| effects::fade(text_color, t))
                .unwrap_or(text_color);
            let line_height =
                iced::widget::text::LineHeight::Absolute(bar_line_height(font_size).into());
            let value_text = text(&s.value)
                .color(value_color)
                .size(font_size)
                .font(value_font_obj)
                .line_height(line_height);
            let effect_value = s.effect
                && matches!(value_effect, effects::UNDERLINE | effects::SHEEN);
            let value_width = if effect_value {
                s.fixed_width.map(f32::from).unwrap_or_else(|| {
                    f32::from(text_width::widest(
                        &value_font_obj,
                        font_size,
                        std::slice::from_ref(&s.value),
                    ))
                })
            } else {
                0.0
            };
            let value_element: Element<'a, Message> = if effect_value
                && value_effect == effects::UNDERLINE
            {
                effects::underline(
                    value_text.into(),
                    value_width,
                    bar_line_height(font_size),
                    text_color,
                    effect_progress,
                )
            } else if effect_value && value_effect == effects::SHEEN {
                effects::sheen(
                    value_text.into(),
                    value_width,
                    bar_line_height(font_size),
                    text_color,
                    effect_progress,
                )
            } else {
                value_text.into()
            };
            let value_widget: Element<'a, Message> = match s.fixed_width {
                Some(width) => container(value_element)
                    .width(Length::Fixed(f32::from(width)))
                    .align_x(Alignment::Center)
                    .into(),
                None => value_element,
            };

            // Name and value side by side, separated by a small gap (the
            // colon separator was removed on request); an empty name (e.g.
            // static_text sensors) renders the value alone, with no gap.
            // A vertical panel stacks the value UNDER the name instead:
            // its width is there to spend, and one long line would leave
            // the value stranded at the far right of a thick side panel.
            // Tray sensors skip the text row: `tray::row` builds their
            // icon row with clicks, wheel and tooltip included. The style
            // passed here drives the tooltip, which mirrors the sensor box;
            // a transparent sensor shows the section behind it on the bar,
            // so the tooltip borrows that colour to stay legible anywhere.
            // Icon rows (tray, taskbar) flow inside their sensor box and
            // turn to the next line instead of growing past it: iced does
            // not clip a Row, an overflow would paint over the neighbours.
            // The room is the box's width — the thickness in a vertical
            // panel, the screen along a horizontal bar's main axis (the
            // bar spans it) — net of the box's horizontal inset only (the
            // wrap runs along it).
            // The sensor's padding is horizontal only: the content is
            // always centred on the thickness axis, whatever the box's
            // size, so a large padding can never squeeze the text to
            // nothing on a thin bar. Workspace buttons carry the inset
            // themselves (roadmap2 #4), so their box drops it.
            let pad_h = if s.workspaces.is_some() {
                0
            } else {
                s.padding
            };
            let icon_room = (if vertical {
                panel_thickness(&panel.config)
            } else {
                unsafe { SCREEN_WIDTH }
            })
            .saturating_sub(2 * (u32::from(content_pad) + u32::from(pad_h)));
            let is_volume_meter = panel
                .config
                .panel
                .sections
                .get(section_idx)
                .and_then(|section| section.sensors.get(sensor_idx))
                .is_some_and(|sensor| {
                    matches!(sensor.source, config::SensorSource::VolumeMeter { .. })
                });
            let workspace_dots = panel
                .config
                .panel
                .sections
                .get(section_idx)
                .and_then(|section| section.sensors.get(sensor_idx))
                .and_then(|sensor| match &sensor.source {
                    config::SensorSource::WorkspaceDots { count } => Some(*count),
                    _ => None,
                });
            let main: Element<'a, Message> = if let Some(icons) = &s.tray {
                let per_line =
                    icons::icons_per_line(icon_room, tray::slot_width(), u32::from(tray::ROW_GAP));
                tray::row(icons, per_line)
            } else if let Some(buttons) = &s.workspaces {
                // One button per available workspace (roadmap2 #4): the
                // names shown, the active one inverted, click focuses —
                // the buttons own their mouse handling like tray icons
                // and wrap into as many lines as needed (like the tray
                // row), from the widest name's estimated width. The
                // sensor's own icon still leads the row (its config
                // predates the buttons), centred like every other sensor
                // row so it never rides above the baseline.
                let per_line = icons::icons_per_line(
                    icon_room,
                    workspace::slot_width(buttons, font_size, s.padding),
                    u32::from(workspace::ROW_GAP),
                );
                let ws_row = workspace::row(
                    buttons,
                    font_size,
                    font_obj,
                    &workspace::Style {
                        bg_color: s.bg_color.unwrap_or(section_bg),
                        text_color,
                        border_color: s.border_color,
                        border_width: s.border_width,
                        border_radius: s.border_radius,
                        border_type: s.border_type.clone(),
                        padding: s.padding,
                    },
                    per_line,
                );
                let mut row = Row::new().spacing(6).align_y(Alignment::Center);
                if let Some(path) = &s.icon {
                    row = row.push(icons::widget(path, s.icon_size as f32));
                }
                row.push(ws_row).into()
            } else if let Some(slots) = &s.taskbar {
                // One icon per open window: the slots own their mouse
                // handling like tray icons (left focuses, middle closes,
                // right opens the move menu); their fills and tooltip
                // come from the sensor's own colours — the active
                // window keeps the sensor's background, the hovered
                // slot wins over it.
                let active_bg = s.bg_color.or(Some(section_bg));
                let row_style = windows::RowStyle {
                    radius: s.border_radius,
                    hover_bg: s.bg_color_hover.or(active_bg),
                    active_bg,
                };
                let per_line = icons::icons_per_line(
                    icon_room,
                    windows::slot_width(),
                    u32::from(windows::ROW_GAP),
                );
                windows::row(slots, panel.taskbar_hover.as_deref(), &row_style, per_line)
            } else if is_volume_meter {
                // Graphical volume meter: speaker, optional name and the
                // click/drag bar. The percentage is added only while the
                // sensor is hovered, which widens the box on demand.
                let mut row = Row::new().spacing(6).align_y(Alignment::Center);
                if let Some(path) = &s.icon {
                    row = row.push(icons::widget(path, s.icon_size as f32));
                }
                if !s.name.is_empty() {
                    row = row.push(
                        text(&s.name)
                            .color(text_color)
                            .size(font_size)
                            .font(font_obj),
                    );
                }
                row = row.push(volume_ui::bar(
                    section_idx,
                    sensor_idx,
                    &s.value,
                    text_color,
                ));
                if is_hovered {
                    row = row.push(volume_ui::percent(
                        &s.value,
                        text_color,
                        font_obj,
                        font_size,
                    ));
                }
                row.into()
            } else if let Some(media) = &s.media {
                // MPRIS transport: previous, play/pause and next as
                // buttons (dimmed when the player disables them), then
                // the track text; the hover popup carries the full
                // title and the position/length.
                let mut row = Row::new().spacing(6).align_y(Alignment::Center);
                if let Some(path) = &s.icon {
                    row = row.push(icons::widget(path, s.icon_size as f32));
                }
                row = row.push(media_ui::row(media, font_size, font_obj, text_color));
                row.into()
            } else if let Some(count) = workspace_dots {
                // Purely graphical workspace switch: one circle per
                // numbered workspace, the active one filled.
                workspace::dots(panel.hypr.as_ref(), count, text_color, font_size)
            } else {
                // Icon first when the sensor declares one (roadmap N1), then
                // the optional name, then the value: an empty name still
                // renders the value alone, without a gap. The row centres
                // its children: the icon box is `font_size` square while a
                // text line box is `font_size × 1.3` (iced's default line
                // height), so a top-aligned row left the icon riding ~2px
                // above the text baseline (roadmap2: icon alignment).
                let mut row = Row::new().spacing(6).align_y(Alignment::Center);
                if let Some(path) = &s.icon {
                    row = row.push(icons::widget(path, s.icon_size as f32));
                }
                // An icon-only text/icon sensor has both an empty name and
                // an empty value: pushing that empty text would still add
                // the row's 6px gap, making the box wider than the icon and
                // shifting it left of centre. Omit empty values (unless a
                // fixed width pins the slot for jitter-free readings).
                // Wi-Fi names the SSID only while hovered: the signal icon
                // stays, the network name is one hover away.
                let show_value = if is_list {
                    s.name.is_empty() && !s.value.is_empty()
                } else {
                    !s.value.is_empty() || s.fixed_width.is_some()
                } && !(is_wifi && !is_hovered);
                if s.name.is_empty() {
                    if show_value {
                        row = row.push(value_widget);
                    }
                } else {
                    let name_text = text(&s.name)
                        .color(text_color)
                        .size(font_size)
                        .font(font_obj)
                        .line_height(line_height);
                    if vertical {
                        let mut col = Column::new().push(name_text);
                        if show_value {
                            col = col.push(value_widget);
                        }
                        row = row.push(col.spacing(2));
                    } else {
                        row = row.push(name_text);
                        if show_value {
                            row = row.push(value_widget);
                        }
                    }
                }
                row.into()
            };

            let desc_str: String = s.description.clone();
            // The dots sensor is purely graphical: a configured description
            // must not put text back into its box.
            let has_desc = !desc_str.is_empty() && workspace_dots.is_none();
            let desc_value = desc_str.clone();

            let content: Element<'a, Message> = if has_desc {
                let desc_size = font_size.saturating_sub(2);
                let desc_widget = text(desc_value)
                    .color(Color::from_rgba(0.5, 0.5, 0.5, 0.7))
                    .size(desc_size)
                    .font(font_obj)
                    .line_height(iced::widget::text::LineHeight::Absolute(
                        bar_line_height(desc_size).into(),
                    ));
                Column::new().push(main).push(desc_widget).into()
            } else {
                main
            };

            // Determine colors: hover when the cursor is over this sensor.
            let bg_color = if is_hovered {
                s.bg_color_hover.or(s.bg_color)
            } else {
                s.bg_color
            };
            let border_color = s.border_color;
            // Horizontal inset only; the box centres the content on the
            // thickness axis by itself (see `styled_box`).
            let pad = iced::Padding {
                left: pad_h as f32,
                right: pad_h as f32,
                top: 0.0,
                bottom: 0.0,
            };

            // The wrapper only tags the box for `FindSensorRect`: same
            // size as the pill it holds, so the layout does not change.
            let styled_container = container(styled_box(
                content,
                bg_color,
                border_color,
                s.border_width,
                s.border_radius,
                &s.border_type,
                // Sensors sit inside sections: never on a screen edge.
                [false, false, false, false],
                pad,
                box_w,
                box_h,
            ))
            .width(box_w)
            .height(box_h)
            .id(sensor_box_id(section_idx, sensor_idx));
            // Tray icons carry their own MouseAreas (press/wheel/tooltip);
            // wrapping them in the sensor MouseArea as well would handle the
            // same events twice, so a tray sensor stays bare — and so does a
            // workspace sensor (its buttons press themselves) and the
            // taskbar (its icon slots press themselves).
            if s.tray.is_some() || s.workspaces.is_some() || s.taskbar.is_some() {
                return Some(styled_container.into());
            }

            // Wrap in MouseArea to deliver hover + click for this specific
            // sensor. The volume meter's bar owns its press/drag, so only
            // its hover and wheel ride the outer area.
            let area = MouseArea::new(styled_container)
                .on_enter(Message::SensorHovered(section_idx, sensor_idx, true))
                .on_exit(Message::SensorHovered(section_idx, sensor_idx, false))
                // Wheel over a volume sensor adjusts it (positive y = wheel
                // up, verified on this stack). Per notch the stack sends a
                // pixel delta, a zero delta (axis_source) and the same delta
                // in lines: only non-zero pixel deltas act — the line form
                // always accompanies the pixel one (wayland protocol), so
                // exactly one adjustment happens per notch.
                .on_scroll(move |delta| {
                    let up = match delta {
                        iced::mouse::ScrollDelta::Pixels { y, .. } if y != 0.0 => Some(y > 0.0),
                        _ => None,
                    };
                    Message::SensorScrolled(section_idx, sensor_idx, up)
                })
                .interaction(iced::mouse::Interaction::Pointer);
            if is_volume_meter || workspace_dots.is_some() || s.media.is_some() {
                return Some(area.into());
            }
            Some(
                area.on_press(Message::SensorPressed(
                    section_idx,
                    sensor_idx,
                    s.action.clone(),
                ))
                .into(),
            )
        }).collect();

        // The section title, in both orientations: above the stacked
        // column, leading the horizontal row (a thin bar has room only
        // sideways). Hidden by an empty name or `panel.section_title`.
        let title_size = font_size.saturating_add(2);
        let title_el: Element<'a, Message> = container(
            text(section.name.clone())
                .color(title_color)
                .size(title_size)
                .font(font_obj)
                .line_height(iced::widget::text::LineHeight::Absolute(
                    bar_line_height(title_size).into(),
                )),
        )
        .align_x(Alignment::Center)
        .padding(content_pad)
        .into();

        let sensors_inner: Element<'a, Message> = if stack_sensors {
            // Spaced like the horizontal row: stacked boxes must not
            // touch and form one continuous strip.
            let sensor_col = Column::new()
                .extend(sensor_widgets)
                .spacing(SENSOR_SPACING)
                .padding(content_pad);
            // `stretch` here means: stacked sensors that must fill the
            // thickness (width) so the boxes stretch — a vertical panel
            // always, a horizontal one only for vertical sections; the
            // title stays content-sized and is centred above them.
            let sensor_col = if stretch { sensor_col.width(Length::Fill) } else { sensor_col };
            let col = if has_title {
                let col = Column::new().push(title_el).push(sensor_col);
                if stretch { col.width(Length::Fill).align_x(Alignment::Center) } else { col }
            } else {
                sensor_col
            };
            col.into()
        } else {
            // The title leads the row; `align_y` centres it against the
            // stretched (Fill-height) sensor boxes.
            let mut row = Row::new()
                .spacing(SENSOR_SPACING)
                .padding(content_pad)
                .align_y(Alignment::Center);
            if has_title {
                row = row.push(title_el);
            }
            row = row.extend(sensor_widgets);
            // `stretch` here means: horizontal section in a horizontal panel ->
            // fill the thickness (height) so the boxes stretch.
            let row = if stretch { row.height(Length::Fill) } else { row };
            row.into()
        };

        // A collapsible section leads with a dot handle in the section's
        // own text colour; collapsed, the box holds only that dot.
        let collapsible = panel
            .config
            .panel
            .sections
            .get(section_idx)
            .is_some_and(|cfg| cfg.collapsible);
        let collapsed = collapsible && panel.collapsed_sections.contains(&section_idx);
        let sensors_inner: Element<'a, Message> = if collapsible {
            let dot: Element<'a, Message> = MouseArea::new(
                container(Space::new())
                    .width(Length::Fixed(8.0))
                    .height(Length::Fixed(8.0))
                    .style(move |_t| ContainerStyle {
                        background: Some(Background::Color(section_text)),
                        border: iced::Border {
                            radius: 4.0.into(),
                            ..Default::default()
                        },
                        ..ContainerStyle::default()
                    }),
            )
            .on_press(Message::SectionToggled(section_idx))
            .interaction(iced::mouse::Interaction::Pointer)
            .into();
            if collapsed {
                container(dot).padding(4).into()
            } else if stack_sensors {
                Column::new()
                    .push(container(dot).padding(2))
                    .push(sensors_inner)
                    .spacing(4)
                    .into()
            } else {
                Row::new()
                    .push(container(dot).padding(2))
                    .push(sensors_inner)
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .into()
            }
        } else {
            sensors_inner
        };

        // Wrap in a styled box: section background plus the section border,
        // drawn with the same rules as the sensor boxes. It fills the panel's
        // cross (thickness) axis and centres the section content on that axis,
        // so `thickness` reaches the sections: stretched boxes fill it, stacked
        // groups are centred within it. Content padding stays in the Row/
        // Column above, so the box itself adds none.
        let section_element: Element<'a, Message> = styled_box(
            sensors_inner,
            Some(section_bg),
            section.border_color,
            section.border_width,
            section.border_radius,
            &section.border_type,
            touch,
            iced::Padding::ZERO,
            if vertical { Length::Fill } else { Length::Shrink },
            if vertical { Length::Shrink } else { Length::Fill },
        );

        section_elements.push(section_element);
    }

    // `align` decides how the sections are spread inside the surface; read
    // it before building the layout so `fill` can pick its own container.
    let align = layout.align.as_deref().unwrap_or("center");
    let fill = align == "fill";

    // Build the full panel layout. The main axis shrinks to the content (so
    // `align` can place the panel within the surface); the cross axis fills
    // the surface thickness, which is what makes `thickness` reach the
    // sections and the sensor boxes. With `align: "fill"` the sections are
    // spread along the whole main axis by `FillSections` instead.
    let main_layout: Element<'a, Message> = if fill {
        FillSections::new(vertical, section_elements).into()
    } else if vertical {
        Column::new()
            .extend(section_elements)
            .width(Length::Fill)
            .into()
    } else {
        Row::new()
            .extend(section_elements)
            .height(Length::Fill)
            .into()
    };

    let panel_bg = panel_background(&panel.config, panel.palette.as_ref());

    // Position the real panel within the surface according to `align`.
    // The surface is screen-sized on the cross axis and sized to the panel's
    // own thickness on the main axis. The `align` must be applied on a
    // container that fills the main axis of the surface; applied on the
    // shrink-wrapped panel it would be a no-op (centering within itself).
    // With `align: "fill"` the panel fills the main axis by itself, so the
    // container alignment below becomes a no-op too.
    let panel_container = container(main_layout)
        .width(if fill || vertical { Length::Fill } else { Length::Shrink })
        .height(if fill || !vertical { Length::Fill } else { Length::Shrink })
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(panel_bg)),
            ..ContainerStyle::default()
        });

    // Outer container fills the surface on BOTH axes, so `align` has room to
    // move the (shrink) panel on its main axis: left / center / right
    // (horizontal) or top / center / bottom (vertical). "fill" lands on the
    // `_` arm on both axes, where there is nothing to move: the panel
    // already spans the surface.
    let surface_container = container(panel_container)
        .width(Length::Fill)
        .height(Length::Fill);

    let surface_container = if vertical {
        surface_container.align_y(match align {
            "top" => Alignment::Start,
            "bottom" => Alignment::End,
            _ => Alignment::Center,
        })
    } else {
        surface_container.align_x(match align {
            "left" => Alignment::Start,
            "right" => Alignment::End,
            _ => Alignment::Center,
        })
    };

    // Discreet setup entry: a right-click on the bar opens the editor.
    // Tray and taskbar areas already handle the right button and capture
    // it, so `MouseArea` stops here only for the zones that own nothing.
    MouseArea::new(surface_container)
        .on_right_press(Message::OpenSetup)
        .into()
}

/// Id of the bar's layer surface, captured on the first render of the
/// bar itself (the popup renders its own tree and never reaches this
/// line, so the first bar `view` call sets it regardless of the order
/// in which the surfaces come up). `LayoutChange` and
/// `ExclusiveZoneChange` carry it to keep targeting the bar once other
/// surfaces are registered — the multi-surface runtime keys actions by id.
pub(crate) static BAR_ID: std::sync::OnceLock<iced::window::Id> = std::sync::OnceLock::new();


