//! Tweaks tab: session settings that patch the compositor at runtime
//! instead of the bar's own look. One card per tweak — the dynamic gaps
//! of Hyprland (`panel.tweaks.dynamic_gaps`) and the floating
//! window-actions menu (`panel.tweaks.window_menu`).

use iced::widget::{button, column, pick_list, text, toggler, Row, Slider};
use iced::{Alignment, Element, Length};

use crate::controls::{color_control, form_grid, invalid_under, slider_control};
use crate::model::{
    window_menu_label, AutoHideCfg, BordersCfg, GlassCfg, ScreensaverCfg, ShadowsCfg,
    BORDER_ACTIVE, BORDER_INACTIVE, SHADOW_COLOR, SHADOW_COLOR_INACTIVE, WINDOW_MENU_POSITIONS,
};
use crate::state::{Message, PiumaSetup};
use crate::style::*;

/// A slider row whose negative side is a real value: the shared
/// `slider_control` reads negatives as the style-nums' "-1 = disabled"
/// and would label every one of them `-1`.
fn signed_slider<'a>(
    range: std::ops::RangeInclusive<i32>,
    value: i32,
    invalid: Option<&'a str>,
    on_input: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    invalid_under(
        Row::new()
            .push(Slider::new(range, value, on_input).width(Length::Fill))
            .push(text(value.to_string()).size(BODY).width(56))
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into(),
        invalid,
    )
}

/// One labelled slider row (value on the right, like the panel tab's).
fn gap_slider<'a>(
    label: &'a str,
    value: i32,
    range: std::ops::RangeInclusive<i32>,
    on_input: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    Row::new()
        .push(text(label).size(BODY).width(42))
        .push(Slider::new(range, value, on_input).width(Length::Fill))
        .push(text(value.to_string()).size(BODY).width(28))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// The Tweaks tab: one card per tweak, each with its own switch, settings
/// and explanation — a single card would make the two hard to tell apart.
pub(crate) fn tweaks_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    column![
        text(
            "Settings that patch the compositor at runtime. When a tweak is off \
             nothing is queried or dispatched."
        )
        .size(BODY)
        .style(muted_text(pal)),
        dynamic_gaps_card(state, pal),
        window_menu_card(state, pal),
        shadows_card(state, pal),
        borders_card(state, pal),
        screensaver_card(state, pal),
        glass_card(state, pal),
        auto_hide_card(state, pal),
    ]
    .spacing(12)
    .width(Length::Fill)
    .into()
}

