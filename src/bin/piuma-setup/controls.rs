//! Form controls (inputs, sliders, pick lists, collapsible cards) and the
//! icon-theme discovery feeding the sensor icon pickers.
use iced::widget::text::Style as TextStyle;
use iced::widget::{button, canvas, column, container, text, PickList, Row, Slider, TextInput, Button, Column};
use iced::{Alignment, Background, Border, Color, Element, Length, Point, Size, Theme};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::config;
use crate::icons;
use crate::style::{
    btn_primary, btn_secondary, error_text_style, hex_color, inset_style, muted_text, palette,
    text_input_style, Palette, BODY, ERROR_RED, LABEL_WIDTH,
};
use crate::model::{PiumaConfig, Section, Sensor};
use crate::state::Message;


/// One label + control pair filling half of a form row.
pub(crate) fn form_cell<'a>(label: &'a str, control: Element<'a, Message>) -> Row<'a, Message> {
    Row::new()
        .push(text(label).size(BODY).width(LABEL_WIDTH))
        .push(control)
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill)
}

/// Text input using the standard body font size.
pub(crate) fn input<'a>(placeholder: &'a str, value: &'a str, on_input: impl Fn(String) -> Message + 'a) -> TextInput<'a, Message> {
    TextInput::new(placeholder, value)
        .on_input(on_input)
        .size(BODY)
        .style(text_input_style)
}

/// Pick list using the standard body font size.
pub(crate) fn picklist<'a, T, L, V>(
    options: L,
    selected: Option<V>,
    on_select: impl Fn(T) -> Message + 'a,
) -> PickList<'a, T, L, V, Message>
where
    T: ToString + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
{
    PickList::new(options, selected, on_select).text_size(BODY)
}

/// Small arrow button reordering a list item; without `enabled` it carries no
/// action, which iced renders as disabled (used at the list boundaries).
pub(crate) fn move_btn<'a>(
    pal: Palette,
    label: &'a str,
    enabled: bool,
    on_press: Message,
) -> Button<'a, Message> {
    let b = button(text(label).size(BODY))
        .padding(5)
        .style(move |t, s| btn_secondary(pal, t, s));
    if enabled {
        b.on_press(on_press)
    } else {
        b
    }
}

// --- Sensor icons: WYSIWYG preview + theme-name browser ----------------

/// The icon theme exactly as the bar computes it: the configured theme
/// or the system one (the resolve key, not a display name).
pub(crate) fn icon_theme_of(cfg: &PiumaConfig) -> &str {
    cfg.icon_theme
        .as_deref()
        .unwrap_or_else(|| icons::icon_theme())
}

/// The sensor's effective padding on the bar's own chain: the sensor's
/// value when enabled (-1 = unset), else `panel.sensor_defaults`, else 0.
pub(crate) fn sensor_padding(cfg: &PiumaConfig, sensor: &Sensor) -> u32 {
    config::style_num(sensor.padding)
        .or_else(|| config::style_num(cfg.sensor_defaults.padding))
        .unwrap_or(0)
}

/// The icon edge for one sensor, exactly as the bar sizes it
/// (`icons::row_icon_target`: the sensor row's real height, clamped):
/// the preview and the browser cells must draw at that size or they
/// would not show what the bar draws.
pub(crate) fn sensor_icon_size(cfg: &PiumaConfig, section: &Section, sensor: &Sensor) -> u16 {
    let layout = section.layout.as_deref().unwrap_or("horizontal");
    let title_extra =
        icons::stacked_title_extra(layout, section.name.as_deref().unwrap_or(""), cfg.font_size);
    let thickness = if cfg.thickness == 0 {
        config::DEFAULT_HEIGHT
    } else {
        cfg.thickness
    };
    icons::row_icon_target(thickness, title_extra, sensor_padding(cfg, sensor))
}

