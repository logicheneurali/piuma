//! The style matrix: colours resolved from config + palette, the
//! background, and applying a resolved matrix to the panel.

use crate::config;
use crate::geometry::section_touch;
use crate::palette;
use crate::{
    icons, icons_theme, panel_thickness, Panel, SectionStyle, SensorStyle, DEFAULT_BG_COLOR,
    DEFAULT_FONT_SIZE, DEFAULT_TEXT_COLOR,
};
use iced::Color;

pub(crate) fn resolve_color<'a>(cfg: Option<Color>, default: Color) -> Color {
    cfg.unwrap_or(default)
}

/// The panel's background: the wallpaper palette's base tone when it's
/// active — keeping the configured colour's TRANSPARENCY (a `#000000ff`
/// bar renders opaque in the palette's hue, a `#00000000` one stays
/// see-through) — else the configured `default_bg_color`, else the
/// built-in default. Used by both the surface's container and the
/// sections' fallback, so they can't disagree.
pub(crate) fn panel_background(config: &config::Config, pal: Option<&palette::Palette>) -> Color {
    let configured = config
        .panel
        .default_bg_color
        .as_ref()
        .and_then(|s| config::parse_color(s).ok());
    pal.and_then(|p| config::parse_color(&p.bg).ok())
        .map(|mut c| {
            if let Some(b) = configured {
                c.a = b.a;
            }
            c
        })
        .or(configured)
        .unwrap_or(DEFAULT_BG_COLOR)
}

