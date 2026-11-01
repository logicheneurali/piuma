//! Wallpaper tab: the freedesktop folder scan feeding the image pick list
//! plus its card, and the autostart tab listing the launchers that start
//! with the session.
use iced::widget::image::Handle;
use iced::widget::{button, column, container, image, scrollable, text, toggler, Row};
use iced::{Alignment, Background, Border, Element, Length, Theme};
use std::fs;
use std::path::{Path, PathBuf};

use crate::controls::{color_control, input, picklist};
use crate::style::*;
use crate::model::{PiumaConfig, WallpaperCfg};
use crate::state::{Message, PiumaSetup};

/// Freedesktop wallpaper folders (base-dir + user-dirs specs): the
/// per-user data dir, every system data dir (`…/wallpapers`) and the
/// user's pictures directory. Missing folders are skipped by the scan.
pub(crate) fn wallpaper_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(base) = crate::xdg::data() {
        dirs.push(base.join("wallpapers"));
    }
    dirs.extend(
        crate::xdg::data_dirs()
            .into_iter()
            .map(|dir| dir.join("wallpapers")),
    );
    if let Some(pictures) = user_pictures_dir() {
        dirs.push(pictures);
    }
    dirs
}

/// Pictures folder from `~/.config/user-dirs.dirs`: the value is quoted
/// shell-ish text with `$HOME` placeholders, expanded here.
fn user_pictures_dir() -> Option<PathBuf> {
    let home = crate::xdg::home()?;
    let content = fs::read_to_string(home.join(".config/user-dirs.dirs")).ok()?;
    let raw = parse_user_dirs_pictures(&content)?;
    Some(PathBuf::from(raw.replace("$HOME", home.to_str()?)))
}

/// `XDG_PICTURES_DIR` value of `user-dirs.dirs` content; comments and
/// other keys are ignored, an empty value means "no pictures dir".
pub(crate) fn parse_user_dirs_pictures(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("XDG_PICTURES_DIR=")
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .filter(|v| !v.is_empty())
}

/// Every image under `dirs`, sorted and deduplicated: common raster
/// formats only (what the bar's image widget decodes).
pub(crate) fn scan_wallpaper_dirs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        collect_images(dir, &mut found, 6);
    }
    found.sort();
    found.dedup();
    found
}

/// Recursive worker of `scan_wallpaper_dirs`: unreadable entries are
/// skipped and `depth` caps the walk, so a symlink cycle can never hang
/// the setup window.
fn collect_images(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, out, depth - 1);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push(path);
        }
    }
}

/// Thumbnail size in pixels: the visible grid never needs more.
const THUMB_WIDTH: u32 = 160;
const THUMB_HEIGHT: u32 = 90;

/// `~/.cache/piuma/wallpapers`: downscaled previews live here, so a
/// later setup run lists every wallpaper without decoding it again.
fn thumbnail_cache_dir() -> Option<PathBuf> {
    crate::xdg::cache().map(|dir| dir.join("piuma/wallpapers"))
}

/// Cache file of one wallpaper: the key carries path, size and mtime, so
/// an edited image is decoded again instead of showing a stale preview.
fn cache_file(dir: &Path, path: &Path) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(meta) = path.metadata() {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            if let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) {
                elapsed.as_nanos().hash(&mut hasher);
            }
        }
    }
    dir.join(format!("{:016x}.png", hasher.finish()))
}

/// One downscaled preview: from the disk cache when fresh, decoded and
/// then cached otherwise. `None` for an unreadable image — the grid still
/// lists it by name, so the list stays complete even then.
pub(crate) fn thumbnail(path: &Path) -> Option<(String, Handle)> {
    thumbnail_cached(thumbnail_cache_dir().as_deref(), path)
}

/// Cache-aware worker of [`thumbnail`] (`None` cache = decode only, used
/// by the tests so they never touch the user's cache).
pub(crate) fn thumbnail_cached(cache: Option<&Path>, path: &Path) -> Option<(String, Handle)> {
    let label = path.to_string_lossy().into_owned();
    let cached = cache.map(|dir| cache_file(dir, path));
    if let Some(file) = &cached {
        if let Ok(bytes) = fs::read(file) {
            if !bytes.is_empty() {
                return Some((label, Handle::from_bytes(bytes)));
            }
        }
    }
    let decoded = ::image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let thumb = decoded.thumbnail(THUMB_WIDTH, THUMB_HEIGHT).to_rgba8();
    let mut bytes = Vec::new();
    ::image::DynamicImage::ImageRgba8(thumb)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            ::image::ImageFormat::Png,
        )
        .ok()?;
    if let (Some(dir), Some(file)) = (cache, cached) {
        let _ = fs::create_dir_all(dir);
        let _ = fs::write(file, &bytes);
    }
    Some((label, Handle::from_bytes(bytes)))
}