/// The themed icon widget at the sensor row's size, or `None` when the
/// name is empty or the theme has no such icon — the one resolve path
/// shared by the preview box, the browser cells and the compact rows.
pub(crate) fn resolved_icon<'a>(
    cfg: &PiumaConfig,
    size: u16,
    name: Option<&str>,
) -> Option<Element<'a, Message>> {
    let theme = icon_theme_of(cfg);
    name.filter(|n| !n.is_empty())
        .and_then(|n| icons::resolve(theme, n, size))
        .map(|path| icons::widget(&path, size as f32))
}

/// Live preview beside the icon input: the resolved icon in a small
/// inset box, a red ✕ for an unknown name, the empty box while nothing
/// is typed — the name's fate is visible while typing it. Sized to the
/// sensor row, so the box shows the bar's real icon edge.
pub(crate) fn icon_preview<'a>(cfg: &PiumaConfig, size: u16, name: Option<&str>) -> Element<'a, Message> {
    let pal = palette(cfg);
    let inner: Element<'a, Message> = match name.filter(|n| !n.is_empty()) {
        Some(n) => match resolved_icon(cfg, size, Some(n)) {
            Some(icon) => icon,
            None => text("✕")
                .size(BODY)
                .style(|_t: &Theme| TextStyle {
                    color: Some(Color::from_rgb(1.0, 0.35, 0.35)),
                    ..TextStyle::default()
                })
                .into(),
        },
        None => text("").into(),
    };
    let box_size = f32::from(size) + 12.0;
    container(inner)
        .width(Length::Fixed(box_size))
        .height(Length::Fixed(box_size))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(inset_style(pal))
        .into()
}

/// Sentinel label of the "no configured theme" picklist entry: the bar
/// then falls back to the system icon-theme setting.
pub(crate) const SYSTEM_ICON_THEME: &str = "system default";

/// Icon-theme picklist options: sentinel first, then the installed
/// themes sorted — by directory name, the lookup's own keys, so every
/// option resolves (the index's display Name= would not); a hand-typed
/// name missing from the list joins it, so the current value never
/// falls out of the options.
pub(crate) fn icon_theme_options(current: Option<&str>) -> Vec<String> {
    let mut themes = installed_theme_names();
    if let Some(c) = current.filter(|c| !c.is_empty() && !themes.iter().any(|t| t == c)) {
        themes.insert(0, c.to_string());
    }
    themes.insert(0, SYSTEM_ICON_THEME.to_string());
    themes
}

/// The XDG data dirs of the lookup: the user's data home, then
/// $XDG_DATA_DIRS (defaulting to /usr/local/share and /usr/share).
pub(crate) fn xdg_data_dirs() -> Vec<PathBuf> {
    crate::xdg::data_search_dirs()
}

/// Theme roots: the XDG data dirs' `icons` plus the legacy ~/.icons.
pub(crate) fn icon_theme_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = xdg_data_dirs().iter().map(|d| d.join("icons")).collect();
    if let Some(home) = crate::xdg::home() {
        roots.push(home.join(".icons"));
    }
    roots
}

/// Flat pixmaps dirs of the same data dirs: the lookup's last resort,
/// where distro logos live outside every theme (/usr/share/pixmaps).
pub(crate) fn icon_pixmap_dirs() -> Vec<PathBuf> {
    xdg_data_dirs().into_iter().map(|d| d.join("pixmaps")).collect()
}

/// Whether a file is an icon image (the extensions the lookup builds).
pub(crate) fn is_icon_image(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_ascii_lowercase().as_str(),
            "svg" | "svgz" | "png" | "xpm"
        )
    })
}