/// Every section's and sensor's resolved style: the precedence chains
/// the tick used to re-run per sensor per second — `parse_color` on
/// every colour, `to_string` on every shape field — computed once per
/// config load and once per wallpaper-palette change instead. The
/// readings stay out of it: the tick pairs them with these styles.
pub(crate) fn resolve_styles(
    cfg: &config::Config,
    pal: Option<&palette::Palette>,
) -> Vec<SectionStyle> {
    let icon_theme = icons_theme(cfg);
    let thickness = panel_thickness(cfg);
    let font_size = cfg.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let default_bg = panel_background(cfg, pal);
    cfg.panel
        .sections
        .iter()
        .enumerate()
        .map(|(section_idx, section_cfg)| {
            let section_default_txt = pal
                .and_then(|p| config::parse_color(&p.text).ok())
                .or_else(|| {
                    cfg.panel
                        .default_text_color
                        .as_ref()
                        .and_then(|s| config::parse_color(s).ok())
                })
                .unwrap_or(DEFAULT_TEXT_COLOR);

            // Text colour chain: section > wallpaper palette (when
            // active) > section_defaults > panel > built-in. The
            // palette keeps the configured transparency, like bg.
            let section_text_color = section_cfg
                .text_color
                .as_ref()
                .and_then(|s| config::parse_color(s).ok())
                .or_else(|| {
                    let configured = cfg
                        .panel
                        .section_defaults
                        .as_ref()
                        .and_then(|sd| sd.text_color.as_ref())
                        .and_then(|s| config::parse_color(s).ok());
                    pal.and_then(|p| config::parse_color(&p.text).ok())
                        .map(|mut t| {
                            if let Some(c) = configured {
                                t.a = c.a;
                            }
                            t
                        })
                })
                .or_else(|| {
                    cfg.panel
                        .section_defaults
                        .as_ref()
                        .and_then(|sd| sd.text_color.as_ref())
                        .and_then(|s| config::parse_color(s).ok())
                })
                .unwrap_or(section_default_txt);

            let sensors: Vec<SensorStyle> = section_cfg
                .sensors
                .iter()
                .map(|s| {
                    // Style precedence, per style GROUP: a sensor takes a
                    // group over only when it defines the group's key
                    // colour (`border_color`, `bg_color`, `text_color`).
                    // When that key is null on the sensor, every override
                    // of that group on the sensor is ignored and
                    // `cfg.panel.sensor_defaults` applies as a whole. When
                    // the key is set, the sensor's own fields win and the
                    // remaining ones fall back to the defaults.
                    // `padding` is a single value: sensor when set,
                    // otherwise the default. It also feeds the icon fit
                    // below, so it is resolved before the icon.
                    let defaults = cfg.panel.sensor_defaults.as_ref();
                    let padding = config::style_num(s.padding)
                        .or(defaults.and_then(|sd| config::style_num(sd.padding)))
                        .unwrap_or(0) as u16;
                    // Roadmap N1: theme file for the sensor's icon, sized
                    // to this sensor row's real height — the fit the tray
                    // already uses (thickness minus the layout chrome,
                    // clamped), so the icon fills its box instead of
                    // trailing the font size. The size travels with the
                    // path (resolve and widget must agree on it); the
                    // lookup is memoized, so a rebuild only clones the
                    // cached path. With `icon_theme == None` (gate) this
                    // arm never runs.
                    let icon_size = icons::row_icon_target(
                        thickness,
                        icons::stacked_title_extra(
                            section_cfg.layout.as_deref().unwrap_or("horizontal"),
                            &section_cfg.name,
                            font_size,
                        ),
                        u32::from(padding),
                    );
                    let icon = match (icon_theme, s.icon.as_deref()) {
                        (Some(theme), Some(icon_name)) => {
                            icons::resolve(theme, icon_name, icon_size)
                        }
                        _ => None,
                    };
                    let own_border = s.border_color.is_some();
                    let own_bg = s.bg_color.is_some();
                    let own_text = s.text_color.is_some();
                    // Key fields: per-sensor, then the wallpaper
                    // palette (when active), then defaults, then
                    // section/built-in. The palette overrides the
                    // configured defaults only — an explicit
                    // per-sensor value always wins — and the
                    // configured TRANSPARENCY survives it (a
                    // `#..80` default keeps its alpha in the
                    // wallpaper's hue).
                    let tc_pal = pal.map(|p| {
                        palette::keep_config_alpha(
                            &p.text,
                            defaults.and_then(|sd| sd.text_color.as_deref()),
                        )
                    });
                    let tc_str = s
                        .text_color
                        .as_deref()
                        .or(tc_pal.as_deref())
                        .or(defaults.and_then(|sd| sd.text_color.as_deref()));
                    let bg_pal = pal.map(|p| {
                        palette::keep_config_alpha(
                            &p.card,
                            defaults.and_then(|sd| sd.bg_color.as_deref()),
                        )
                    });
                    let bg_str = s
                        .bg_color
                        .as_deref()
                        .or(bg_pal.as_deref())
                        .or(defaults.and_then(|sd| sd.bg_color.as_deref()));
                    let bc_str = s
                        .border_color
                        .as_deref()
                        .or(defaults.and_then(|sd| sd.border_color.as_deref()));
                    // Secondary fields only count when the sensor owns the group.
                    let bg_h_pal = pal.map(|p| {
                        palette::keep_config_alpha(
                            &p.card_hover,
                            defaults.and_then(|sd| sd.bg_color_hover.as_deref()),
                        )
                    });
                    let bg_h_str = (if own_bg {
                        s.bg_color_hover.as_deref()
                    } else {
                        None
                    })
                    .or(bg_h_pal.as_deref())
                    .or(defaults.and_then(|sd| sd.bg_color_hover.as_deref()));
                    let tc_h_pal = pal.map(|p| {
                        palette::keep_config_alpha(
                            &p.text_hover,
                            defaults.and_then(|sd| sd.text_color_hover.as_deref()),
                        )
                    });
                    let tc_h_str = (if own_text {
                        s.text_color_hover.as_deref()
                    } else {
                        None
                    })
                    .or(tc_h_pal.as_deref())
                    .or(defaults.and_then(|sd| sd.text_color_hover.as_deref()));
                    // Numeric style fields follow the -1 = disabled rule:
                    // only an enabled value (>= 0) overrides the defaults.
                    let bw_val = (if own_border {
                        config::style_num(s.border_width)
                    } else {
                        None
                    })
                    .or(defaults.and_then(|sd| config::style_num(sd.border_width)))
                    .unwrap_or(0);
                    let br_val = (if own_border {
                        config::style_num(s.border_radius)
                    } else {
                        None
                    })
                    .or(defaults.and_then(|sd| config::style_num(sd.border_radius)))
                    .unwrap_or(0);
                    let bt_str = (if own_border {
                        s.border_type.as_deref()
                    } else {
                        None
                    })
                    .or(defaults.and_then(|sd| sd.border_type.as_deref()))
                    .unwrap_or("full");

                    let sensor_tc = tc_str
                        .and_then(|sc| config::parse_color(sc).ok())
                        .unwrap_or(section_text_color);
                    let sensor_bg: Option<Color> =
                        bg_str.and_then(|sc| config::parse_color(sc).ok());
                    let sensor_bg_hover: Option<Color> =
                        bg_h_str.and_then(|sc| config::parse_color(sc).ok());
                    let sensor_tc_hover: Option<iced::Color> =
                        tc_h_str.and_then(|sc| config::parse_color(sc).ok());
                    let sensor_border: Option<Color> =
                        bc_str.and_then(|sc| config::parse_color(sc).ok());
                    // Threshold colour rules, colours resolved once and
                    // sorted ascending: the view picks the highest rule
                    // the reading reaches.
                    let mut color_rules: Vec<(f32, Color)> = s
                        .color_rules
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|r| {
                            config::parse_color(&r.color)
                                .ok()
                                .map(|c| (r.min as f32, c))
                        })
                        .collect();
                    color_rules.sort_by(|a, b| a.0.total_cmp(&b.0));
                    SensorStyle {
                        // static_text and tray sensors render only their
                        // content: the name stays in the config (setup
                        // keeps editing it) but never appears on the bar.
                        name: if matches!(
                            s.source,
                            config::SensorSource::StaticText { .. }
                                | config::SensorSource::Tray {}
                                // The buttons carry the names already.
                                | config::SensorSource::Workspace {}
                        ) {
                            String::new()
                        } else {
                            s.name.clone()
                        },
                        description: s
                            .description
                            .as_ref()
                            .map(|d| d.to_string())
                            .unwrap_or_default(),
                        action: s.action.clone(),
                        is_notifications: matches!(
                            s.source,
                            config::SensorSource::Notifications {}
                        ),
                        padding,
                        text_color: sensor_tc,
                        bg_color: sensor_bg,
                        text_color_hover: sensor_tc_hover,
                        bg_color_hover: sensor_bg_hover,
                        border_color: sensor_border,
                        border_width: bw_val as u16,
                        border_radius: br_val as u16,
                        border_type: bt_str.to_string(),
                        color_rules,
                        icon,
                        icon_size,
                    }
                })
                .collect();

            // Section style precedence, per style GROUP (the same rule as
            // `cfg.panel.sensor_defaults`): the border group belongs to the
            // section only when it sets `border_color`; otherwise the
            // whole group comes from `cfg.panel.section_defaults`. bg/text
            // are single keys: section value when set, then the active
            // wallpaper palette, else the default.
            let sdefaults = cfg.panel.section_defaults.as_ref();
            let own_border = section_cfg.border_color.is_some();
            let bg_pal = pal.map(|p| {
                palette::keep_config_alpha(&p.card, sdefaults.and_then(|sd| sd.bg_color.as_deref()))
            });
            let bg_str = section_cfg
                .bg_color
                .as_deref()
                .or(bg_pal.as_deref())
                .or(sdefaults.and_then(|sd| sd.bg_color.as_deref()));
            let bc_str = section_cfg
                .border_color
                .as_deref()
                .or(sdefaults.and_then(|sd| sd.border_color.as_deref()));
            // Numeric fields follow the -1 = disabled rule: only an
            // enabled value (>= 0) overrides the defaults.
            let bw_val = (if own_border {
                config::style_num(section_cfg.border_width)
            } else {
                None
            })
            .or(sdefaults.and_then(|sd| config::style_num(sd.border_width)))
            .unwrap_or(0);
            let br_val = (if own_border {
                config::style_num(section_cfg.border_radius)
            } else {
                None
            })
            .or(sdefaults.and_then(|sd| config::style_num(sd.border_radius)))
            .unwrap_or(0);
            let bt_str = (if own_border {
                section_cfg.border_type.as_deref()
            } else {
                None
            })
            .or(sdefaults.and_then(|sd| sd.border_type.as_deref()))
            .unwrap_or("full");

            SectionStyle {
                name: section_cfg.name.clone(),
                layout: section_cfg
                    .layout
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or("horizontal".to_string()),
                bg_color: resolve_color(
                    bg_str.and_then(|s| config::parse_color(s).ok()),
                    default_bg,
                ),
                text_color: section_text_color,
                // Title = section text colour at 80% alpha (same chain).
                title_color: section_text_color.scale_alpha(0.8),
                border_color: bc_str.and_then(|s| config::parse_color(s).ok()),
                border_width: bw_val as u16,
                border_radius: br_val as u16,
                border_type: bt_str.to_string(),
                // Roadmap M3: the edge rule is a layout hint coming from
                // the defaults only (a section's own border group
                // doesn't carry it).
                touch: if sdefaults
                    .and_then(|sd| sd.respect_screen_edges)
                    .unwrap_or(false)
                {
                    section_touch(cfg, section_idx, cfg.panel.sections.len())
                } else {
                    [false; 4]
                },
                sensors,
            }
        })
        .collect()
}

