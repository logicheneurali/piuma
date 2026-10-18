//! Bar and popup geometry: the panel's layer size/anchor/thickness,
//! per-corner touching and the shared placement rectangle.

use crate::config;
use iced_exwlshell::reexport::{Anchor, Margin};

/// Panel thickness on the cross axis, in pixels: `panel.thickness` from the
/// config (default 32px, minimum 1px). Single source of truth for the surface
/// size and for the exclusive zone hyprland reserves.
pub(crate) fn panel_thickness(config: &config::Config) -> u32 {
    config
        .panel
        .thickness
        .unwrap_or(config::DEFAULT_HEIGHT)
        .max(1)
}

/// Panel size (as the LayerSurface requests it).
///
/// The surface matches the screen on the axis *perpendicular* to the panel
/// orientation, and matches the panel's own thickness on the axis it
/// extends along:
///
/// * horizontal panel -> `screen_width  x thickness`
/// * vertical panel   -> `thickness x screen_height`
///
/// `thickness` comes from `panel.thickness` in the config (default 32px).
pub(crate) fn panel_size(
    config: &config::Config,
    screen_width: u32,
    screen_height: u32,
) -> iced_exwlshell::settings::LayerSize {
    let layout = config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal");
    let vertical = layout == "vertical";
    let thickness = panel_thickness(config);

    let (width, height) = if vertical {
        (
            iced_exwlshell::settings::Extent::px(thickness),
            iced_exwlshell::settings::Extent::px(screen_height),
        )
    } else {
        (
            iced_exwlshell::settings::Extent::px(screen_width),
            iced_exwlshell::settings::Extent::px(thickness),
        )
    };

    iced_exwlshell::settings::LayerSize { width, height }
}

/// Anchor set for the surface.
///
/// The surface is pinned to the screen edge on the axis it extends along.
/// `align` is handled inside the surface by iced (the inner panel is aligned
/// to left/center/right or top/center/bottom), so the anchor only needs the
/// single edge that sticks the panel to the screen border, which comes from
/// `panel.anchor`:
///
/// * horizontal, top    -> `Top`
/// * horizontal, bottom -> `Bottom`
/// * vertical,   left   -> `Left`
/// * vertical,   right  -> `Right`
pub(crate) fn panel_anchor(config: &config::Config) -> Anchor {
    let vertical = config
        .panel
        .section_layout
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("horizontal")
        == "vertical";

    // Edge from `anchor` (default: top for horizontal, left for vertical);
    // `align` positions the panel INSIDE the surface, it does not pick the
    // edge — reading it here ignored `panel.anchor` completely.
    let anchor = config
        .panel
        .anchor
        .as_deref()
        .unwrap_or(if vertical { "left" } else { "top" });

    if vertical {
        match anchor {
            "bottom" => Anchor::Bottom,
            "right" => Anchor::Right,
            _ => Anchor::Left,
        }
    } else {
        match anchor {
            "bottom" => Anchor::Bottom,
            _ => Anchor::Top,
        }
    }
}

/// Which sides `[top, right, bottom, left]` of the section at `idx` sit
/// flush on a screen edge (roadmap M3): the bar's anchored side touches for
/// every section (the sections span the full thickness and the surface has
/// no root padding); on the length axis the flush ends follow `align` —
/// `fill` keeps the first/last section at the panel ends (both flush when a
/// single section fills the row), `left`/`top` parks only the first one
/// there, `right`/`bottom` only the last, `center` none.
pub(crate) fn section_touch(config: &config::Config, idx: usize, count: usize) -> [bool; 4] {
    let vertical = config.panel.section_layout.as_deref() == Some("vertical");
    let anchor = panel_anchor(config);
    let mut touch = [
        anchor.contains(Anchor::Top),
        anchor.contains(Anchor::Right),
        anchor.contains(Anchor::Bottom),
        anchor.contains(Anchor::Left),
    ];
    let align = config.panel.align.as_deref().unwrap_or("center");
    let at_start = if vertical {
        align == "top"
    } else {
        align == "left"
    };
    let at_end = if vertical {
        align == "bottom"
    } else {
        align == "right"
    };
    let (start, end) = if align == "fill" {
        (idx == 0, idx + 1 == count)
    } else if at_start {
        (idx == 0, false)
    } else if at_end {
        (false, idx + 1 == count)
    } else {
        (false, false)
    };
    if vertical {
        touch[0] |= start;
        touch[2] |= end;
    } else {
        touch[3] |= start;
        touch[1] |= end;
    }
    touch
}