/// The `Inherits=` parents of a theme's index.theme (unlocalized key):
/// the fallback chain the lookup walks one level deep.
pub(crate) fn index_theme_inherits(dir: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(dir.join("index.theme")) else {
        return Vec::new();
    };
    text.lines()
        .find_map(|line| line.strip_prefix("Inherits=").map(str::to_string))
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Dirs of every root hosting a child with this exact theme name: the
/// lookup keys themes by directory name, so the match is exact ("Breeze"
/// with only `breeze` installed misses, exactly like the bar misses).
pub(crate) fn theme_dirs_by_key(theme: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in icon_theme_roots() {
        let Ok(entries) = fs::read_dir(&root) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let named = path.file_name().is_some_and(|n| n == std::ffi::OsStr::new(theme));
            if named && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// Sorted theme names of one icons root: a directory is a theme when it
/// carries an index.theme, and the name the lookup keys by is the
/// directory name itself (never the index's display `Name=`).
pub(crate) fn theme_names_in_root(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("index.theme").exists())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    names.sort();
    names
}

/// Every installed theme name across the roots (sorted, deduped): the
/// picklist offers lookup keys, so a picked option always resolves.
pub(crate) fn installed_theme_names() -> Vec<String> {
    let mut names: Vec<String> =
        icon_theme_roots().iter().flat_map(|root| theme_names_in_root(root)).collect();
    names.sort();
    names.dedup();
    names
}

/// Recursively collects the icon stems (image files) of a theme dir:
/// the theme module of freedesktop-icons is private, so the browser
/// does its own one-shot directory walk.
pub(crate) fn collect_icon_names(dir: &Path, out: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_icon_names(&path, out);
        } else if let (true, Some(stem)) = (is_icon_image(&path), path.file_stem().and_then(|s| s.to_str())) {
            out.insert(stem.to_string());
        }
    }
}

/// Image stems directly inside a flat dir (no recursion: the base-path
/// fallback of the lookup only checks files at its top level).
pub(crate) fn collect_flat_icon_names(dir: &Path, out: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let (true, Some(stem)) = (is_icon_image(&path), path.file_stem().and_then(|s| s.to_str())) {
            out.insert(stem.to_string());
        }
    }
}

/// Icon names the bar can resolve under `theme`, mirroring the lookup
/// chain of freedesktop-icons: the theme's own dirs (keyed by dir name,
/// an unknown name starts from hicolor), the `Inherits` parents, then
/// the flat base/pixmaps dirs. Sorted, deduped, cached per theme name.
pub(crate) fn theme_icon_names(theme: &str) -> Vec<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(names) = guard.get(theme) {
        return names.clone();
    }
    let mut set = HashSet::new();
    // 1) the theme's dirs; an unknown theme name starts from hicolor.
    let mut dirs = theme_dirs_by_key(theme);
    if dirs.is_empty() {
        dirs = theme_dirs_by_key("hicolor");
    }
    let mut parents: Vec<String> = Vec::new();
    for dir in &dirs {
        collect_icon_names(dir, &mut set);
        parents.extend(index_theme_inherits(dir));
    }
    // 2) the inherited parents, one level like the lookup walks them.
    let mut seen = HashSet::new();
    for parent in parents {
        if !seen.insert(parent.to_ascii_lowercase()) {
            continue;
        }
        for dir in theme_dirs_by_key(&parent) {
            collect_icon_names(&dir, &mut set);
        }
    }
    // 3) the flat base dirs (icons roots + pixmaps): the last resort,
    // where distro logos live outside every theme.
    for dir in icon_theme_roots().into_iter().chain(icon_pixmap_dirs()) {
        collect_flat_icon_names(&dir, &mut set);
    }
    let mut names: Vec<String> = set.into_iter().collect();
    names.sort();
    guard.insert(theme.to_string(), names.clone());
    names
}

/// Grid bounds of the icon browser: a broad query still renders a
/// bounded widget count (names shown = cap at most).
pub(crate) const ICON_GRID_CAP: usize = 120;
pub(crate) const ICONS_PER_ROW: usize = 4;

/// Case-insensitive substring filter over the theme names, sorted and
/// capped for the browser grid.
pub(crate) fn filter_icon_names<'a>(names: &'a [String], query: &str) -> Vec<&'a str> {
    let q = query.trim().to_ascii_lowercase();
    let mut hits: Vec<&'a str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| q.is_empty() || n.to_ascii_lowercase().contains(&q))
        .collect();
    hits.sort_unstable();
    hits.truncate(ICON_GRID_CAP);
    hits
}

