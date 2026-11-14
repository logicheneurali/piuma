//! Small shared widgets: the styled sensor box, its partial borders,
//! the bar text line box and the compositor blur.

use iced::widget::container::Style as ContainerStyle;
use iced::widget::{container, Column, Row, Space, Stack};
use iced::{Alignment, Background, Color, Element, Length};
use iced_exwlshell::reexport::BlurOption;

use crate::geometry::{corner_radius, leaf_radius};
use crate::{config, Command, Message};

/// The compositor blur for piuma's surfaces: on while the glass tweak
/// is enabled.
pub(crate) fn glass_blur(config: &config::Config) -> BlurOption {
    if config.glass().is_some() {
        BlurOption::FullRegion
    } else {
        BlurOption::None
    }
}


/// Run a system command on its own thread: the menu must close at once
/// and the UI thread must never wait for logind.
pub(crate) fn spawn(program: &'static str, args: &'static [&'static str]) -> Command<Message> {
    std::thread::spawn(move || {
        let _ = std::process::Command::new(program).args(args).status();
    });
    Command::none()
}

// ── helpers ─────────────────────────────────────────────────────────





/// A solid bar widget used to compose partial sensor borders.
pub(crate) fn border_bar(color: Color, width: Length, height: Length) -> Element<'static, Message> {
    container(Space::new().width(width).height(height))
        .style(move |_theme: &iced::Theme| ContainerStyle {
            background: Some(Background::Color(color)),
            ..ContainerStyle::default()
        })
        .into()
}

/// The line box of bar text, rounded to an even whole number of pixels.
/// The renderer floors the text origin and rounds the baseline onto the
/// pixel grid, so a fractional line height (11 px × 1.2 = 13.2) drags the
/// ink a pixel off the box centre; an even box stays on the grid.
pub(crate) fn bar_line_height(font_size: u32) -> f32 {
    (font_size as f32 * 0.6).round().max(1.0) * 2.0
}

