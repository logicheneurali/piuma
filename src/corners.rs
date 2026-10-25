//! Rounded screen corners (roadmap angoli-schermo): one square overlay
//! surface per output corner, painted in the configured colour except
//! the inner quarter disk — the eye reads the display corner as
//! rounded, matching the bar's own rounded sections.
//!
//! Surfaces, not a compositor feature: layer-shell is the only API the
//! bar has, and the Overlay layer makes the mask cover windows and the
//! bar itself. Keyed "output:corner:radius" so a radius change (surface
//! size is fixed at open) reopens the masks through the same plan the
//! wallpaper uses.

use std::f32::consts::{FRAC_PI_2, PI};

use crate::wallpaper_ui::{forget_closed, wallpaper_plan};
use crate::{config, monitor_names, Message, Panel};
use iced::mouse::Cursor;
use iced::widget::canvas::{self, Frame, Geometry, Path, path::Arc};
use iced::widget::container;
use iced::widget::text;
use iced::{Color, Element, Length, Point, Radians, Rectangle, Task as Command};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;

/// The four screen corners: which edge pair the surface is anchored to
/// and how the mask path is oriented (the opaque sliver's tip sits on
/// the screen's own corner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    const ALL: [Corner; 4] =
        [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

    /// Anchored to the two edges meeting at this corner — the surface
    /// snaps to the screen's true corner whatever else is reserved.
    fn anchor(self) -> Anchor {
        match self {
            Corner::TopLeft => Anchor::Top | Anchor::Left,
            Corner::TopRight => Anchor::Top | Anchor::Right,
            Corner::BottomLeft => Anchor::Bottom | Anchor::Left,
            Corner::BottomRight => Anchor::Bottom | Anchor::Right,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Corner::TopLeft => "tl",
            Corner::TopRight => "tr",
            Corner::BottomLeft => "bl",
            Corner::BottomRight => "br",
        }
    }

    fn from_tag(tag: &str) -> Option<Corner> {
        match tag {
            "tl" => Some(Corner::TopLeft),
            "tr" => Some(Corner::TopRight),
            "bl" => Some(Corner::BottomLeft),
            "br" => Some(Corner::BottomRight),
            _ => None,
        }
    }
}

/// Plan key of one mask: output name, corner tag and the radius the
/// surface was sized with. A radius change yields different keys, so
/// the plan closes the old masks and opens correctly sized ones.
fn key(output: &str, corner: Corner, radius: u32) -> String {
    format!("{output}:{}:{radius}", corner.tag())
}

fn parse_key(key: &str) -> Option<(String, Corner, u32)> {
    let (rest, radius) = key.rsplit_once(':')?;
    let (output, tag) = rest.split_once(':')?;
    Some((output.to_string(), Corner::from_tag(tag)?, radius.parse().ok()?))
}

/// Open or close the corner masks so they match the config — the same
/// per-tick plan as the wallpaper, one surface per output × corner. The
/// radius reaching the surface is clamped here too: a hand-edited
/// config cannot open a degenerate or huge surface even if validation
/// was skipped.
pub(crate) fn sync_corners(panel: &mut Panel) -> Command<Message> {
    let wanted = panel
        .config
        .panel
        .screen_corners
        .as_ref()
        .filter(|c| c.enable)
        .map(|c| c.radius.clamp(1, 256));
    let desired: Vec<String> = match wanted {
        Some(radius) => monitor_names()
            .iter()
            .flat_map(|output| {
                Corner::ALL
                    .iter()
                    .map(move |corner| key(output, *corner, radius))
                    .collect::<Vec<_>>()
            })
            .collect(),
        None => Vec::new(),
    };
    let (to_open, to_close) = wallpaper_plan(&panel.corners, &desired);
    // Forget the closed masks in the same breath: a replugged output
    // (same name) must open fresh surfaces — the id kept here would
    // otherwise match the plan's "already placed" and never reopen.
    forget_closed(&mut panel.corners, &to_close);
    let mut cmds: Vec<Command<Message>> = to_close.into_iter().map(iced::window::close).collect();
    for key in to_open {
        let Some((output, corner, radius)) = parse_key(&key) else {
            continue;
        };
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            // The surface is the mask square: side = rounding radius.
            size: LayerSize::px(radius, radius),
            layer: Layer::Overlay,
            anchor: corner.anchor(),
            // -1 = ignore other surfaces' reservations: the bar's strip
            // must not push a corner away from the screen's true edge.
            exclusive_zone: Some(-1),
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::OutputName(output),
            // Clicks fall through: an invisible square over the corner
            // must never eat input meant for what's underneath.
            events_transparent: true,
            namespace: Some("piuma-corners".to_string()),
            ..Default::default()
        });
        panel.corners.push((key, id));
        cmds.push(open);
    }
    Command::batch(cmds)
}