/// Number of names the query matches, uncapped (the grid's caption).
pub(crate) fn count_icon_matches(names: &[String], query: &str) -> usize {
    let q = query.trim().to_ascii_lowercase();
    names
        .iter()
        .filter(|n| q.is_empty() || n.to_ascii_lowercase().contains(&q))
        .count()
}

/// One browser cell: the icon at the target sensor's row size over its
/// name; pressing writes the name into the sensor's field and closes
/// the modal.
pub(crate) fn icon_grid_cell(
    pal: Palette,
    cfg: &PiumaConfig,
    size: u16,
    name: &str,
    section_index: usize,
    sensor_index: usize,
) -> Element<'static, Message> {
    // The name is copied into the widgets, so the cell keeps no borrow:
    // it can outlive the filtered list that produced it.
    let preview: Element<'static, Message> =
        resolved_icon(cfg, size, Some(name)).unwrap_or_else(|| text("✕").size(BODY).into());
    button(
        column![preview, text(name.to_string()).size(9)]
            .spacing(4)
            .align_x(Alignment::Center)
            .width(Length::Fill),
    )
    .on_press(Message::IconPicked(section_index, sensor_index, name.to_string()))
    .padding(6)
    .width(Length::Fixed(96.0))
    .style(move |t, s| btn_secondary(pal, t, s))
    .into()
}

/// Scrollable holding the tab content: the add-section/add-sensor tasks
/// share its id to snap the freshly created row into view.
pub(crate) fn content_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("content")
}

/// Text-input id of one sensor's name field, focused after add/duplicate.
pub(crate) fn sensor_name_id(section: usize, sensor: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("sensor-name-{section}-{sensor}"))
}

/// Widget id of one sensor's card: the mini-bar scrolls to it.
pub(crate) fn sensor_card_id(section: usize, sensor: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("sensor-card-{section}-{sensor}"))
}

/// Scroll offset (content px) that puts a sensor card near the top of the
/// content scrollable: `sensor_y` and `scroll_top` are window coords, so
/// the current translation turns them back into content coords.
pub(crate) fn scroll_target(scroll_top: f32, translation: f32, sensor_y: f32) -> f32 {
    (sensor_y - scroll_top + translation - 8.0).max(0.0)
}

/// Two-column form grid: fields alternate left/right, one per cell, so long
/// forms stay compact and their labels line up in fixed columns.
pub(crate) fn form_grid<'a>(fields: Vec<(&'a str, Element<'a, Message>)>) -> Element<'a, Message> {
    let mut rows = Column::new().spacing(8).width(Length::Fill);
    let mut iter = fields.into_iter();
    while let Some(first) = iter.next() {
        let mut row = Row::new().spacing(20).width(Length::Fill)
            .push(form_cell(first.0, first.1));
        if let Some(second) = iter.next() {
            row = row.push(form_cell(second.0, second.1));
        }
        rows = rows.push(row);
    }
    rows.into()
}

/// Stacks the red reason under a field whose config value is invalid.
///
/// The widget shape must be identical with and without a reason: iced
/// replaces a whole subtree when a child's type (tag) changes (`Tree::diff`),
/// and that rebuild destroys the focused text input's state — so the reason
/// appearing/disappearing mid-typing used to steal the focus. The reason line
/// therefore stays always in the tree, collapsed to zero height while valid.
pub(crate) fn invalid_under<'a>(field: Element<'a, Message>, reason: Option<&'a str>) -> Element<'a, Message> {
    let reason_line = text(reason.unwrap_or(""))
        .size(10)
        .height(if reason.is_some() { Length::Shrink } else { Length::Fixed(0.0) })
        .style(error_text_style);
    Column::new()
        .push(field)
        .push(reason_line)
        .spacing(2)
        .width(Length::Fill)
        .into()
}

