//! The sensor graph popup: the small window a sensor hover opens, its
//! placement/surface geometry and the connector link drawn between
//! the sensor and the window.

use crate::graphs;
use crate::menus::menu_placement;
use crate::sensors;
use crate::tray;
use crate::{
    panel_font, placement_rect, popup_output_option, popup_style, styled_box, FindSensorRect,
    GraphPopup, Message, Panel, SCREEN_HEIGHT, SCREEN_WIDTH,
};
use iced::widget::container::Style as ContainerStyle;
use iced::widget::{container, text, Column, MouseArea, Row, Stack};
use iced::{Alignment, Background, Color, Element, Length, Task as Command};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, Margin, NewLayerShellSettings,
};
use iced_exwlshell::settings::LayerSize;

pub(crate) const GRAPH_POPUP_W: u32 = 260;

pub(crate) const GRAPH_POPUP_H: u32 = 140;

/// Padding around the connector's bounding box: its surface is the
/// box plus this on every side (the end dots must not clip).
pub(crate) const GRAPH_LINK_PAD: i32 = 4;

/// Content geometry inside the popup: a fixed header box and gap, so
/// the average label and the canvas share one exact coordinate system
/// (the label sits on the guide line the draw pass computes).
pub(crate) const GRAPH_HEADER_H: u32 = 20;

pub(crate) const GRAPH_SPACING: u32 = 6;

/// Grace ticks an open graph popup gets once its sensor and its
/// surface both lost the cursor (≈2 s at the 1 Hz tick): the sensor's
/// leave can act on stale coordinates and the widget's exit can be
/// missed, so the reap is the backstop that cannot hang.
pub(crate) const GRAPH_ABANDON_GRACE: u8 = 2;

/// Alpha of the tinted plate behind the average label: opaque enough
/// to hold the text where it crosses the trend line, translucent
/// enough to keep the guide visible through it.
pub(crate) const LABEL_BG_ALPHA: f32 = 0.7;

/// The plate's padding around the average label (v, h).
pub(crate) const LABEL_PAD_V: f32 = 1.0;

pub(crate) const LABEL_PAD_H: f32 = 4.0;

/// One tick of an open popup's abandonment bookkeeping: a pointer on
/// its surface or back on any sensor resets the count (the popup is
/// live), otherwise the grace ticks accumulate — `true` once they run
/// out and the popup must close.
///
/// The leave-guard alone cannot decide this: `last_cursor` can be
/// stale when the sensor's leave processes (widget messages run
/// before the cursor's own update), and once the cursor rests on
/// another application's window no event ever arrives again.
pub(crate) fn popup_missed_tick(gp: &mut GraphPopup, sensor_under_cursor: bool) -> bool {
    if sensor_under_cursor || gp.pointer_on_popup {
        gp.missed = 0;
        return false;
    }
    gp.missed = gp.missed.saturating_add(1);
    gp.missed >= GRAPH_ABANDON_GRACE
}

/// Run once per tick: the abandoned popup to close (its grace spent —
/// see `popup_missed_tick`), `None` while it lives.
pub(crate) fn reap_abandoned_popup(panel: &mut Panel) -> Option<GraphPopup> {
    let sensor_under_cursor = panel.hovered.is_some();
    let expired = match panel.graph_popup.as_mut() {
        Some(gp) => popup_missed_tick(gp, sensor_under_cursor),
        None => return None,
    };
    if !expired {
        return None;
    }
    eprintln!("[piuma] graph: close (cursor left the sensor and the popup)");
    panel.graph_popup.take()
}