/// Applies one wallpaper edit and collapses a block the user backed out
/// of (disabled + empty path) back to `None`, so a save never injects
/// the key unless the section was really used.
pub(crate) fn wallpaper_set(config: &mut PiumaConfig, edit: impl FnOnce(&mut WallpaperCfg)) {
    let w = config.wallpaper.get_or_insert_with(WallpaperCfg::default);
    edit(w);
    if !w.enabled && w.path.is_empty() && w.color.is_empty() {
        config.wallpaper = None;
    }
}

/// One thumbnail cell of the wallpaper grid: the decoded cover preview
/// over the file name; pressing it selects that wallpaper.
fn wallpaper_cell(
    pal: Palette,
    path: &str,
    handle: Option<&Handle>,
    selected: bool,
) -> Element<'static, Message> {
    let label = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    // Every file is a cell from the first frame; a thumbnail that is
    // still decoding shows a neutral placeholder, never a missing row.
    let preview: Element<'static, Message> = match handle {
        Some(handle) => image(handle.clone())
            .width(Length::Fill)
            .height(Length::Fixed(84.0))
            .content_fit(iced::ContentFit::Cover)
            .into(),
        None => container(text("◌").size(20))
            .width(Length::Fill)
            .height(Length::Fixed(84.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(inset_style(pal))
            .into(),
    };
    button(
        column![
            preview,
            text(label)
                .size(10)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::Word),
        ]
        .spacing(4)
        .width(Length::Fill),
    )
    .on_press(Message::WallpaperPathChanged(path.to_string()))
    .padding(4)
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