/// Checkerboard + colour swatch drawn on a canvas, so transparency reads
/// as transparency (the grey squares) instead of a wrong solid colour.
struct Swatch {
    color: Color,
    invalid: bool,
}

impl canvas::Program<Message> for Swatch {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let cell = 6.0;
        let light = Color::from_rgb(0.58, 0.58, 0.58);
        let dark = Color::from_rgb(0.34, 0.34, 0.34);
        let cols = (bounds.width / cell).ceil() as i32;
        let rows = (bounds.height / cell).ceil() as i32;
        for row in 0..rows {
            for col in 0..cols {
                let color = if (row + col) % 2 == 0 { light } else { dark };
                frame.fill_rectangle(
                    Point::new(col as f32 * cell, row as f32 * cell),
                    Size::new(cell, cell),
                    color,
                );
            }
        }
        // The colour over the board: alpha now blends visibly.
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), self.color);
        if self.invalid {
            frame.stroke_rectangle(
                Point::ORIGIN,
                bounds.size(),
                canvas::Stroke::default()
                    .with_color(ERROR_RED)
                    .with_width(2.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// A 28×28 alpha-aware swatch: the visible colour of a field, with a red
/// ring when the typed value does not parse.
pub(crate) fn swatch(color: Color, invalid: bool) -> Element<'static, Message> {
    canvas(Swatch { color, invalid })
        .width(28)
        .height(28)
        .into()
}

/// Colour input filling its cell, without override controls (see
/// [`color_control_over`]).
pub(crate) fn color_control<'a>(
    value: &'a str,
    invalid: Option<&'a str>,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    color_control_over(value, invalid, None, on_input, None)
}

/// Colour input filling its cell: text field, a large alpha-aware swatch
/// and the colour's alpha percentage. `inherited` fills the placeholder
/// when the field is empty (the value the bar would use) and `on_reset`
/// adds the × that drops the override, so the inheritance is visible
/// instead of being an invisible "empty means default".
pub(crate) fn color_control_over<'a>(
    value: &'a str,
    invalid: Option<&'a str>,
    inherited: Option<&'a str>,
    on_input: impl Fn(String) -> Message + 'a,
    on_reset: Option<Message>,
) -> Element<'a, Message> {
    let error_color = ERROR_RED;
    let parsed = config::parse_color(value).ok();
    let swatch_color = parsed.unwrap_or_else(|| hex_color(value));
    let alpha = parsed.map(|c| c.a).unwrap_or(1.0);
    let placeholder = inherited
        .filter(|i| !i.trim().is_empty())
        .unwrap_or("#rrggbb");
    let field = input(placeholder, value, on_input)
        .width(Length::Fill)
        .style(move |t: &Theme, status| {
            let mut style = text_input_style(t, status);
            if invalid.is_some() {
                style.border = Border { width: 1.0, color: error_color, radius: 3.0.into() };
            }
            style
        });
    let mut row = Row::new()
        .push(field)
        .push(swatch(swatch_color, invalid.is_some()));
    if alpha < 0.999 {
        row = row.push(text(format!("{:.0}%", alpha * 100.0)).size(10));
    }
    if value.trim().is_empty() {
        row = row.push(text("inherited").size(10).style(|_t: &Theme| TextStyle {
            color: Some(Color::from_rgb(0.55, 0.55, 0.58)),
            ..TextStyle::default()
        }));
    } else if let Some(reset) = on_reset {
        row = row.push(
            button(text("×").size(BODY))
                .on_press(reset)
                .padding(4)
                .style(button::secondary),
        );
    }
    invalid_under(
        row.spacing(6)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into(),
        invalid,
    )
}

/// Slider control without override controls (see [`slider_control_over`]).
pub(crate) fn slider_control<'a>(
    range: RangeInclusive<i32>,
    value: i32,
    invalid: Option<&'a str>,
    on_input: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    slider_control_over(range, value, invalid, None, on_input, None)
}