/// Keep the hover graph popup in step with a sensor hover: it opens at
/// the cursor (same output-clamped placement as the tray menu) when
/// the sensor opted in with `draw_graph` on a graphable source that
/// already recorded a sample (no data, no popup). On
/// leave it closes only when the cursor sits outside the popup rect —
/// right after opening the surface maps **under the cursor** and steals
/// the pointer from the bar, and that leave must not close it (every
/// other departure either clears the rect test or is caught by the
/// popup's own `GraphPopupExited`). When neither lands — stale
/// coordinates, a missed widget exit — the tick's reap closes the
/// popup after `GRAPH_ABANDON_GRACE` (see `reap_abandoned_popup`).
pub(crate) fn graph_hover_sync(
    panel: &mut Panel,
    section: usize,
    sensor: usize,
    hovered: bool,
) -> Command<Message> {
    let showing = panel
        .graph_popup
        .as_ref()
        .is_some_and(|gp| (gp.section, gp.sensor) == (section, sensor));
    if !hovered {
        if showing && !cursor_in_graph_popup(panel) {
            let gp = panel.graph_popup.take().expect("showing");
            eprintln!("[piuma] graph: close (sensor left, cursor off the popup)");
            return iced::window::close(gp.id);
        }
        if showing {
            eprintln!("[piuma] graph: keep (sensor left, cursor still on the popup)");
        }
        return Command::none();
    }
    if showing {
        return Command::none(); // this sensor is already on screen
    }
    // Opted in and already carrying data? The samples themselves
    // arrive with the ticks; without one there is no trend to show.
    let opted_in = panel
        .config
        .panel
        .sections
        .get(section)
        .and_then(|sec| sec.sensors.get(sensor))
        .is_some_and(|s| s.draw_graph && graphs::scale_of(&s.source).is_some())
        && panel.graphs.has_samples(section, sensor);
    if !opted_in {
        return Command::none();
    }
    eprintln!("[piuma] graph: hover on a graphable sensor ({section},{sensor})");
    let mut cmds: Vec<Command<Message>> = Vec::new();
    // Crossing between two graph sensors: the old surface dies with its
    // own leave events too, but closing here keeps the order definite.
    if let Some(old) = panel.graph_popup.take() {
        cmds.push(iced::window::close(old.id));
    }
    let Some((x, y)) = tray::cursor_output_point(panel) else {
        // The enter can beat the first CursorMoved event (nothing moved
        // since startup): the motion itself retries via `IcedEvent`.
        eprintln!("[piuma] graph: no cursor position yet, waiting for the motion event");
        return Command::batch(cmds);
    };
    let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
    let (anchor, margin) = menu_placement(x, y, GRAPH_POPUP_W, GRAPH_POPUP_H, sw, sh);
    let rect = placement_rect(anchor, &margin, GRAPH_POPUP_W, GRAPH_POPUP_H, sw, sh);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(GRAPH_POPUP_W, GRAPH_POPUP_H),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        // The bar's own output: `LastOutput` landed the popup on a
        // second monitor once one joined the session.
        output_option: popup_output_option(),
        namespace: Some("piuma-graph".to_string()),
        ..Default::default()
    });
    panel.graph_popup = Some(GraphPopup {
        id,
        section,
        sensor,
        rect,
        anchor,
        sensor_rect: None,
        pointer_on_popup: false,
        missed: 0,
    });
    eprintln!("[piuma] graph: open rect={rect:?} cursor=({x},{y})");
    cmds.push(open);
    // Ask the bar for the sensor box's on-screen rect: the connector
    // window is pinned to it once the answer lands (`sync_graph_link`).
    cmds.push(iced_runtime::task::widget(FindSensorRect::new(
        section, sensor,
    )));
    Command::batch(cmds)
}

/// Whether the cursor sits inside the open graph popup's rect.
pub(crate) fn cursor_in_graph_popup(panel: &Panel) -> bool {
    let Some((x, y)) = tray::cursor_output_point(panel) else {
        return false;
    };
    panel
        .graph_popup
        .as_ref()
        .is_some_and(|gp| x >= gp.rect.0 && x < gp.rect.2 && y >= gp.rect.1 && y < gp.rect.3)
}

pub(crate) fn popup_surface_rect(
    gp: &GraphPopup,
    bar_anchor: Anchor,
    zone: i32,
) -> (i32, i32, i32, i32) {
    shifted_surface_rect(gp.rect, gp.anchor, bar_anchor, zone)
}

/// The placement rect shifted to where the surface really lands (the
/// graph popup's `popup_surface_rect` shares this math): a layer
/// surface is pushed past the reservations of the edge it anchors to —
/// and the bar's reservation lives on the edge the BAR anchors to. So
/// the shift applies per axis, only where the two anchors point to the
/// same side (measured on this stack: top bar, popup anchored Left|Top
/// → +28 on y, none on x).
pub(crate) fn shifted_surface_rect(
    rect: (i32, i32, i32, i32),
    anchor: Anchor,
    bar_anchor: Anchor,
    zone: i32,
) -> (i32, i32, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = rect;
    if anchor.contains(Anchor::Top) && bar_anchor.contains(Anchor::Top) {
        y0 += zone;
        y1 += zone;
    }
    if anchor.contains(Anchor::Bottom) && bar_anchor.contains(Anchor::Bottom) {
        y0 -= zone;
        y1 -= zone;
    }
    if anchor.contains(Anchor::Left) && bar_anchor.contains(Anchor::Left) {
        x0 += zone;
        x1 += zone;
    }
    if anchor.contains(Anchor::Right) && bar_anchor.contains(Anchor::Right) {
        x0 -= zone;
        x1 -= zone;
    }
    (x0, y0, x1, y1)
}