/// `panel.tweaks.dynamic_gaps`: gap size driven by the open-window count.
fn dynamic_gaps_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let gaps = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.dynamic_gaps.as_ref());
    let enabled = gaps.is_some_and(|g| g.enabled);
    // Absent block = the bar's own fallbacks (off, 4..16).
    // Both values live in 1..=200 and the sliders are mutually bounded:
    // the minimum cannot pass the maximum, nor the maximum drop below the
    // minimum. A hand-edited invalid pair is clamped for display and
    // still flagged by the shared validation.
    let max = gaps.map_or(16, |g| g.max).clamp(1, 200);
    let min = gaps.map_or(4, |g| g.min).clamp(1, max);
    card(
        pal,
        column![
            text("Dynamic gaps").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("Shrink the gaps as more windows open")
                .on_toggle(Message::DynamicGapsEnabled),
            invalid_under(
                form_grid(vec![(
                    "Gaps (px)",
                    column![
                        gap_slider("Min", min, 1..=max, Message::GapMinChanged),
                        gap_slider("Max", max, min..=200, Message::GapMaxChanged),
                    ]
                    .spacing(6)
                    .width(Length::Fill)
                    .into(),
                )]),
                state.field_err("panel.tweaks.dynamic_gaps"),
            ),
            text(
                "The gap to the screen edge (general:gaps_out) starts at the maximum with one \
                 open window and shrinks by 25% of (maximum - minimum) for every extra open \
                 window, never below the minimum; the gap between windows (general:gaps_in) is \
                 half of it. All open windows count, so switching workspace does not re-tile \
                 the screen. With 4 and 16 the edge gap goes 16, 13, 10, 7, 4 (inner 8, 6, 5, \
                 3, 2). Turning the tweak off restores Hyprland's own values."
            )
            .size(BODY)
            .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// `panel.tweaks.window_menu`: the floating close/float/sticky menu.
fn window_menu_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let menu = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.window_menu.as_ref());
    let enabled = menu.is_some_and(|m| m.enabled);
    let position = menu.map(|m| m.position.as_str()).unwrap_or("top-center");
    let options: Vec<&str> = WINDOW_MENU_POSITIONS.iter().map(|(_, label)| *label).collect();
    card(
        pal,
        column![
            text("Window menu").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("Close / float / sticky for the focused window")
                .on_toggle(Message::WindowMenuEnabled),
            invalid_under(
                form_grid(vec![(
                    "Position",
                    pick_list(
                        options,
                        Some(window_menu_label(position)),
                        |label: &str| Message::WindowMenuPositionChanged(label.to_string()),
                    )
                    .width(Length::Fill)
                    .into(),
                )]),
                state.field_err("panel.tweaks.window_menu"),
            ),
            text(
                "The menu follows the focused window: one small pill, 4 px above or below it \
                 and flush with its left or right edge (inside the window when it sits against \
                 the screen edge), with close, float and sticky buttons. Sticky floats a tiled window before pinning it, as \
                 Hyprland only pins floating windows. The menu never takes focus, so clicking \
                 it leaves the window active. Changing the toggle or the position applies at \
                 the next second, without restarting."
            )
            .size(BODY)
            .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// `panel.tweaks.shadows`: Hyprland's `decoration:shadow:*`, every option
/// the compositor takes.
fn shadows_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let shadows = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.shadows.as_ref());
    let enabled = shadows.is_some_and(|s| s.enabled);
    // Absent block = the bar's (and Hyprland's) defaults.
    let d = ShadowsCfg::default();
    let range = shadows.map_or(d.range, |s| s.range);
    let power = shadows.map_or(d.render_power, |s| s.render_power);
    let sharp = shadows.is_some_and(|s| s.sharp);
    let color = shadows.map_or(SHADOW_COLOR, |s| s.color.as_str());
    let inactive = shadows.map_or(SHADOW_COLOR_INACTIVE, |s| s.color_inactive.as_str());
    let offset = shadows.map_or(d.offset, |s| s.offset);
    let scale = shadows.map_or(d.scale, |s| s.scale);
    let err = |path: &str| state.field_err(path);
    card(
        pal,
        column![
            text("Window shadows").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("Apply these shadow settings to Hyprland")
                .on_toggle(Message::ShadowsEnabled),
            invalid_under(
                form_grid(vec![
                    (
                        "Range",
                        slider_control(
                            0..=100,
                            range,
                            err("panel.tweaks.shadows.range"),
                            Message::ShadowRangeChanged,
                        ),
                    ),
                    (
                        "Render power",
                        slider_control(
                            1..=4,
                            power,
                            err("panel.tweaks.shadows.render_power"),
                            Message::ShadowPowerChanged,
                        ),
                    ),
                    (
                        "Scale",
                        invalid_under(
                            tenths_slider(scale, Message::ShadowScaleChanged),
                            err("panel.tweaks.shadows.scale"),
                        ),
                    ),
                    (
                        "Sharp",
                        toggler(sharp)
                            .label("Sharp edge")
                            .on_toggle(Message::ShadowSharpChanged)
                            .into(),
                    ),
                    (
                        "Color",
                        color_control(
                            color,
                            err("panel.tweaks.shadows.color"),
                            Message::ShadowColorChanged,
                        ),
                    ),
                    (
                        "Inactive color",
                        color_control(
                            inactive,
                            err("panel.tweaks.shadows.color_inactive"),
                            Message::ShadowInactiveColorChanged,
                        ),
                    ),
                    (
                        "Offset X",
                        signed_slider(
                            -50..=50,
                            offset[0],
                            err("panel.tweaks.shadows.offset"),
                            |value| Message::ShadowOffsetChanged(0, value),
                        ),
                    ),
                    (
                        "Offset Y",
                        signed_slider(
                            -50..=50,
                            offset[1],
                            err("panel.tweaks.shadows.offset"),
                            |value| Message::ShadowOffsetChanged(1, value),
                        ),
                    ),
                ]),
                None,
            ),
            text(
                "Hyprland's decoration:shadow block, written with one hl.config call when this \
                 switch is on; turning it off (or quitting piuma) puts your own values back. \
                 Range is the blur radius (0-100), render power the blur passes (1-4: higher is \
                 smoother and heavier) and scale sizes the shadow around the window. Offset \
                 shifts it in pixels; the inactive colour only shows where no_shadow window \
                 rules do not hide unfocused shadows. Colours are #rrggbbaa, alpha included."
            )
            .size(BODY)
            .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// The shadow scale as a slider over tenths (1.0 shows as 1.0, not 10).
fn tenths_slider<'a>(
    value: f32,
    on_input: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    let tenths = (value * 10.0).round().clamp(1.0, 100.0) as i32;
    Row::new()
        .push(Slider::new(1..=100, tenths, on_input).width(Length::Fill))
        .push(
            text(format!("{:.1}", tenths as f32 / 10.0))
                .size(BODY)
                .width(56),
        )
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// `panel.tweaks.borders`: `general:border_size` and the two border
/// colours.
fn borders_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let borders = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.borders.as_ref());
    let enabled = borders.is_some_and(|b| b.enabled);
    // Absent block = the bar's (and Hyprland's) defaults.
    let d = BordersCfg::default();
    let size = borders.map_or(d.size, |b| b.size);
    let active = borders.map_or(BORDER_ACTIVE, |b| b.active.as_str());
    let inactive = borders.map_or(BORDER_INACTIVE, |b| b.inactive.as_str());
    let err = |path: &str| state.field_err(path);
    card(
        pal,
        column![
            text("Window borders").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("Apply these border settings to Hyprland")
                .on_toggle(Message::BordersEnabled),
            invalid_under(
                form_grid(vec![
                    (
                        "Thickness",
                        slider_control(
                            0..=20,
                            size,
                            err("panel.tweaks.borders.size"),
                            Message::BorderSizeChanged,
                        ),
                    ),
                    (
                        "Active color",
                        color_control(
                            active,
                            err("panel.tweaks.borders.active"),
                            Message::BorderActiveChanged,
                        ),
                    ),
                    (
                        "Inactive color",
                        color_control(
                            inactive,
                            err("panel.tweaks.borders.inactive"),
                            Message::BorderInactiveChanged,
                        ),
                    ),
                ]),
                None,
            ),
            text(
                "Hyprland's general:border_size and general:col.active_border / \
                 inactive_border, written with one hl.config call while this switch is on. \
                 Thickness 0 removes the border; the active colour is the focused window's. \
                 The first time the switch is turned on the fields are filled from the values \
                 Hyprland is using right now, so nothing changes until you edit one. Turning \
                 the tweak off (or quitting piuma) puts your own values back."
            )
            .size(BODY)
            .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// `panel.tweaks.screensaver`: the idle overlay, its delay and a button
/// that shows it now (SIGUSR1 to the bar).
fn screensaver_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let saver = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.screensaver.as_ref());
    let enabled = saver.is_some_and(|s| s.enabled);
    // Absent block = the bar's defaults (off, 10 minutes).
    let d = ScreensaverCfg::default();
    let minutes = saver.map_or(d.minutes, |s| s.minutes) as i32;
    card(
        pal,
        column![
            text("Screensaver").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("After the idle delay, cover the outputs with palette rectangles")
                .on_toggle(Message::ScreensaverEnabled),
            invalid_under(
                form_grid(vec![(
                    "Idle minutes",
                    slider_control(
                        1..=120,
                        minutes,
                        state.field_err("panel.tweaks.screensaver.minutes"),
                        |value| Message::ScreensaverMinutesChanged(value as u32),
                    ),
                )]),
                None,
            ),
            button(text("Lock now").size(BODY))
                .on_press(Message::ScreensaverShow)
                .padding(10)
                .style(move |t, s| btn_secondary(pal, t, s)),
            text(
                "Locks the screen after the idle delay with a palette grid, unlocked by your \
                 session password (no hyprlock needed). A `lock` sensor locks at once; it \
                 stays off while a window is fullscreen or a video is playing."
            )
            .size(BODY)
            .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// `panel.tweaks.glass`: compositor blur behind the bar, with the bar
/// alpha that lets it show.
fn glass_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let glass = state.config.tweaks.as_ref().and_then(|t| t.glass.as_ref());
    let enabled = glass.is_some_and(|g| g.enabled);
    let d = GlassCfg::default();
    let opacity = (glass.map_or(d.opacity, |g| g.opacity) * 100.0).round() as i32;
    card(
        pal,
        column![
            text("Glass").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("Blur behind the bar and its popups")
                .on_toggle(Message::GlassEnabled),
            invalid_under(
                form_grid(vec![(
                    "Bar opacity",
                    slider_control(
                        5..=100,
                        opacity,
                        state.field_err("panel.tweaks.glass.opacity"),
                        Message::GlassOpacityChanged,
                    ),
                )]),
                None,
            ),
            text("Uses the compositor's own blur; the bar's background keeps its colour at the chosen opacity.")
                .size(BODY)
                .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// `panel.tweaks.auto_hide`: the bar leaves the screen while a
/// fullscreen window has focus.
fn auto_hide_card(state: &PiumaSetup, pal: Palette) -> Element<'_, Message> {
    let enabled = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.auto_hide.as_ref())
        .is_some_and(|a| a.enabled);
    let _ = AutoHideCfg::default();
    card(
        pal,
        column![
            text("Auto-hide").size(16).style(fg_text(pal)),
            toggler(enabled)
                .label("Hide the bar while a fullscreen window has focus")
                .on_toggle(Message::AutoHideEnabled),
            text("The reserved strip is given back to the window; the bar returns when fullscreen ends.")
                .size(BODY)
                .style(muted_text(pal)),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}