/// Slider control filling its cell: slider takes the space, value on the
/// right (style numbers: -1 = disabled, 0..=N = enabled). With `inherited`
/// a disabled field shows the value it would fall back to; `on_reset`
/// adds the × that removes an enabled override.
pub(crate) fn slider_control_over<'a>(
    range: RangeInclusive<i32>,
    value: i32,
    invalid: Option<&'a str>,
    inherited: Option<i32>,
    on_input: impl Fn(i32) -> Message + 'a,
    on_reset: Option<Message>,
) -> Element<'a, Message> {
    let label = if value < 0 {
        inherited.map_or_else(|| "-1".to_string(), |n| format!("auto {n}"))
    } else {
        value.to_string()
    };
    let value_text = text(label)
        .size(BODY)
        .width(56)
        .style(move |_t: &Theme| TextStyle {
            color: invalid.map(|_| ERROR_RED),
            ..TextStyle::default()
        });
    let mut row = Row::new()
        .push(Slider::new(range, value, on_input).width(Length::Fill))
        .push(value_text);
    if value >= 0 {
        if let Some(reset) = on_reset {
            row = row.push(
                button(text("×").size(BODY))
                    .on_press(reset)
                    .padding(4)
                    .style(button::secondary),
            );
        }
    }
    invalid_under(
        row.spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into(),
        invalid,
    )
}

/// Collapsible header: chevron + title + summary; pressing toggles the block.
/// The title is owned so section/sensor rows can pass their dynamic names.
pub(crate) fn collapsible_header<'a>(
    pal: Palette,
    title: String,
    summary: String,
    expanded: bool,
    on_toggle: Message,
) -> Element<'a, Message> {
    let summary = text(summary).size(10).style(muted_text(pal)).into();
    collapsible_header_el(pal, title, summary, expanded, on_toggle)
}

/// Same header with a widget summary (the coloured override chips).
pub(crate) fn collapsible_header_el<'a>(
    pal: Palette,
    title: String,
    summary: Element<'a, Message>,
    expanded: bool,
    on_toggle: Message,
) -> Element<'a, Message> {
    button(
        Row::new()
            .push(text(if expanded { "▾" } else { "▸" }).size(BODY).width(14))
            .push(text(title).size(13))
            .push(summary)
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(6)
    .style(move |t, s| btn_secondary(pal, t, s))
    .on_press(on_toggle)
    .into()
}

/// Threshold rules in evaluation order: ascending `min` — the order the
/// bar reads them (styles.rs) and the order setup stores them.
pub(crate) fn sorted_rules(mut rules: Vec<config::ColorRule>) -> Vec<config::ColorRule> {
    rules.sort_by(|a, b| a.min.total_cmp(&b.min));
    rules
}

/// The border-type picklists' options, from the bar's own list.
pub(crate) fn border_type_options() -> Vec<String> {
    config::BORDER_TYPES.iter().map(|b| b.to_string()).collect()
}

// --- override chips and visual placement grids -------------------------

