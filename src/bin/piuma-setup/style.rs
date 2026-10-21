//! Visual tokens and widget styles: the palette derived from the bar's
//! own config, button/card/text styles and the theme handed to iced.
use iced::theme::Palette as IcedPalette;
use iced::widget::text::Style as TextStyle;
use iced::widget::{button, container, Container};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::config;
use crate::model::PiumaConfig;
use crate::state::{Message, PiumaSetup};

/// The one red every invalid-value highlight shares (text, borders, badge).
pub(crate) const ERROR_RED: Color = Color::from_rgb(0.9, 0.3, 0.3);
/// The GUI theme:iced's palette fed from the bar's own config, so inputs,
/// pick lists, togglers and scrollbars follow the bar's colours instead of
/// a fixed dark variant.
pub(crate) fn theme(state: &PiumaSetup) -> Theme {
    let pal = palette(&state.config);
    Theme::custom(
        "Piuma",
        IcedPalette {
            background: pal.bg,
            text: pal.fg,
            primary: pal.accent,
            success: IcedPalette::DARK.success,
            warning: IcedPalette::DARK.warning,
            danger: ERROR_RED,
        },
    )
}

// ---------------------------------------------------------------------------
// View helpers: cards, aligned form rows and collapsible blocks
// ---------------------------------------------------------------------------

/// Fixed label column width so every form row lines up and labels never
/// overlap their controls (`Length` only accepts u32).
pub(crate) const LABEL_WIDTH: u32 = 110;

/// Body text size: the iced default (16 pt) reduced by 2 pt as requested.
pub(crate) const BODY: u32 = 14;

/// Bar fallbacks mirrored from `src/main.rs` (DEFAULT_FONT / DEFAULT_*_COLOR),
/// alpha forced opaque: this is a window, not a translucent bar layer.
pub(crate) const SETUP_DEFAULT_FONT: &str = "Noto Sans";
pub(crate) const SETUP_DEFAULT_BG: Color = Color::from_rgba(0.06, 0.06, 0.08, 1.0);
pub(crate) const SETUP_DEFAULT_FG: Color = Color::from_rgb(0.88, 0.88, 0.94);

/// Visual tokens derived from the bar's own config: every surface, border
/// and text colour below comes from `piuma.json` through the same
/// `config::parse_color` the bar uses, so setup and bar can't disagree.
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) bg: Color,           // window background (`default_bg_color`)
    pub(crate) fg: Color,           // primary text (`default_text_color`)
    pub(crate) muted: Color,        // secondary text (fg blended toward bg)
    pub(crate) card: Color,         // card surface (`section_defaults.bg_color`)
    pub(crate) card_border: Color,  // card hairline (section border, kept visible)
    pub(crate) chrome: Color,       // rail/footer (bg blended toward card)
    pub(crate) inset: Color,        // child surface (card blended toward bg)
    pub(crate) accent: Color,       // primary/selected (`sensor_defaults.bg_color`)
    pub(crate) accent_text: Color,  // text on accent (hover text → normal text → fg)
    pub(crate) accent_hover: Color, // hover ring (`border_color_hover`)
    pub(crate) radius: f32,         // card corners (`section_defaults.border_radius`)
    pub(crate) pill: f32,           // button corners (`sensor_defaults.border_radius`)
}

/// Linear blend `a → b` by `t` (0 = a, 1 = b).
pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// GTK windows (and this one) are opaque: surfaces drop their alpha, the
/// same way the N3 theme bridge drops it in its generated file.
pub(crate) fn opaque(c: Color) -> Color {
    Color { a: 1.0, ..c }
}

/// Resolve a config colour like the bar does (main.rs): `config::parse_color`,
/// else the mirrored bar default.
pub(crate) fn resolved(value: &Option<String>, fallback: Color) -> Color {
    opaque(
        value
            .as_deref()
            .and_then(|s| config::parse_color(s).ok())
            .unwrap_or(fallback),
    )
}