/// Re-resolve every colour chain: the config (reload) or the runtime
/// wallpaper palette moved, and both feed the same precedence.
pub(crate) fn restyle(panel: &mut Panel) {
    panel.styles = resolve_styles(&panel.config, panel.palette.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel background follows the active palette's base tone,
    /// keeping the configured colour's transparency — and the configured
    /// colour itself when no palette is active.
    #[test]
    fn panel_background_follows_the_palette() {
        let opaque: config::Config =
            serde_json::from_str(r##"{"panel":{"sections":[],"default_bg_color":"#000000ff"}}"##)
                .unwrap();
        let pal = palette::Palette {
            bg: "#481919".to_string(),
            card: "#481919".to_string(),
            card_hover: "#8a3a3a".to_string(),
            text: "#e0d2d2".to_string(),
            text_hover: "#f6f4f4".to_string(),
        };
        assert_eq!(
            panel_background(&opaque, Some(&pal)),
            config::parse_color("#481919").unwrap(),
            "an opaque configured bar renders the palette's hue opaque"
        );
        assert_eq!(
            panel_background(&opaque, None),
            config::parse_color("#000000ff").unwrap()
        );
        let clear: config::Config =
            serde_json::from_str(r##"{"panel":{"sections":[],"default_bg_color":"#00000000"}}"##)
                .unwrap();
        assert_eq!(
            panel_background(&clear, Some(&pal)),
            config::parse_color("#48191900").unwrap(),
            "a transparent configured bar stays see-through in the hue"
        );
    }
}