/// Coloured chips of the active overrides (a swatch per colour, the
/// number per style value) instead of a string summary; "inherited"
/// when nothing is overridden.
pub(crate) fn style_chips<'a>(
    pal: Palette,
    colors: &[(&'a str, &Option<String>)],
    numbers: &[(&'a str, &Option<i32>)],
) -> Element<'a, Message> {
    let mut row = Row::new().spacing(6).align_y(Alignment::Center);
    let mut any = false;
    for (name, value) in colors {
        let Some(value) = value else { continue };
        any = true;
        let color = hex_color(value);
        row = row.push(
            Row::new()
                .push(
                    container(text(""))
                        .width(10)
                        .height(10)
                        .style(move |_t: &Theme| container::Style {
                            background: Some(Background::Color(color)),
                            border: Border {
                                width: 1.0,
                                color: Color::from_rgb(0.4, 0.4, 0.4),
                                radius: 2.0.into(),
                            },
                            ..container::Style::default()
                        }),
                )
                .push(text((*name).to_string()).size(9).style(muted_text(pal)))
                .spacing(3)
                .align_y(Alignment::Center),
        );
    }
    for (name, value) in numbers {
        let Some(value) = value else { continue };
        any = true;
        row = row.push(
            text(format!("{name} {value}"))
                .size(9)
                .style(muted_text(pal)),
        );
    }
    if any {
        row.into()
    } else {
        text("inherited").size(10).style(muted_text(pal)).into()
    }
}

/// The nine cells of the anchor grid: the eight placements around the
/// screen edge, with `None` for the centre (the screen itself).
pub(crate) fn anchor_cells() -> [(&'static str, Option<&'static str>); 9] {
    [
        ("↖", Some("top-left")),
        ("↑", Some("top")),
        ("↗", Some("top-right")),
        ("←", Some("left")),
        ("▦", None),
        ("→", Some("right")),
        ("↙", Some("bottom-left")),
        ("↓", Some("bottom")),
        ("↘", Some("bottom-right")),
    ]
}

/// 3×3 screen grid placing the bar on an edge or corner by clicking.
pub(crate) fn anchor_grid<'a>(pal: Palette, current: &str) -> Element<'a, Message> {
    let mut grid = Column::new().spacing(4);
    for chunk in anchor_cells().chunks(3) {
        let mut row = Row::new().spacing(4);
        for (glyph, value) in chunk {
            let cell: Element<Message> = match value {
                Some(value) => {
                    let selected = current == *value;
                    button(text(*glyph).size(16))
                        .on_press(Message::AnchorChanged((*value).to_string()))
                        .width(Length::Fixed(34.0))
                        .height(Length::Fixed(28.0))
                        .style(move |t, s| {
                            if selected {
                                btn_primary(pal, t, s)
                            } else {
                                btn_secondary(pal, t, s)
                            }
                        })
                        .into()
                }
                None => container(text("▦").size(16))
                    .width(Length::Fixed(34.0))
                    .height(Length::Fixed(28.0))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into(),
            };
            row = row.push(cell);
        }
        grid = grid.push(row);
    }
    grid.width(Length::Shrink).into()
}

/// Alignment along the bar: four visual choices instead of a picklist.
pub(crate) fn align_row<'a>(pal: Palette, current: &str) -> Element<'a, Message> {
    let mut row = Row::new().spacing(4);
    for (glyph, value) in [("◧", "left"), ("▣", "center"), ("◨", "right"), ("↔", "fill")] {
        let selected = current == value;
        row = row.push(
            button(text(glyph).size(16))
                .on_press(Message::AlignChanged(value.to_string()))
                .width(Length::Fixed(34.0))
                .height(Length::Fixed(28.0))
                .style(move |t, s| {
                    if selected {
                        btn_primary(pal, t, s)
                    } else {
                        btn_secondary(pal, t, s)
                    }
                }),
        );
    }
    row.width(Length::Shrink).into()
}

/// The six popup positions on a two-row screen grid (row = top/bottom,
/// column = left/center/right).
pub(crate) fn notif_cells() -> [(&'static str, &'static str); 6] {
    [
        ("↖", "top-left"),
        ("↑", "top-center"),
        ("↗", "top-right"),
        ("↙", "bottom-left"),
        ("↓", "bottom-center"),
        ("↘", "bottom-right"),
    ]
}

/// Popup-placement grid with the extra `auto` cell.
pub(crate) fn notif_position_grid<'a>(pal: Palette, current: &str) -> Element<'a, Message> {
    let mut grid = Column::new().spacing(4);
    for chunk in notif_cells().chunks(3) {
        let mut row = Row::new().spacing(4);
        for (glyph, value) in chunk {
            let selected = current == *value;
            row = row.push(
                button(text(*glyph).size(16))
                    .on_press(Message::NotifPositionChanged((*value).to_string()))
                    .width(Length::Fixed(34.0))
                    .height(Length::Fixed(28.0))
                    .style(move |t, s| {
                        if selected {
                            btn_primary(pal, t, s)
                        } else {
                            btn_secondary(pal, t, s)
                        }
                    }),
            );
        }
        grid = grid.push(row);
    }
    let auto = current == "auto";
    Row::new()
        .push(grid)
        .push(
            button(text("Auto").size(BODY))
                .on_press(Message::NotifPositionChanged("auto".to_string()))
                .padding(6)
                .style(move |t, s| {
                    if auto {
                        btn_primary(pal, t, s)
                    } else {
                        btn_secondary(pal, t, s)
                    }
                }),
        )
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
}

// --- sensor parameter pick lists ---------------------------------------

/// Sentinel of a parameter pick list: clearing the key (the bar's own
/// default applies).
pub(crate) const DEFAULT_OPTION: &str = "(system default)";

/// Options of a parameter pick list: the sentinel, the current custom
/// value when the detected list does not carry it, then the detected
/// values — the selection never falls out of the list.
pub(crate) fn options_with_current(mut options: Vec<String>, current: &str) -> Vec<String> {
    if !current.is_empty() && !options.iter().any(|o| o == current) {
        options.insert(0, current.to_string());
    }
    options.insert(0, DEFAULT_OPTION.to_string());
    options
}

/// PulseAudio/PipeWire sink names from `pactl` (read once; an absent
/// `pactl` simply leaves the list empty and the text field usable).
pub(crate) fn audio_devices() -> Vec<String> {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            std::process::Command::new("pactl")
                .args(["list", "short", "sinks"])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| {
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        })
        .clone()
}

/// Monitor names from `hyprctl monitors -j` (read once); an absent or
/// unparsable answer leaves the list empty and the picklist keeps the
/// current value.
pub(crate) fn monitor_outputs() -> Vec<String> {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            std::process::Command::new("hyprctl")
                .args(["monitors", "-j"])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| {
                    serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout).ok()
                })
                .map(|monitors| {
                    monitors
                        .iter()
                        .filter_map(|monitor| {
                            monitor.get("name")?.as_str().map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .clone()
}

/// Network interfaces from `/sys/class/net`, sorted.
pub(crate) fn network_interfaces() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir("/sys/class/net")
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Clock format presets for the pick list next to the free-text field.
pub(crate) fn clock_formats() -> Vec<String> {
    ["%H:%M", "%H:%M:%S", "%a %d %b %H:%M", "%d/%m/%Y %H:%M", "%I:%M %p"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Calendar format presets.
pub(crate) fn calendar_formats() -> Vec<String> {
    ["%B %Y", "%A %d %B", "%d %b", "%d/%m/%Y"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// --- icon theme grid ----------------------------------------------------

/// Icon names previewed in each icon-theme cell.
pub(crate) const THEME_SAMPLES: [&str; 3] = ["folder", "utilities-terminal", "text-x-generic"];

/// One icon-theme cell: three sample icons resolved under that theme plus
/// the theme name; pressing it selects the theme (`None` = system).
pub(crate) fn theme_cell(
    pal: Palette,
    theme: Option<&str>,
    label: &str,
    selected: bool,
) -> Element<'static, Message> {
    let size = 20;
    let mut row = Row::new().spacing(4).align_y(Alignment::Center);
    for name in THEME_SAMPLES {
        let icon: Element<'static, Message> = theme
            .and_then(|theme| icons::resolve(theme, name, size))
            .map(|path| icons::widget(&path, f32::from(size)))
            .unwrap_or_else(|| text("·").size(BODY).into());
        row = row.push(icon);
    }
    button(
        column![
            row,
            text(label.to_string())
                .size(10)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::Word),
        ]
        .spacing(4)
        .width(Length::Fill),
    )
    .on_press(Message::IconThemeChanged(theme.unwrap_or("").to_string()))
    .padding(6)
    .width(Length::Fill)
    .style(move |t, s| {
        if selected {
            btn_primary(pal, t, s)
        } else {
            btn_secondary(pal, t, s)
        }
    })
    .into()
}