/// The whole token set from the loaded config (cheap: re-derived per call).
pub(crate) fn palette(cfg: &PiumaConfig) -> Palette {
    let bg = resolved(&cfg.default_bg_color, SETUP_DEFAULT_BG);
    let fg = resolved(&cfg.default_text_color, SETUP_DEFAULT_FG);
    let opaque_visible = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|s| config::parse_color(s).ok())
            .filter(|c| c.a > 0.0)
            .map(opaque)
    };
    // Surfaces: the configured colour when it draws something, otherwise a
    // token re-derived from bg/fg so the GUI keeps visible structure.
    let card = opaque_visible(&cfg.section_defaults.bg_color)
        .unwrap_or_else(|| mix(bg, Color::WHITE, 0.07));
    // A transparent section border draws nothing on the bar; the GUI still
    // needs a hairline, re-derived from the text colour.
    let card_border =
        opaque_visible(&cfg.section_defaults.border_color).unwrap_or_else(|| mix(fg, bg, 0.82));
    let accent = opaque_visible(&cfg.sensor_defaults.bg_color).unwrap_or_else(|| mix(fg, bg, 0.75));
    let accent_text = cfg
        .sensor_defaults
        .text_color_hover
        .as_deref()
        .or(cfg.sensor_defaults.text_color.as_deref())
        .and_then(|s| config::parse_color(s).ok())
        .map(opaque)
        .unwrap_or(fg);
    let accent_hover = opaque_visible(&cfg.sensor_defaults.border_color_hover)
        .unwrap_or_else(|| mix(accent, Color::WHITE, 0.25));
    let radius = cfg
        .section_defaults
        .border_radius
        .map(|r| (r.max(0) as f32).min(16.0))
        .unwrap_or(6.0);
    let pill = cfg
        .sensor_defaults
        .border_radius
        .map(|r| (r.max(0) as f32).min(16.0))
        .unwrap_or(6.0);
    Palette {
        bg,
        fg,
        muted: mix(fg, bg, 0.45),
        card,
        card_border,
        chrome: mix(bg, card, 0.4),
        inset: mix(card, bg, 0.5),
        accent,
        accent_text,
        accent_hover,
        radius,
        pill,
    }
}

/// Text style in the palette's primary colour (shared by the explicit
/// `.style` calls of the editor).
pub(crate) fn fg_text(pal: Palette) -> impl Fn(&Theme) -> TextStyle {
    move |_t: &Theme| TextStyle {
        color: Some(pal.fg),
        ..TextStyle::default()
    }
}

/// Text style in the palette's secondary colour.
pub(crate) fn muted_text(pal: Palette) -> impl Fn(&Theme) -> TextStyle {
    move |_t: &Theme| TextStyle {
        color: Some(pal.muted),
        ..TextStyle::default()
    }
}

/// Primary button skin: the bar's sensor language — accent pill, the hover
/// ring (`border_color_hover`) appearing only while hovered.
pub(crate) fn btn_primary(pal: Palette, t: &Theme, s: button::Status) -> button::Style {
    let mut st = button::primary(t, s); // the built-in disabled handling stays
    match s {
        button::Status::Disabled => {}
        button::Status::Hovered => {
            st.background = Some(Background::Color(mix(pal.accent, Color::WHITE, 0.12)));
            st.text_color = pal.accent_text;
            st.border = Border {
                width: 1.0,
                color: pal.accent_hover,
                radius: pal.pill.into(),
            };
        }
        _ => {
            st.background = Some(Background::Color(pal.accent));
            st.text_color = pal.accent_text;
            st.border = Border {
                width: 0.0,
                color: pal.accent_hover,
                radius: pal.pill.into(),
            };
        }
    }
    st
}

/// Secondary button skin: the card surface + the config hairline.
pub(crate) fn btn_secondary(pal: Palette, t: &Theme, s: button::Status) -> button::Style {
    let mut st = button::secondary(t, s);
    if !matches!(s, button::Status::Disabled) {
        let bg = match s {
            button::Status::Hovered => mix(pal.card, Color::WHITE, 0.06),
            _ => pal.card,
        };
        st.background = Some(Background::Color(bg));
        st.text_color = pal.fg;
        st.border = Border {
            width: 1.0,
            color: pal.card_border,
            radius: pal.pill.into(),
        };
    }
    st
}

/// The 16x16 swatch colour of a typed value: the bar's own parser, with
/// the grey "unparsable" fallback the editor uses for blank fields.
pub(crate) fn hex_color(value: &str) -> Color {
    config::parse_color(value).unwrap_or(Color::from_rgb(0.35, 0.35, 0.35))
}

/// Text style naming an invalid value: the single red used by every
/// highlighted field, reason line, badge and refusal list.
pub(crate) fn error_text_style(_t: &Theme) -> TextStyle {
    TextStyle {
        color: Some(ERROR_RED),
        ..TextStyle::default()
    }
}

/// Shared card style: the section surface, its hairline and the radius
/// from `panel.section_defaults`.
pub(crate) fn card_style(pal: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_t: &Theme| container::Style {
        background: Some(Background::Color(pal.card)),
        border: Border {
            width: 1.0,
            color: pal.card_border,
            radius: pal.radius.into(),
        },
        ..container::Style::default()
    }
}

/// The visual unit of the nested layout (panel > section > sensor).
pub(crate) fn card<'a>(pal: Palette, content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content.into())
        .padding(12)
        .width(Length::Fill)
        .style(card_style(pal))
}

/// Inset background marking content as a child of the surrounding card.
pub(crate) fn inset_style(pal: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_t: &Theme| container::Style {
        background: Some(Background::Color(pal.inset)),
        border: Border {
            width: 1.0,
            color: pal.card_border,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