/// One corner mask surface: the square canvas the compositor sized to
/// the radius. The shape comes from the actual bounds (not the stored
/// radius), so a fractional scale that resizes the surface cannot clip
/// or overshoot the mask.
pub(crate) fn corners_view(panel: &Panel, window: iced::window::Id) -> Element<'_, Message> {
    let color = panel
        .config
        .panel
        .screen_corners
        .as_ref()
        .and_then(|c| config::parse_color(&c.color).ok())
        .unwrap_or(Color::BLACK);
    let corner = panel
        .corners
        .iter()
        .find(|(_, id)| *id == window)
        .and_then(|(key, _)| parse_key(key));
    match corner {
        Some((_, corner, _)) => canvas::Canvas::new(CornerMask { corner, color })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        // The id is routed here only while the mask exists; an empty
        // transparent surface beats panicking on a vanished entry.
        None => container(text("")).width(Length::Fill).height(Length::Fill).into(),
    }
}

/// The drawn mask: the square minus the quarter disk centred on the
/// corner farthest from the screen's own — one arc, no blend modes, the
/// uncovered surface stays real transparency.
struct CornerMask {
    corner: Corner,
    color: Color,
}

impl<Message> canvas::Program<Message> for CornerMask {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height);
        if side <= 0.0 {
            return vec![frame.into_geometry()];
        }
        // Arc parameters per corner: centre of the transparent quarter
        // disk (the square corner farthest from the screen corner), and
        // the arc between its two tangent points — angles are clockwise
        // from +x (screen coords), so a positive sweep from start to end
        // bulges toward the opaque tip the path closes on.
        let (center, start, end, tip) = match self.corner {
            Corner::TopLeft => ((side, side), PI, PI + FRAC_PI_2, (0.0, 0.0)),
            Corner::TopRight => ((0.0, side), PI + FRAC_PI_2, 2.0 * PI, (side, 0.0)),
            Corner::BottomLeft => ((side, 0.0), FRAC_PI_2, PI, (0.0, side)),
            Corner::BottomRight => ((0.0, 0.0), 0.0, FRAC_PI_2, (side, side)),
        };
        let mask = Path::new(|builder| {
            // `arc` starts its own sub-path at the arc's first point, so
            // it opens the shape and the two lines below close it around
            // the opaque tip.
            builder.arc(Arc {
                center: Point::new(center.0, center.1),
                radius: side,
                start_angle: Radians(start),
                end_angle: Radians(end),
            });
            builder.line_to(Point::new(tip.0, tip.1));
            builder.close();
        });
        frame.fill(&mask, self.color);
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan key round-trips: every corner of every output parses
    /// back to the values the surface is opened with.
    #[test]
    fn corner_keys_roundtrip() {
        for corner in Corner::ALL {
            let k = key("HDMI-A-2", corner, 32);
            let (output, c, radius) = parse_key(&k).expect("key must parse");
            assert_eq!(output, "HDMI-A-2");
            assert_eq!(c, corner);
            assert_eq!(radius, 32);
        }
        assert!(parse_key("HDMI-A-2:nope:32").is_none());
        assert!(parse_key("HDMI-A-2:tl:x").is_none());
    }

    /// Each corner anchors to the two edges meeting there — what makes
    /// the surface snap to the screen's true corner.
    #[test]
    fn corner_anchors_match_the_edges() {
        assert_eq!(Corner::TopLeft.anchor(), Anchor::Top | Anchor::Left);
        assert_eq!(Corner::TopRight.anchor(), Anchor::Top | Anchor::Right);
        assert_eq!(Corner::BottomLeft.anchor(), Anchor::Bottom | Anchor::Left);
        assert_eq!(Corner::BottomRight.anchor(), Anchor::Bottom | Anchor::Right);
    }
}