/// The connector drawing in the link surface's own coordinates, the
/// surface's top-left output position and its size — `None` while the
/// popup or its sensor rect is missing (the window renders empty in
/// that gap and the tick closes it).
pub(crate) fn graph_link_geometry(
    panel: &Panel,
) -> Option<(graphs::Connector, (i32, i32), (u32, u32))> {
    let gp = panel.graph_popup.as_ref()?;
    let sensor = gp.sensor_rect?;
    let popup = popup_surface_rect(gp, panel.last_anchor, panel.last_exclusive_zone);
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let (d1, d2, path) = graphs::connector_geometry(sensor, popup);
    // The surface is the connector's bounding box (the end dots are
    // its extremes — the rail lies between them) plus a dot-sized
    // padding on every side; the drawing shifts into local space.
    let x0 = d1.x.min(d2.x).floor() as i32 - GRAPH_LINK_PAD;
    let y0 = d1.y.min(d2.y).floor() as i32 - GRAPH_LINK_PAD;
    let x1 = d1.x.max(d2.x).ceil() as i32 + GRAPH_LINK_PAD;
    let y1 = d1.y.max(d2.y).ceil() as i32 + GRAPH_LINK_PAD;
    let shift = |p: iced::Point| iced::Point::new(p.x - x0 as f32, p.y - y0 as f32);
    let conn = graphs::Connector {
        d1: shift(d1),
        d2: shift(d2),
        path: path.iter().copied().map(shift).collect(),
        color: st.border,
        width: (st.border_width as f32).max(1.0),
    };
    Some((conn, (x0, y0), ((x1 - x0) as u32, (y1 - y0) as u32)))
}

/// One place drives the connector surface: it opens once the popup's
/// sensor rect has landed (the query answers a render after the popup
/// opened), it swaps when the popup jumps to another sensor, and it
/// dies with the popup. With `exclusive_zone = -1` the surface ignores
/// every reservation, so its margins are the exact output position of
/// the connector's bounding box.
pub(crate) fn sync_graph_link(panel: &mut Panel) -> Option<Command<Message>> {
    let wanted = panel
        .graph_popup
        .as_ref()
        .filter(|gp| gp.sensor_rect.is_some())
        .map(|gp| gp.id);
    // Settled: the open connector draws the popup that is showing.
    if let (Some((_, popup_id)), Some(wanted)) = (panel.graph_link, wanted) {
        if popup_id == wanted {
            return None;
        }
    }
    let mut cmds: Vec<Command<Message>> = Vec::new();
    // The popup died or moved to another sensor: the old connector
    // goes (a swap reopens it on the new geometry right after).
    if let Some((id, _)) = panel.graph_link.take() {
        eprintln!("[piuma] graph: link close");
        cmds.push(iced::window::close(id));
    }
    if let Some(popup_id) = wanted {
        let (_, (ox, oy), (w, h)) = graph_link_geometry(panel)?;
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            size: LayerSize::px(w, h),
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Left,
            // -1 = ignore other surfaces' reservations (the bar's
            // strip must not inset this one — as on the wallpaper).
            exclusive_zone: Some(-1),
            margin: Some(Margin {
                top: oy,
                right: 0,
                bottom: 0,
                left: ox,
            }),
            keyboard_interactivity: KeyboardInteractivity::None,
            // Same output as the popup it points at (see above).
            output_option: popup_output_option(),
            // A pure drawing over the desktop: hovers and clicks
            // pass through (as on the wallpaper).
            events_transparent: true,
            namespace: Some("piuma-graph-link".to_string()),
            ..Default::default()
        });
        panel.graph_link = Some((id, popup_id));
        eprintln!("[piuma] graph: link at ({ox},{oy}) size={w}x{h}");
        cmds.push(open);
    }
    Some(Command::batch(cmds))
}

