//! Tweaks tab: session settings that patch the compositor at runtime
//! instead of the bar's own look. Currently the dynamic gaps of
//! Hyprland (`panel.tweaks.dynamic_gaps`).

use iced::widget::{column, text, toggler, Row, Slider};
use iced::{Alignment, Element, Length};

use crate::controls::{form_grid, invalid_under};
use crate::state::{Message, PiumaSetup};
use crate::style::*;

/// One labelled slider row (value on the right, like the panel tab's).
fn gap_slider<'a>(
    label: &'a str,
    value: i32,
    on_input: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    Row::new()
        .push(text(label).size(BODY).width(42))
        .push(Slider::new(0..=50, value, on_input).width(Length::Fill))
        .push(text(value.to_string()).size(BODY).width(28))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// The Tweaks card: currently the Hyprland dynamic gaps.
pub(crate) fn tweaks_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let gaps = state
        .config
        .tweaks
        .as_ref()
        .and_then(|t| t.dynamic_gaps.as_ref());
    let enabled = gaps.is_some_and(|g| g.enabled);
    // Absent block = the bar's own fallbacks (off, 4..16).
    let min = gaps.map_or(4, |g| g.min);
    let max = gaps.map_or(16, |g| g.max);

    card(
        pal,
        column![
            text("Session Tweaks").size(16).style(fg_text(pal)),
            text(
                "Settings that patch the compositor at runtime. When a tweak is off \
                 nothing is queried or dispatched."
            )
            .size(BODY)
            .style(muted_text(pal)),
            form_grid(vec![
                (
                    "Enable",
                    toggler(enabled)
                        .label("Dynamic gaps")
                        .on_toggle(Message::DynamicGapsEnabled)
                        .into(),
                ),
                (
                    "Gaps (px)",
                    invalid_under(
                        column![
                            gap_slider("Min", min, Message::GapMinChanged),
                            gap_slider("Max", max, Message::GapMaxChanged),
                        ]
                        .spacing(6)
                        .width(Length::Fill)
                        .into(),
                        state.field_err("panel.tweaks.dynamic_gaps"),
                    ),
                ),
            ]),
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