/// Border radius per corner for roadmap M3: `touch = [top, right, bottom,
/// left]`; a corner renders square when **either** of its two sides is flush
/// with a screen edge, rounded to `radius` otherwise.
pub(crate) fn corner_radius(touch: [bool; 4], radius: f32) -> iced::border::Radius {
    let corner = |flush: bool| if flush { 0.0 } else { radius };
    iced::border::Radius {
        top_left: corner(touch[0] || touch[3]),
        top_right: corner(touch[0] || touch[1]),
        bottom_right: corner(touch[2] || touch[1]),
        bottom_left: corner(touch[2] || touch[3]),
    }
}

/// Output-space rect `(x0, y0, x1, y1)` of a surface placed by
/// `menu_placement`: the anchored side starts at its margin, the
/// flipped side's margin pins the far edge to the output.
pub(crate) fn placement_rect(
    anchor: Anchor,
    margin: &Margin,
    w: u32,
    h: u32,
    sw: i32,
    sh: i32,
) -> (i32, i32, i32, i32) {
    let (w, h) = (w as i32, h as i32);
    let (x0, x1) = if anchor.contains(Anchor::Left) {
        (margin.left, margin.left + w)
    } else {
        (sw - margin.right - w, sw - margin.right)
    };
    let (y0, y1) = if anchor.contains(Anchor::Top) {
        (margin.top, margin.top + h)
    } else {
        (sh - margin.bottom - h, sh - margin.bottom)
    };
    (x0, y0, x1, y1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touching sides `[top, right, bottom, left]` (roadmap M3): the bar's
    /// anchored side always touches; on the length axis `fill` flushes the
    /// first/last section (a single one on both ends), `center` none, and a
    /// vertical bar parked at the top gives its first section the top side.
    #[test]
    fn section_touch_follows_anchor_and_align() {
        let mut cfg = crate::config::default_config(); // horizontal, top, center
        assert_eq!(section_touch(&cfg, 0, 3), [true, false, false, false]);
        assert_eq!(section_touch(&cfg, 2, 3), [true, false, false, false]);

        cfg.panel.align = Some("fill".to_string());
        assert_eq!(section_touch(&cfg, 0, 2), [true, false, false, true]);
        assert_eq!(section_touch(&cfg, 1, 2), [true, true, false, false]);
        assert_eq!(section_touch(&cfg, 0, 1), [true, true, false, true]);

        cfg.panel.section_layout = Some("vertical".to_string());
        cfg.panel.align = Some("top".to_string());
        assert_eq!(section_touch(&cfg, 0, 2), [true, false, false, true]);
        assert_eq!(section_touch(&cfg, 1, 2), [false, false, false, true]);
    }
    /// A corner squares when either of its sides touches (roadmap M3): a
    /// top-anchored row's first section keeps only the bottom-right rounding,
    /// and a section touching nothing keeps the pre-M3 look on all corners.
    #[test]
    fn corner_radius_squares_only_corners_by_touching_sides() {
        let r = corner_radius([true, false, false, true], 12.0);
        assert_eq!(r.top_left, 0.0); // top AND left touch
        assert_eq!(r.top_right, 0.0); // top touches (right does not)
        assert_eq!(r.bottom_left, 0.0); // left touches (bottom does not)
        assert_eq!(r.bottom_right, 12.0); // neither side touches
        let none = corner_radius([false; 4], 12.0);
        assert_eq!(
            (
                none.top_left,
                none.top_right,
                none.bottom_right,
                none.bottom_left
            ),
            (12.0, 12.0, 12.0, 12.0)
        );
    }
}
