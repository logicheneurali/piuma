//! The calendar popup: month pages for the `calendar` sensor, its
//! size from the font, and its abandon/grace reaping.

use crate::calendar;
use crate::config;
use crate::menus::menu_placement;
use crate::popup_graph::shifted_surface_rect;
use crate::text_width;
use crate::tray;
use crate::{
    corner_radius, panel_font, placement_rect, popup_output_option, popup_style, styled_box,
    CalendarPopup, Message, Panel, DEFAULT_FONT_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH,
};
use iced::widget::container::Style as ContainerStyle;
use iced::widget::{container, text, Column, MouseArea, Row, Space};
use iced::{Alignment, Background, Element, Length, Task as Command};
use iced_exwlshell::reexport::{KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_exwlshell::settings::LayerSize;

/// Grace ticks the calendar popup gets after the cursor has left both
/// its sensor and its surface (1 tick = 1 s at the 1 Hz tick): the
/// spec asks for three seconds before it disappears.
pub(crate) const CALENDAR_ABANDON_GRACE: u8 = 3;

/// Horizontal padding inside one day/label cell, either side.
pub(crate) const CALENDAR_CELL_PAD: u32 = 6;

/// Keep the calendar popup in step with a sensor hover: hovering a
/// `calendar` sensor opens it at the cursor (the trend popup's
/// output-clamped placement), crossing to another calendar sensor
/// swaps it. No hover close runs here — the popup must survive three
/// more seconds after the cursor leaves both it and its sensor: the
/// tick's grace decides (see `reap_abandoned_calendar`).
pub(crate) fn calendar_hover_sync(
    panel: &mut Panel,
    section: usize,
    sensor: usize,
    hovered: bool,
) -> Command<Message> {
    let showing = panel
        .calendar
        .as_ref()
        .is_some_and(|cp| (cp.section, cp.sensor) == (section, sensor));
    if !hovered || showing {
        return Command::none();
    }
    // Opted in? Only the `calendar` source opens this popup (a hover
    // on any other sensor leaves an open popup counting down).
    let is_calendar = panel
        .config
        .panel
        .sections
        .get(section)
        .and_then(|sec| sec.sensors.get(sensor))
        .is_some_and(|s| matches!(s.source, config::SensorSource::Calendar { .. }));
    if !is_calendar {
        return Command::none();
    }
    let Some((x, y)) = tray::cursor_output_point(panel) else {
        // The enter can beat the first CursorMoved event (nothing moved
        // since startup): the motion itself retries via `IcedEvent`.
        eprintln!("[piuma] calendar: no cursor position yet, waiting for the motion event");
        return Command::none();
    };
    let Some(today) = calendar::today() else {
        // Without today's date there is no month to open on.
        eprintln!("[piuma] calendar: no local date, staying closed");
        return Command::none();
    };
    let mut cmds: Vec<Command<Message>> = Vec::new();
    // Crossing between two calendar sensors: the old surface dies with
    // its own leave events too, but closing here keeps the order definite.
    if let Some(old) = panel.calendar.take() {
        cmds.push(iced::window::close(old.id));
    }
    let weekdays = calendar::weekday_labels();
    let (w, h) = calendar_size(&panel.config, &weekdays);
    let (sw, sh) = unsafe { (SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32) };
    let (anchor, margin) = menu_placement(x, y, w, h, sw, sh);
    let rect = placement_rect(anchor, &margin, w, h, sw, sh);
    // Where the surface really lands: the bar's reservation on the
    // shared edge pushes it, so the pointer tests below must not use
    // the intended rect (it contains the cursor while the surface,
    // e.g. for a top bar, sits a zone lower).
    let surface = shifted_surface_rect(rect, anchor, panel.last_anchor, panel.last_exclusive_zone);
    let (id, open) = Message::layershell_open(NewLayerShellSettings {
        size: LayerSize::px(w, h),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: None,
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        // The bar's own output (the graph popup's `LastOutput` landed
        // on the wrong monitor once a second one joined the session).
        output_option: popup_output_option(),
        namespace: Some("piuma-calendar".to_string()),
        ..Default::default()
    });
    panel.calendar = Some(CalendarPopup {
        id,
        section,
        sensor,
        offset: 0,
        today,
        weekdays,
        month_label: calendar::month_label(today.0, today.1),
        // The cursor is on the surface only if it really lies inside
        // it: on the sensor (the common open) that is false and the
        // sensor's own hover keeps the popup alive; a flipped or
        // pushed placement opens away from the cursor either way.
        pointer_on_popup: (x >= surface.0 && x < surface.2 && y >= surface.1 && y < surface.3),
        rect: surface,
        missed: 0,
    });
    eprintln!(
        "[piuma] calendar: open rect={rect:?} surface={surface:?} cursor=({x},{y}) \
         pointer={} size={w}x{h}",
        x >= surface.0 && x < surface.2 && y >= surface.1 && y < surface.3,
    );
    cmds.push(open);
    Command::batch(cmds)
}

/// Page the calendar popup by `delta` months and refresh the header's
/// month label (one `date` spawn per click).
pub(crate) fn calendar_page(panel: &mut Panel, delta: i32) -> Command<Message> {
    let Some(cp) = panel.calendar.as_mut() else {
        return Command::none(); // a click can race the grace's close
    };
    cp.offset += delta;
    let (year, month) = calendar::shift_month(cp.today.0, cp.today.1, cp.offset);
    cp.month_label = calendar::month_label(year, month);
    Command::none()
}

/// Run once per tick: the calendar popup whose grace is spent —
/// neither its own sensor nor its surface has had the cursor for
/// [`CALENDAR_ABANDON_GRACE`] seconds — `None` while it lives. The
/// popup's pointer flag is event-driven (enter/exit), so this counts
/// only real absences, including after the cursor rests for good on
/// another application's window where no event ever arrives again.
pub(crate) fn reap_abandoned_calendar(panel: &mut Panel) -> Option<CalendarPopup> {
    let Some(cp) = panel.calendar.as_mut() else {
        return None;
    };
    if panel.hovered == Some((cp.section, cp.sensor)) || cp.pointer_on_popup {
        cp.missed = 0;
        return None;
    }
    cp.missed = cp.missed.saturating_add(1);
    if cp.missed < CALENDAR_ABANDON_GRACE {
        return None;
    }
    eprintln!("[piuma] calendar: close (cursor left the sensor and the popup)");
    panel.calendar.take()
}

/// The calendar popup's surface size from the panel's font: every cell
/// fits the widest of a day number, the weekday labels and the paging
/// arrows, and the eight rows (header, labels, six weeks) grow with
/// `panel.font_size` — the surface adapts to the font, never the
/// reverse. Returns `(width, height)`.
pub(crate) fn calendar_size(config: &config::Config, weekdays: &[String; 7]) -> (u32, u32) {
    let font = panel_font(config);
    let fs = config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    // Only the padding matters for the size — no colour to resolve.
    let st = popup_style(config, None);
    let mut patterns: Vec<String> = vec!["88".into(), "←".into(), "→".into()];
    patterns.extend(weekdays.iter().cloned());
    let cell_w = u32::from(text_width::widest(&font, fs, &patterns)) + 2 * CALENDAR_CELL_PAD;
    let row_h = fs.saturating_mul(14) / 10; // line height + breathing room
    let w = st
        .padding
        .saturating_mul(2)
        .saturating_add(cell_w.saturating_mul(7));
    let h = st
        .padding
        .saturating_mul(2)
        .saturating_add(row_h.saturating_mul(8));
    (w, h)
}

/// The calendar popup: the month header with its paging arrows, the
/// weekday labels and the month's Monday-first grid, inside the shared
/// popup box (the notification cards' style, like the graph popup).
/// The panel's font and font size draw every cell — the surface was
/// sized from them when the popup opened (`calendar_size`) — and today's
/// number stands out on the accent plate of the notifications' border.
pub(crate) fn calendar_popup_view<'a>(
    panel: &'a Panel,
    cp: &CalendarPopup,
) -> Element<'a, Message> {
    let st = popup_style(&panel.config, panel.palette.as_ref());
    let font = panel_font(&panel.config);
    let fs = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let accent = st.border;
    let (year, month) = calendar::shift_month(cp.today.0, cp.today.1, cp.offset);
    let weeks = calendar::month_matrix(year, month);

    // One grid cell: its content centred in the cell's equal share.
    let cell = |content: Element<'a, Message>| -> Element<'a, Message> {
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    };
    // A paging arrow: a hit target as tall as the header row.
    let arrow = |label: &'static str, msg: Message| -> Element<'a, Message> {
        MouseArea::new(
            container(text(label).size(fs).color(st.text).font(font.clone()))
                .width(Length::Fixed(fs as f32 * 1.6))
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(msg)
        .into()
    };

    let header = Row::new()
        .push(arrow("←", Message::CalendarPrev))
        .push(
            text(cp.month_label.clone())
                .size(fs)
                .color(st.text)
                .font(font.clone())
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .push(arrow("→", Message::CalendarNext))
        .width(Length::Fill)
        .height(Length::Fill);
    let mut labels = Row::new().width(Length::Fill).height(Length::Fill);
    for label in &cp.weekdays {
        labels = labels.push(cell(
            text(label.clone())
                .size(fs)
                .color(st.text)
                .font(font.clone())
                .into(),
        ));
    }
    // Eight equal rows: header, weekday labels and the six week rows.
    let mut rows = Column::new().width(Length::Fill).height(Length::Fill);
    rows = rows.push(header).push(labels);
    for week in weeks {
        let mut row = Row::new().width(Length::Fill).height(Length::Fill);
        for day in week {
            // Today only exists in the month the offset started from.
            let is_today = cp.offset == 0 && day == Some(cp.today.2);
            let day_font = if is_today {
                iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font.clone()
                }
            } else {
                font.clone()
            };
            let body: Element<'_, Message> = match day {
                Some(d) => text(d.to_string())
                    .size(fs)
                    .color(st.text)
                    .font(day_font)
                    .into(),
                None => Space::new().into(),
            };
            let c = container(body)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center);
            let day_cell: Element<'_, Message> = if is_today {
                c.style(move |_theme: &iced::Theme| {
                    let mut style = ContainerStyle::default();
                    style.background = Some(Background::Color(accent));
                    style.border.radius = corner_radius([false; 4], 4.0);
                    style
                })
                .into()
            } else {
                c.into()
            };
            row = row.push(day_cell);
        }
        rows = rows.push(row);
    }
    let content: Element<'_, Message> = rows.into();
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
        iced::Padding::ZERO,
        Length::Fill,
        Length::Fill,
    );
    // The whole surface reports its departure: the tick's grace takes
    // it from there (the cursor may still return within three seconds).
    MouseArea::new(surface)
        .on_exit(Message::CalendarExited)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The calendar popup's surface adapts to the panel font: doubling
    /// `panel.font_size` must grow both dimensions strictly (the cells
    /// fit the numbers and labels they show, the rows follow the size).
    #[test]
    fn calendar_size_follows_the_font() {
        let weekdays = ["lun", "mar", "mer", "gio", "ven", "sab", "dom"].map(String::from);
        let mut cfg = crate::config::default_config();
        cfg.panel.font_size = Some(12);
        let small = calendar_size(&cfg, &weekdays);
        cfg.panel.font_size = Some(24);
        let large = calendar_size(&cfg, &weekdays);
        assert!(
            large.0 > small.0 && large.1 > small.1,
            "{small:?} vs {large:?}"
        );
        // With the default style the surface holds seven columns of at
        // least two characters plus padding — never a degenerate box.
        assert!(small.0 >= 7 * (12 + 2 * 6), "narrow: {:?}", small);
    }
}