/// The connector window's view: one canvas with the polyline and the
/// two end dots. The surface lives exactly as long as the popup; in
/// the tick-sized gap before its close the drawing is empty.
pub(crate) fn graph_link_view<'a>(panel: &'a Panel) -> Element<'a, Message> {
    match graph_link_geometry(panel) {
        Some((conn, _, _)) => graphs::link_widget(conn),
        None => graphs::link_widget(graphs::Connector {
            d1: iced::Point::new(0.0, 0.0),
            d2: iced::Point::new(0.0, 0.0),
            path: Vec::new(),
            color: Color::TRANSPARENT,
            width: 0.0,
        }),
    }
}

/// The hover graph popup: the sensor's name and current value over its
/// trend line, inside the shared popup box (the notification cards'
/// style, same as the tray menu). The whole surface is one MouseArea
/// whose exit reports the departure to `GraphPopupExited`.
pub(crate) fn graph_popup_view<'a>(panel: &'a Panel, gp: &GraphPopup) -> Element<'a, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let sensor = panel
        .sections
        .get(gp.section)
        .and_then(|sec| sec.sensors.get(gp.sensor));
    let (name, current, color) = match sensor {
        // The threshold rule tints the popup like the bar (header,
        // trend, labels and average all follow the reading).
        Some(s) => {
            let base = s.text_color.unwrap_or(st.text);
            let ruled = sensors::leading_number(&s.value)
                .and_then(|v| {
                    s.color_rules
                        .iter()
                        .rev()
                        .find(|(min, _)| v >= *min)
                        .map(|(_, c)| *c)
                })
                .unwrap_or(base);
            (s.name.clone(), s.value.clone(), ruled)
        }
        // The sensor died under the popup (its exit will follow): keep
        // the box empty rather than missing layers.
        None => (String::new(), "N/A".to_string(), st.text),
    };
    let (scale, samples) = panel
        .graphs
        .window(gp.section, gp.sensor)
        .unwrap_or((graphs::Scale::Auto, Vec::new()));
    // Fixed content geometry: header box + gap, canvas takes the rest
    // of the fixed-size window — the label below is positioned against
    // exactly these numbers.
    let graph_h = GRAPH_POPUP_H
        .saturating_sub(2 * st.padding)
        .saturating_sub(GRAPH_HEADER_H)
        .saturating_sub(GRAPH_SPACING) as f32;
    let header: Element<'_, Message> = container(
        Row::new()
            .push(
                text(name)
                    .size(st.font_size)
                    .color(st.text)
                    .font(font.clone())
                    .wrapping(iced::widget::text::Wrapping::None)
                    .width(Length::Fill),
            )
            // The current reading leads the header: bigger than the
            // sensor's name, in the same colour as the trend below.
            .push(
                text(current)
                    .size(st.font_size.saturating_add(6))
                    .color(color)
                    .font(font),
            )
            .align_y(Alignment::Center)
            .spacing(8)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .height(Length::Fixed(GRAPH_HEADER_H as f32))
    .into();
    // The average guide and its label share the canvas's coordinates:
    // the same `axis_y` / `y_bounds` math the draw pass uses.
    let guide = graphs::mean(&samples).map(|avg| {
        let (lo, hi) = graphs::y_bounds(scale, &samples);
        (avg, graphs::axis_y(graph_h, (avg - lo) / (hi - lo)))
    });
    // The sensor's unit feeds both the canvas scale labels and the
    // average label.
    let unit = panel
        .config
        .panel
        .sections
        .get(gp.section)
        .and_then(|sec| sec.sensors.get(gp.sensor))
        .map(|s| graphs::unit_of(&s.source))
        .unwrap_or("");
    let graph_box: Element<'_, Message> = container(graphs::widget(samples, scale, color, unit))
        .width(Length::Fill)
        .height(Length::Fixed(graph_h))
        .into();
    let graph_area: Element<'_, Message> = match guide {
        Some((avg, y)) => {
            // The label rides centred on the guide, on a tinted plate
            // that keeps it readable where it crosses the trend line
            // (line-height estimate + the plate's own padding).
            let label_h = st.font_size as f32 * 1.2 + 2.0 * LABEL_PAD_V;
            let plate = container(
                text(graphs::avg_label(avg, unit))
                    .size(st.font_size.saturating_sub(1).max(8))
                    .color(color),
            )
            .style(move |_theme: &iced::Theme| ContainerStyle {
                background: Some(Background::Color(Color {
                    a: LABEL_BG_ALPHA,
                    ..st.bg
                })),
                ..ContainerStyle::default()
            })
            .padding(iced::Padding {
                top: LABEL_PAD_V,
                bottom: LABEL_PAD_V,
                left: LABEL_PAD_H,
                right: LABEL_PAD_H,
            });
            let label: Element<'_, Message> = container(plate)
                .width(Length::Fill)
                .height(Length::Fixed(graph_h))
                .align_x(Alignment::Center)
                .padding(iced::Padding {
                    top: (y - label_h / 2.0).clamp(0.0, (graph_h - label_h).max(0.0)),
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .into();
            Stack::with_children([graph_box, label])
                .width(Length::Fill)
                .height(Length::Fixed(graph_h))
                .into()
        }
        None => graph_box,
    };
    let content: Element<'_, Message> = Column::new()
        .push(header)
        .push(graph_area)
        .spacing(GRAPH_SPACING as f32)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    let surface = styled_box(
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::from(st.padding as f32))
            .into(),
        Some(st.bg),
        Some(st.border),
        st.border_width.min(u16::MAX as u32) as u16,
        st.border_radius.min(u16::MAX as u32) as u16,
        &st.border_type,
        [false, false, false, false],
        // The content carries the padding; the box only draws its border.
        0,
        Length::Fill,
        Length::Fill,
    );
    MouseArea::new(surface)
        .on_exit(Message::GraphPopupExited)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The surface really lands shifted past the bar's reservation on
    /// the shared edge only — the calendar popup's pointer tests and
    /// the graph link both rely on this math: a top bar pushes a
    /// top-anchored popup down by the zone (the intended rect then
    /// contains the cursor while the surface does not); mismatched
    /// edges leave the placement untouched.
    #[test]
    fn surface_rect_shifts_only_the_shared_edge() {
        let rect = (100, 15, 397, 167);
        assert_eq!(
            shifted_surface_rect(rect, Anchor::Left | Anchor::Top, Anchor::Top, 30),
            (100, 45, 397, 197),
            "top bar + top popup: +30 on y, none on x"
        );
        assert_eq!(
            shifted_surface_rect(rect, Anchor::Left | Anchor::Top, Anchor::Bottom, 30),
            rect,
            "different edges: no shift"
        );
        assert_eq!(
            shifted_surface_rect(rect, Anchor::Right | Anchor::Bottom, Anchor::Bottom, 28),
            (100, -13, 397, 139),
            "bottom bar + bottom popup: -28 on y"
        );
    }
    /// The tick reaps a popup whose sensor and surface both lost the
    /// cursor — but only after the grace, and never while a sensor is
    /// under the cursor or the pointer sits on the surface. The flag
    /// is two-way: a pointer that visits and then leaves falls back to
    /// the grace instead of pinning the popup open forever.
    #[test]
    fn an_abandoned_popup_is_reaped_after_its_grace() {
        let fresh = || GraphPopup {
            id: iced::window::Id::unique(),
            section: 0,
            sensor: 0,
            rect: (0, 0, 10, 10),
            anchor: Anchor::Top,
            sensor_rect: None,
            pointer_on_popup: false,
            missed: 0,
        };
        let mut abandoned = fresh();
        assert!(
            !popup_missed_tick(&mut abandoned, false),
            "the first tick only grants grace"
        );
        assert!(
            popup_missed_tick(&mut abandoned, false),
            "the second tick reaps it"
        );

        let mut on_surface = fresh();
        on_surface.pointer_on_popup = true;
        assert!(
            !popup_missed_tick(&mut on_surface, false),
            "pointer on it: never abandoned"
        );

        let mut visited = fresh();
        visited.pointer_on_popup = true;
        assert!(
            !popup_missed_tick(&mut visited, false),
            "pointer on it: never abandoned"
        );
        visited.pointer_on_popup = false;
        assert!(
            !popup_missed_tick(&mut visited, false),
            "the pointer left: the first tick only grants grace"
        );
        assert!(
            popup_missed_tick(&mut visited, false),
            "the pointer left: the second tick reaps it"
        );

        let mut on_sensor = fresh();
        on_sensor.missed = 1;
        assert!(!popup_missed_tick(&mut on_sensor, true), "still hovered");
        assert_eq!(on_sensor.missed, 0, "a hover resets the count");
    }
}
