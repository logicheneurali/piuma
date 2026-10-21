//! Form controls (inputs, sliders, pick lists, collapsible cards) and the
//! icon-theme discovery feeding the sensor icon pickers.
use iced::widget::text::Style as TextStyle;
use iced::widget::{button, column, container, text, PickList, Row, Slider, TextInput, Button, Column};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::config;
use crate::icons;
use crate::style::{
    btn_secondary, error_text_style, hex_color, inset_style, muted_text, palette, Palette, BODY,
    ERROR_RED, LABEL_WIDTH,
};
use crate::model::{PiumaConfig, Section, Sensor};
use crate::state::{Message, PiumaSetup};


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
    TextInput::new(placeholder, value).on_input(on_input).size(BODY)
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

/// The panel's icon-theme picker; the empty config maps to the sentinel
/// and back (empty = the bar's system fallback).
pub(crate) fn icon_theme_picker(state: &PiumaSetup) -> Element<'_, Message> {
    let current = state.config.icon_theme.clone().unwrap_or_default();
    let selected = if current.is_empty() {
        SYSTEM_ICON_THEME.to_string()
    } else {
        current
    };
    picklist(
        icon_theme_options(state.config.icon_theme.as_deref()),
        Some(selected),
        |value: String| {
            Message::IconThemeChanged(if value == SYSTEM_ICON_THEME { String::new() } else { value })
        },
    )
    .width(Length::Fill)
    .into()
}

/// The XDG data dirs of the lookup: the user's data home, then
/// $XDG_DATA_DIRS (defaulting to /usr/local/share and /usr/share).
pub(crate) fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut dirs_: Vec<PathBuf> = dirs::data_dir().into_iter().collect();
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    dirs_.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    dirs_
}

/// Theme roots: the XDG data dirs' `icons` plus the legacy ~/.icons.
pub(crate) fn icon_theme_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = xdg_data_dirs().iter().map(|d| d.join("icons")).collect();
    if let Some(home) = dirs::home_dir() {
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
pub(crate) fn invalid_under<'a>(field: Element<'a, Message>, reason: Option<&'a str>) -> Element<'a, Message> {
    match reason {
        None => field,
        Some(msg) => Column::new()
            .push(field)
            .push(
                text(msg).size(10).style(error_text_style),
            )
            .spacing(2)
            .width(Length::Fill)
            .into(),
    }
}

/// Colour input filling its cell: text field plus a live 16x16 preview
/// swatch. An invalid value (`config::validate_config`'s reason) turns
/// the field and swatch red and shows the reason under them.
pub(crate) fn color_control<'a>(
    value: &'a str,
    invalid: Option<&'a str>,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    let error_color = ERROR_RED;
    let swatch_color = hex_color(value);
    let swatch = container(text(""))
        .width(16)
        .height(16)
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(swatch_color)),
            border: Border {
                width: 1.0,
                color: if invalid.is_some() { error_color } else { Color::from_rgb(0.45, 0.45, 0.45) },
                radius: 3.0.into(),
            },
            ..container::Style::default()
        });
    let field = input("#rrggbb", value, on_input)
        .width(Length::Fill)
        .style(move |t: &Theme, status| {
            let mut style = iced::widget::text_input::default(t, status);
            if invalid.is_some() {
                style.border = Border { width: 1.0, color: error_color, radius: 3.0.into() };
            }
            style
        });
    invalid_under(
        Row::new()
            .push(field)
            .push(swatch)
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into(),
        invalid,
    )
}

/// Slider control filling its cell: slider takes the space, value on the
/// right (style numbers: -1 = disabled, 0..=N = enabled). An invalid
/// loaded value turns the number red with the reason under the row.
pub(crate) fn slider_control<'a>(
    range: RangeInclusive<i32>,
    value: i32,
    invalid: Option<&'a str>,
    on_input: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    let value_text = text(value.to_string())
        .size(BODY)
        .width(40)
        .style(move |_t: &Theme| TextStyle {
            color: invalid.map(|_| ERROR_RED),
            ..TextStyle::default()
        });
    invalid_under(
        Row::new()
            .push(Slider::new(range, value, on_input).width(Length::Fill))
            .push(value_text)
            .spacing(8)
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
    button(
        Row::new()
            .push(text(if expanded { "▾" } else { "▸" }).size(BODY).width(14))
            .push(text(title).size(13))
            .push(text(summary).size(10).style(muted_text(pal)))
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

/// One-line summary of the active style overrides for collapsed headers:
/// colors by field name, numbers with value, "none" when all is inherited.
pub(crate) fn style_summary(colors: &[(&str, &Option<String>)], numbers: &[(&str, &Option<i32>)]) -> String {
    let mut parts = Vec::new();
    let set_colors: Vec<&str> = colors.iter().filter(|(_, v)| v.is_some()).map(|(k, _)| *k).collect();
    if !set_colors.is_empty() {
        parts.push(set_colors.join(" "));
    }
    let set_numbers: Vec<String> = numbers.iter().filter_map(|(k, v)| v.map(|n| format!("{} {}", k, n))).collect();
    if !set_numbers.is_empty() {
        parts.push(set_numbers.join(" "));
    }
    if parts.is_empty() { "none".to_string() } else { parts.join(" · ") }
}

/// The border-type picklists' options, from the bar's own list.
pub(crate) fn border_type_options() -> Vec<String> {
    config::BORDER_TYPES.iter().map(|b| b.to_string()).collect()
}
