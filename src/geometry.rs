//! Bar and popup geometry: the panel's layer size/anchor/thickness,
//! per-corner touching and the shared placement rectangle.

use crate::config;
use crate::icons;
use iced_exwlshell::reexport::{Anchor, Margin};

/// What a section's title and inset take from the icon's cross axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IconRoom {
    /// The stacked title's room, only on a horizontal panel.
    pub title_extra: u32,
    /// Section content padding on the cross axis: tighter on a vertical
    /// side bar, where the panel width is only a little wider than the
    /// icon.
    pub pad: u16,
}

/// Per-section icon room. A title shares the cross axis only on a
/// HORIZONTAL panel whose sensors are stacked vertically; a vertical
/// panel's title runs along the bar's length, so it never narrows the
/// icons. A section shown on a vertical panel alone gets the narrow
/// inset; one also shown horizontally keeps the normal inset (its icon
/// must fit the tighter panel). Hidden titles consume nothing.
pub(crate) fn section_icon_room(config: &config::Config, font_size: u32) -> Vec<IconRoom> {
    let normal = IconRoom {
        title_extra: 0,
        pad: icons::SECTION_CONTENT_PAD,
    };
    let mut rooms = vec![normal; config.panel.sections.len()];
    let titles = config.panel.section_title.unwrap_or(true);
    let mut on_horizontal = vec![false; config.panel.sections.len()];
    for layout in config::panel_layouts(config) {
        let vertical = panel_vertical(config, layout.anchor.as_deref());
        for (index, section) in config.panel.sections.iter().enumerate() {
            let visible = match &layout.sections {
                Some(names) => names.iter().any(|name| name == &section.name),
                None => true,
            };
            if !visible {
                continue;
            }
            if vertical {
                if !on_horizontal[index] {
                    rooms[index].pad = icons::SECTION_CONTENT_PAD_NARROW;
                }
            } else {
                on_horizontal[index] = true;
                rooms[index].pad = icons::SECTION_CONTENT_PAD;
                if titles {
                    rooms[index].title_extra = icons::stacked_title_extra(
                        section.layout.as_deref().unwrap_or("horizontal"),
                        &section.name,
                        font_size,
                    );
                }
            }
        }
    }
    rooms
}

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
    anchor: Option<&str>,
) -> iced_exwlshell::settings::LayerSize {
    let vertical = panel_vertical(config, anchor);
    let thickness = panel_thickness(config);

    // Mirror mode: one surface per output, so the main axis fills
    // whatever monitor the compositor binds it to instead of one
    // monitor's size.
    let all = config.panel.output.as_deref() == Some("all");
    let fill = iced_exwlshell::settings::Extent::Fill;
    let (width, height) = if vertical {
        (
            iced_exwlshell::settings::Extent::px(thickness),
            // Anchored top+bottom (see `panel_anchor_for`): the
            // compositor stretches the bar between the edges, so the
            // other bars' reserved strips are respected.
            fill,
        )
    } else {
        (
            if all { fill } else { iced_exwlshell::settings::Extent::px(screen_width) },
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
///
/// Whether the panel runs vertically: `left`/`right` edges make it a
/// side bar (thickness = width, full height), `top`/`bottom` a
/// horizontal one. Only an absent (or legacy corner) anchor falls back
/// to the global `panel.section_layout`, the single-bar setting.
pub(crate) fn panel_vertical(config: &config::Config, anchor: Option<&str>) -> bool {
    match anchor {
        Some("left" | "right") => true,
        Some("top" | "bottom") => false,
        _ => config.panel.section_layout.as_deref() == Some("vertical"),
    }
}

pub(crate) fn panel_anchor(config: &config::Config) -> Anchor {
    panel_anchor_for(config, config.panel.anchor.as_deref())
}

/// Anchor from an explicit per-layout value (`panel.anchor` when `None`).
/// The anchor also decides the orientation (see [`panel_vertical`]): a
/// `left`/`right` edge is a side bar whatever the global layout says.
pub(crate) fn panel_anchor_for(config: &config::Config, anchor: Option<&str>) -> Anchor {
    let vertical = panel_vertical(config, anchor);

    // Edge from `anchor` (default: top for horizontal, left for vertical);
    // `align` positions the panel INSIDE the surface, it does not pick the
    // edge — reading it here ignored `panel.anchor` completely.
    let anchor = anchor.unwrap_or(if vertical { "left" } else { "top" });

    if vertical {
        // Both length-axis edges are anchored so `Extent::Fill` can
        // stretch the height between them (below/above any horizontal
        // bar's reserved strip).
        match anchor {
            "right" => Anchor::Right | Anchor::Top | Anchor::Bottom,
            _ => Anchor::Left | Anchor::Top | Anchor::Bottom,
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
#[cfg(test)]
pub(crate) fn section_touch(config: &config::Config, idx: usize, count: usize) -> [bool; 4] {
    section_touch_for(
        panel_anchor(config),
        config.panel.align.as_deref().unwrap_or("center"),
        idx,
        count,
    )
}

/// Touch sides of section `idx` of `count` on one panel: the section
/// borders that sit against the screen edge for that panel's anchor and
/// align. The shared styles resolve the root panel's values; the view
/// recomputes with the layout it is drawing.
pub(crate) fn section_touch_for(
    anchor: Anchor,
    align: &str,
    idx: usize,
    count: usize,
) -> [bool; 4] {
    let vertical = anchor.contains(Anchor::Left) || anchor.contains(Anchor::Right);
    // A vertical surface anchors Top|Bottom too, but only to stretch the
    // layer: those are the length axis, not the thickness side flush with
    // the screen. Reading them here squared the top/bottom corners of
    // every section in a side bar, even the ones parked in the middle.
    let mut touch = if vertical {
        [false, anchor.contains(Anchor::Right), false, anchor.contains(Anchor::Left)]
    } else {
        [anchor.contains(Anchor::Top), false, anchor.contains(Anchor::Bottom), false]
    };
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

/// Per-corner radius of the leaf shapes: `right` rounds the top-left and
/// bottom-right corners, `left` rounds the top-right and bottom-left
/// ones; every other corner is square. A corner flush with a screen edge
/// stays square too, like every other border type.
pub(crate) fn leaf_radius(touch: [bool; 4], radius: f32, right: bool) -> iced::border::Radius {
    let corner = |enabled: bool, flush: bool| {
        if enabled && !flush {
            radius
        } else {
            0.0
        }
    };
    iced::border::Radius {
        top_left: corner(right, touch[0] || touch[3]),
        top_right: corner(!right, touch[0] || touch[1]),
        bottom_right: corner(right, touch[2] || touch[1]),
        bottom_left: corner(!right, touch[2] || touch[3]),
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
        cfg.panel.anchor = None;
        cfg.panel.align = Some("top".to_string());
        // A side bar's surface is stretched between top and bottom, but
        // those edges are the length axis: only the first section parked
        // at the top is flush, the rest is rounded there.
        assert_eq!(section_touch(&cfg, 0, 2), [true, false, false, true]);
        assert_eq!(section_touch(&cfg, 1, 2), [false, false, false, true]);

        // Centred on a left side bar: only the screen-edge side touches,
        // the right corners stay rounded.
        cfg.panel.anchor = Some("left".to_string());
        cfg.panel.align = Some("center".to_string());
        assert_eq!(section_touch(&cfg, 0, 1), [false, false, false, true]);
    }
    /// A section on a secondary panel squares the sides of THAT panel:
    /// bottom-anchored, right-aligned, the last section touches bottom+right.
    #[test]
    fn section_touch_for_follows_the_panel_anchor() {
        assert_eq!(
            section_touch_for(Anchor::Bottom, "right", 1, 2),
            [false, true, true, false]
        );
        assert_eq!(
            section_touch_for(Anchor::Top, "left", 0, 2),
            [true, false, false, true]
        );
        // A side bar: the left edge touches; `top` flushes the first
        // section's top side (the length axis is vertical).
        assert_eq!(
            section_touch_for(Anchor::Left, "top", 0, 2),
            [true, false, false, true]
        );
        assert_eq!(
            section_touch_for(Anchor::Right, "fill", 1, 2),
            [false, true, true, false]
        );
    }

    /// The anchor decides the orientation: `left`/`right` make a side
    /// bar even when the global layout is horizontal (and vice versa);
    /// only an absent anchor falls back to `panel.section_layout`.
    #[test]
    fn anchor_decides_the_panel_orientation() {
        let mut cfg = crate::config::default_config(); // horizontal, top
        assert!(!panel_vertical(&cfg, None), "legacy default: horizontal");
        assert!(panel_vertical(&cfg, Some("left")));
        assert!(panel_vertical(&cfg, Some("right")));
        assert!(!panel_vertical(&cfg, Some("top")));
        assert!(!panel_vertical(&cfg, Some("bottom")));

        let left = panel_anchor_for(&cfg, Some("left"));
        assert!(left.contains(Anchor::Left), "side edge: {left:?}");
        assert!(
            left.contains(Anchor::Top) && left.contains(Anchor::Bottom),
            "both length edges, so Fill stretches the height: {left:?}"
        );
        let right = panel_anchor_for(&cfg, Some("right"));
        assert!(right.contains(Anchor::Right), "side edge: {right:?}");
        assert!(right.contains(Anchor::Top) && right.contains(Anchor::Bottom));

        // A left/right anchor sizes the surface as a side bar.
        let thickness = panel_thickness(&cfg);
        let size = panel_size(&cfg, 1920, Some("left"));
        assert_eq!(size.width, iced_exwlshell::settings::Extent::px(thickness));
        assert_eq!(
            size.height,
            iced_exwlshell::settings::Extent::Fill,
            "the anchored edges stretch the height, thickness wide"
        );
        let size = panel_size(&cfg, 1920, Some("top"));
        assert_eq!(size.width, iced_exwlshell::settings::Extent::px(1920));
        assert_eq!(size.height, iced_exwlshell::settings::Extent::px(thickness));

        // The legacy vertical config keeps working with no anchor.
        cfg.panel.section_layout = Some("vertical".to_string());
        assert!(panel_vertical(&cfg, None));
        assert!(panel_anchor_for(&cfg, None).contains(Anchor::Left));
        assert!(
            !panel_vertical(&cfg, Some("top")),
            "an explicit top anchor still wins"
        );
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

    /// The leaf shapes round only their diagonal; a flush screen edge
    /// still squares the corners it touches.
    #[test]
    fn leaf_radius_rounds_only_its_diagonal() {
        let right = leaf_radius([false; 4], 10.0, true);
        assert_eq!(
            (
                right.top_left,
                right.top_right,
                right.bottom_right,
                right.bottom_left
            ),
            (10.0, 0.0, 10.0, 0.0)
        );
        let left = leaf_radius([false; 4], 10.0, false);
        assert_eq!(
            (
                left.top_left,
                left.top_right,
                left.bottom_right,
                left.bottom_left
            ),
            (0.0, 10.0, 0.0, 10.0)
        );
        // The top side is flush: leaf-right loses the top-left round and
        // leaf-left loses the top-right one.
        assert_eq!(leaf_radius([true, false, false, false], 10.0, true).top_left, 0.0);
        assert_eq!(
            leaf_radius([true, false, false, false], 10.0, false).top_right,
            0.0
        );
    }

    /// A stacked title shrinks icons only on a horizontal panel; a
    /// vertical panel's title runs along its length, and a hidden title
    /// consumes nothing at all. A side bar also gets the narrow inset.
    #[test]
    fn icon_room_follows_the_panel_axis() {
        let config = |panels: &str, section_title: bool| {
            serde_json::from_str::<config::Config>(&format!(
                r#"{{"panel":{{"font_size":11,"section_title":{section_title},"sections":[
                    {{"name":"apps","layout":"vertical","sensors":[]}},
                    {{"name":"top","layout":"horizontal","sensors":[]}}
                ]}},"panels":{panels}}}"#
            ))
            .expect("test config")
        };
        let room = |title_extra, pad| IconRoom { title_extra, pad };

        // Vertical section on a horizontal panel with a visible title:
        // font 11 + 2 + 2 × SECTION_CONTENT_PAD (4) = 21.
        assert_eq!(
            section_icon_room(&config(r#"[{"anchor":"top","sections":["apps"]}]"#, true), 11),
            vec![room(21, 4), room(0, 4)]
        );
        // The same section on a vertical panel: no title cost and the
        // narrow side-bar inset.
        assert_eq!(
            section_icon_room(&config(r#"[{"anchor":"left","sections":["apps"]}]"#, true), 11),
            vec![room(0, 2), room(0, 4)]
        );
        // Hidden titles cost nothing anywhere.
        assert_eq!(
            section_icon_room(&config(r#"[{"anchor":"top","sections":["apps"]}]"#, false), 11),
            vec![room(0, 4), room(0, 4)]
        );
        // A horizontal section's title leads the row (length axis).
        assert_eq!(
            section_icon_room(&config(r#"[{"anchor":"top","sections":["top"]}]"#, true), 11),
            vec![room(0, 4), room(0, 4)]
        );
        // Shown on both: the horizontal panel's room wins.
        assert_eq!(
            section_icon_room(
                &config(
                    r#"[{"anchor":"top","sections":["apps"]},{"anchor":"left","sections":["apps"]}]"#,
                    true
                ),
                11
            ),
            vec![room(21, 4), room(0, 4)]
        );
    }
}