/// The `panel.wallpaper` editor (roadmap2 #1): the bar draws the
/// background itself — an image (cover fit) or a solid colour. The
/// Hyprland switch below edits Hyprland's own config and applies
/// immediately; the piuma.json side follows the global Save.
pub(crate) fn wallpaper_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let cfg = state.config.wallpaper.clone().unwrap_or_default();
    let current = cfg.path.clone();
    let pal = palette(&state.config);
    let color_mode = cfg.mode == "color";
    // Borrow the field from the state, not the cloned `cfg`: the colour
    // control's element lives as long as `state`, not this stack frame.
    let color_value = state
        .config
        .wallpaper
        .as_ref()
        .map(|w| w.color.as_str())
        .unwrap_or_default();

    // The image preview uses the same Cover fit as the bar: it fills the
    // box and crops, so it never promises letterboxing that does not
    // happen. Colour mode simply shows the painted output.
    let preview: Element<Message> = if color_mode {
        match crate::config::parse_color(&cfg.color) {
            Ok(color) => container(text(""))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(Background::Color(color)),
                    ..container::Style::default()
                })
                .into(),
            Err(_) => text("No colour — type a valid value").size(BODY).into(),
        }
    } else if !current.is_empty() && PathBuf::from(&current).is_file() {
        iced::widget::image(iced::widget::image::Handle::from_path(current.clone()))
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(iced::ContentFit::Cover)
            .into()
    } else {
        text("No preview — pick an image or check the path").size(BODY).into()
    };

    let mode = picklist(
        vec!["Image".to_string(), "Solid colour".to_string()],
        Some(if color_mode { "Solid colour" } else { "Image" }.to_string()),
        |label: String| {
            Message::WallpaperModeChanged(if label == "Solid colour" {
                "color".to_string()
            } else {
                "image".to_string()
            })
        },
    )
    .width(Length::Fill);

    let mut content = column![
        text("Wallpaper").size(16).style(fg_text(pal)),
        toggler(cfg.enabled)
            .label("Enable wallpaper")
            .on_toggle(Message::WallpaperEnabled),
        Row::new()
            .push(text("Mode").size(BODY).width(Length::Fixed(80.0)))
            .push(mode)
            .align_y(Alignment::Center)
            .spacing(10)
            .width(Length::Fill),
    ]
    .spacing(10)
    .width(Length::Fill);

    if color_mode {
        content = content.push(
            Row::new()
                .push(text("Background colour").size(BODY).width(Length::Fixed(160.0)))
                .push(color_control(
                    color_value,
                    state.field_err("panel.wallpaper.color"),
                    Message::WallpaperColorChanged,
                ))
                .align_y(Alignment::Center)
                .spacing(10)
                .width(Length::Fill),
        );
        content = content.push(
            text("Solid colour fills each output; no image is decoded, so the bar palette stays as configured.")
                .size(BODY),
        );
        content = content.push(
            container(preview)
                .width(Length::Fill)
                .height(Length::Fixed(240.0))
                .style(move |_t: &Theme| container::Style {
                    border: Border {
                        color: pal.card_border,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..container::Style::default()
                }),
        );
    } else {
        // Runtime-only tones (see the save): the file keeps just this
        // switch, so turning it off restores the user's own colours.
        content = content.push(
            toggler(state.config.wallpaper_palette.unwrap_or(false))
                .label("Bar colours from the wallpaper")
                .on_toggle(Message::PaletteFromWallpaper),
        );
        // Every wallpaper is a cell from the first frame (name + cover
        // thumbnail when decoded): three columns, three rows visible and
        // the scrollbar for the rest. Thumbnails arrive in batches from
        // the background decode, so the grid fills in without hiding any
        // file and without blocking the window.
        let mut grid = column![].spacing(6).width(Length::Fill);
        let mut row = Row::new().spacing(6).width(Length::Fill);
        for (i, file) in state.wallpaper_files.iter().enumerate() {
            let path = file.to_string_lossy();
            let handle = state.wallpaper_thumbs.get(path.as_ref());
            row = row.push(wallpaper_cell(
                pal,
                &path,
                handle,
                current == path.as_ref(),
            ));
            if (i + 1) % 3 == 0 {
                grid = grid.push(row);
                row = Row::new().spacing(6).width(Length::Fill);
            }
        }
        if state.wallpaper_files.len() % 3 != 0 {
            grid = grid.push(row);
        }
        let grid: Element<Message> = if state.wallpaper_files.is_empty() {
            text("No wallpapers found in the freedesktop folders.")
                .size(BODY)
                .into()
        } else {
            scrollable(grid)
                .height(Length::Fixed(3.0 * 116.0))
                .width(Length::Fill)
                .into()
        };
        content = content.push(grid);
        if state.wallpaper_thumbs.len() < state.wallpaper_files.len() {
            content = content.push(
                text(format!(
                    "Previews: {} of {} decoded (the list above is complete and selectable).",
                    state.wallpaper_thumbs.len(),
                    state.wallpaper_files.len()
                ))
                .size(10)
                .style(muted_text(pal)),
            );
        }
        content = content.push(
            input(
                "/path/to/image.png",
                state.config.wallpaper.as_ref().map(|w| w.path.as_str()).unwrap_or_default(),
                Message::WallpaperPathChanged,
            )
            .width(Length::Fill),
        );
        content = content.push(
            text("Searched: $XDG_DATA_HOME/wallpapers, $XDG_DATA_DIRS/wallpapers and your pictures folder (freedesktop).\n\
                  Cover fills each output and crops the overhang when the image aspect differs; each monitor is covered separately.")
                .size(BODY),
        );
    }

    if let Some(disabled) = state.hypr_wallpaper_disabled {
        content = content.push(
            toggler(disabled)
                .label("Disable Hyprland wallpaper/logo (8K, saves memory)")
                .on_toggle(Message::HyprWallpaperToggled),
        );
        content = content.push(
            text("Applies immediately with hyprctl reload; edits only the wallpaper/logo options in the Hyprland config.")
                .size(BODY),
        );
    } else {
        content = content.push(
            text("Hyprland config not found (hyprland.lua / hyprland.conf).").size(BODY),
        );
    }

    card(pal, content).into()
}

/// The dedicated "Autostart apps" tab (roadmap2 #2): the global launch
/// switch, plus one row per freedesktop autostart `.desktop` with its
/// own switch — the selection lives in `panel.autostart_disabled` (the
/// Save persists it, both the bar and every save prune stale names)
/// and no autostart file is ever written, so other desktop environments
/// keep their own, unaffected view. The bar applies the selection at
/// its next start.
pub(crate) fn autostart_apps_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let pal = palette(&state.config);
    let mut apps = column![].spacing(4).width(Length::Fill);
    if state.autostart_apps.is_empty() {
        apps = apps.push(text("No autostart .desktop files found.").size(BODY));
    }
    for app in &state.autostart_apps {
        // A spec-off app (its own Hidden/GNOME switches keep it out) has
        // no piuma-side choice to make: its switch is inert.
        let toggle = if app.spec {
            toggler(app.enabled)
                .on_toggle(move |value| Message::AutostartAppToggled(app.file.clone(), value))
        } else {
            toggler(app.enabled)
        };
        apps = apps.push(
            Row::new()
                .push(text(&app.label).size(BODY).width(Length::Fill))
                .push(toggle)
                .align_y(Alignment::Center)
                .spacing(8)
                .width(Length::Fill),
        );
    }

    card(
        pal,
        column![
            text("Autostart apps").size(16).style(fg_text(pal)),
            toggler(state.config.autostart.unwrap_or(false))
                .label("Launch freedesktop autostart apps")
                .on_toggle(Message::AutostartChanged),
            apps,
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
}
