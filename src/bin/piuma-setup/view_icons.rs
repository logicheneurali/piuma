//! Icons tab: the theme the sensor icons resolve from and the edge they
//! draw at (`panel.icon_size`; absent = auto, the row's own fit).

use iced::widget::{column, text, toggler, Row, Slider};
use iced::{Element, Length};

use crate::controls::{
    form_grid, icon_theme_options, invalid_under, section_content_pad, theme_cell,
    SYSTEM_ICON_THEME,
};
use crate::icons;
use crate::state::{Message, PiumaSetup};
use crate::style::*;

/// Icons tab: the size override (auto or an explicit edge) and the theme
/// grid that used to sit at the bottom of the Panel card.
pub(crate) fn icons_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    // The representative auto edge: a horizontal section with no stacked
    // title — what the slider label starts from when auto is turned off.
    let auto_px = icons::row_icon_target(
        state.config.thickness,
        section_content_pad(&state.config, false),
        0,
        0,
    );
    let auto = state.config.icon_size.is_none();
    let size = state.config.icon_size.unwrap_or(u32::from(auto_px));

    let size_row: Element<'_, Message> = if auto {
        text(format!("auto ({auto_px}px)"))
            .size(BODY)
            .color(pal.fg)
            .into()
    } else {
        Row::new()
            .push(Slider::new(4..=48u32, size, Message::IconSizeChanged).width(Length::Fill))
            .push(text(format!("{size}px")).size(BODY).width(56))
            .spacing(8)
            .width(Length::Fill)
            .into()
    };

    card(
        pal,
        column![
            text("Icons").size(16).style(fg_text(pal)),
            text(
                "Edge every sensor, tray and taskbar icon draws at. Auto fills the \
                 sensor row; an explicit value never outgrows it."
            )
            .size(10)
            .style(muted_text(pal)),
            form_grid(vec![
                (
                    "Auto",
                    toggler(auto)
                        .label("Fill the row")
                        .on_toggle(Message::IconSizeAuto)
                        .into(),
                ),
                (
                    "Icon Size",
                    invalid_under(size_row, state.field_err("panel.icon_size")),
                ),
            ]),
            icon_theme_grid(state),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}

/// Icon themes as a 3×3-visible grid of cells previewing three sample
/// icons in that theme; the rest scrolls.
fn icon_theme_grid(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let current = state.config.icon_theme.as_deref().unwrap_or("");
    let mut options: Vec<Option<String>> = vec![None];
    options.extend(
        icon_theme_options(state.config.icon_theme.as_deref())
            .into_iter()
            .filter(|theme| theme != SYSTEM_ICON_THEME)
            .map(Some),
    );
    let mut grid = column![].spacing(6).width(Length::Fill);
    let mut row = Row::new().spacing(6).width(Length::Fill);
    let mut count = 0;
    for option in options {
        let label = option.clone().unwrap_or_else(|| {
            format!("{SYSTEM_ICON_THEME} ({})", icons::icon_theme())
        });
        let selected = option.as_deref().unwrap_or("") == current;
        row = row.push(theme_cell(pal, option.as_deref(), &label, selected));
        count += 1;
        if count % 3 == 0 {
            grid = grid.push(row);
            row = Row::new().spacing(6).width(Length::Fill);
        }
    }
    if count % 3 != 0 {
        grid = grid.push(row);
    }
    column![
        text("Icon Theme").size(12).style(fg_text(pal)),
        text("Three sample icons per theme, three rows visible — scroll for the rest.")
            .size(10)
            .style(muted_text(pal)),
        iced::widget::scrollable(grid)
            .height(Length::Fixed(200.0))
            .width(Length::Fill),
    ]
    .spacing(6)
    .into()
}