/// Build a styled box: background plus its border, drawn according to
/// `border_type`. Used for the sensor boxes, the section containers and the
/// tray sensor's tooltip (which must look like the sensor it belongs to).
///
/// * `full` (or any unknown value) -> native container border, which also
///   honors `border_radius`;
/// * `underline` / `upperline` / `left` / `right` -> a single colored bar on
///   that side, overlaid through a `Stack` so the box keeps exactly the
///   size it would have without a border;
/// * `corners` -> four L-shaped brackets (each arm covers one quarter of
///   its side, so every side keeps a visible middle gap).
///
/// The border is drawn only when both a color and a positive width are set,
/// which is the rule the native border always applied.
#[allow(clippy::too_many_arguments)]
pub(crate) fn styled_box<'a>(
    content: Element<'a, Message>,
    bg_color: Option<Color>,
    border_color: Option<Color>,
    border_width: u16,
    border_radius: u16,
    border_type: &str,
    // Sides `[top, right, bottom, left]` flush with a screen edge: they
    // render square instead of rounded (roadmap M3; `[false; 4]` elsewhere).
    touch: [bool; 4],
    // One inset for all four sides unless the caller breaks the symmetry
    // (the icon rows drop it on the thickness axis to fill the box).
    padding: iced::Padding,
    width: Length,
    height: Length,
) -> Element<'a, Message> {
    // Only these types bypass the native uniform border.
    let partial = matches!(
        border_type,
        "underline" | "upperline" | "left" | "right" | "corners"
    );

    let bw_px = Length::Fixed(border_width as f32);

    // Positions a border bar anywhere inside the box bounds.
    let place = |bar: Element<'a, Message>, x: Alignment, y: Alignment| -> Element<'a, Message> {
        container(bar)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(x)
            .align_y(y)
            .into()
    };

    // The leaf shapes are a native border with only one diagonal rounded;
    // the closure below must not borrow the `border_type` string.
    let leaf = match border_type {
        "leaf-right" => Some(true),
        "leaf-left" => Some(false),
        _ => None,
    };

    // `width`/`height` carry the panel's thickness axis (Fill when the boxes
    // stretch, Shrink otherwise); the content stays content-sized and is
    // centred inside (a no-op on the Shrink axes).
    let base = container(content)
        .padding(padding)
        .width(width)
        .height(height)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme: &iced::Theme| {
            let mut style = ContainerStyle::default();
            if let Some(bg) = bg_color {
                style.background = Some(Background::Color(bg));
            }
            if !partial {
                if let Some(c) = border_color {
                    style.border.color = c;
                    // The native border carves the innermost `border_width`
                    // px of the quad: with an α=0 colour that ring renders as
                    // a hole (premultiplied to nothing) showing the panel
                    // behind — a transparent border draws no border at all.
                    style.border.width = if c.a == 0.0 {
                        0.0
                    } else {
                        border_width as f32
                    };
                    // Roadmap M3: the native border (line *and* background
                    // shape) rounds only the corners away from screen edges;
                    // touching ones render square. Radius is set here — the
                    // renderer composes background and border once, so a
                    // translucent colour shows no seams and nothing is ever
                    // drawn over the sensors.
                    style.border.radius = match leaf {
                        Some(right) => leaf_radius(touch, border_radius as f32, right),
                        None => corner_radius(touch, border_radius as f32),
                    };
                }
            }
            style
        });

    // Full border: the per-corner radius above is all M3 needs — no overlay
    // strips (they used to sit over the content and double-blend the bg).
    if !partial {
        return base.into();
    }
    // Partial borders need a color and a positive width.
    let (Some(color), bw) = (border_color, border_width) else {
        return base.into();
    };
    if bw == 0 {
        return base.into();
    }

    let mut layers: Vec<Element<'a, Message>> = vec![base.into()];
    match border_type {
        "underline" => layers.push(place(
            border_bar(color, Length::Fill, bw_px),
            Alignment::Center,
            Alignment::End,
        )),
        "upperline" => layers.push(place(
            border_bar(color, Length::Fill, bw_px),
            Alignment::Center,
            Alignment::Start,
        )),
        "left" => layers.push(place(
            border_bar(color, bw_px, Length::Fill),
            Alignment::Start,
            Alignment::Center,
        )),
        "right" => layers.push(place(
            border_bar(color, bw_px, Length::Fill),
            Alignment::End,
            Alignment::Center,
        )),
        "corners" => {
            // Each corner is an L of two bars, each arm covering one quarter
            // of its side (FillPortion 1 vs spacer 3), so the middle of every
            // side keeps a gap whatever the sensor size is.
            let arm_h = |bar_first: bool, y: Alignment| -> Element<'a, Message> {
                let bar = border_bar(color, Length::FillPortion(1), bw_px);
                let spacer = Space::new().width(Length::FillPortion(3)).height(bw_px);
                let row = Row::new().width(Length::Fill).spacing(0);
                let row = if bar_first { row.push(bar).push(spacer) } else { row.push(spacer).push(bar) };
                container(row).width(Length::Fill).height(Length::Fill).align_y(y).into()
            };
            let arm_v = |bar_first: bool, x: Alignment| -> Element<'a, Message> {
                let bar = border_bar(color, bw_px, Length::FillPortion(1));
                let spacer = Space::new().width(bw_px).height(Length::FillPortion(3));
                let col = Column::new().height(Length::Fill).spacing(0);
                let col = if bar_first { col.push(bar).push(spacer) } else { col.push(spacer).push(bar) };
                container(col).width(Length::Fill).height(Length::Fill).align_x(x).into()
            };
            layers.push(arm_h(true, Alignment::Start));  // top edge, left quarter
            layers.push(arm_v(true, Alignment::Start));  // left edge, top quarter
            layers.push(arm_h(false, Alignment::Start)); // top edge, right quarter
            layers.push(arm_v(true, Alignment::End));    // right edge, top quarter
            layers.push(arm_h(true, Alignment::End));    // bottom edge, left quarter
            layers.push(arm_v(false, Alignment::Start)); // left edge, bottom quarter
            layers.push(arm_h(false, Alignment::End));   // bottom edge, right quarter
            layers.push(arm_v(false, Alignment::End));   // right edge, bottom quarter
        }
        _ => {}
    }

    Stack::with_children(layers)
        .width(width)
        .height(height)
        .into()
}
