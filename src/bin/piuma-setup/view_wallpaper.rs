//! Wallpaper tab: the freedesktop folder scan feeding the image pick list
//! plus its card, and the autostart tab listing the launchers that start
//! with the session.
use iced::widget::{button, column, container, scrollable, text, toggler, Row};
use iced::{Alignment, Border, Element, Length, Theme};
use std::fs;
use std::path::{Path, PathBuf};

use crate::controls::input;
use crate::style::*;
use crate::model::{PiumaConfig, WallpaperCfg};
use crate::state::{Message, PiumaSetup};

/// Freedesktop wallpaper folders (base-dir + user-dirs specs): the
/// per-user data dir, every system data dir (`…/wallpapers`) and the
/// user's pictures directory. Missing folders are skipped by the scan.
pub(crate) fn wallpaper_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(base) = data_home {
        dirs.push(base.join("wallpapers"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(
            data_dirs
                .split(':')
                .filter(|d| !d.is_empty())
                .map(|d| PathBuf::from(d).join("wallpapers")),
        );
    }
    if let Some(pictures) = user_pictures_dir() {
        dirs.push(pictures);
    }
    dirs
}

/// Pictures folder from `~/.config/user-dirs.dirs`: the value is quoted
/// shell-ish text with `$HOME` placeholders, expanded here.
fn user_pictures_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
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

/// Applies one wallpaper edit and collapses a block the user backed out
/// of (disabled + empty path) back to `None`, so a save never injects
/// the key unless the section was really used.
pub(crate) fn wallpaper_set(config: &mut PiumaConfig, edit: impl FnOnce(&mut WallpaperCfg)) {
    let w = config.wallpaper.get_or_insert_with(WallpaperCfg::default);
    edit(w);
    if !w.enabled && w.path.is_empty() {
        config.wallpaper = None;
    }
}

/// The `panel.wallpaper` editor (roadmap2 #1): the bar draws the image
/// itself, so this tab only chooses it — enable toggle, the images found
/// in the freedesktop folders (list + preview) and a hand-typed path.
/// Saving is the global Save; the bar applies the change on its tick.
pub(crate) fn wallpaper_settings(state: &PiumaSetup) -> Element<'_, Message> {
    let cfg = state.config.wallpaper.clone().unwrap_or_default();
    let current = cfg.path.clone();
    let pal = palette(&state.config);

    // The scanned images as a pickable list; the current path is marked.
    let mut files = column![].spacing(2).width(Length::Fill);
    for file in &state.wallpaper_files {
        let label = file.to_string_lossy().to_string();
        let selected = current == label;
        files = files.push(
            button(
                text(if selected {
                    format!("• {label}")
                } else {
                    label.clone()
                })
                .size(BODY),
            )
            .on_press(Message::WallpaperPathChanged(label))
            .style(move |t, s| {
                if selected {
                    btn_primary(pal, t, s)
                } else {
                    btn_secondary(pal, t, s)
                }
            })
            .width(Length::Fill),
        );
    }
    let list = scrollable(files).height(Length::Fixed(240.0)).width(Length::Fill);

    // Preview of the current path: the image when the file is there, a
    // placeholder otherwise (a typed path may not exist yet).
    let preview: Element<Message> = if !current.is_empty() && PathBuf::from(&current).is_file() {
        iced::widget::image(iced::widget::image::Handle::from_path(current.clone()))
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(iced::ContentFit::Contain)
            .into()
    } else {
        text("No preview — pick an image or check the path").size(BODY).into()
    };

    card(
        pal,
        column![
            text("Wallpaper").size(16).style(fg_text(pal)),
            toggler(cfg.enabled)
                .label("Enable wallpaper")
                .on_toggle(Message::WallpaperEnabled),
            // Runtime-only tones (see the save): the file keeps just
            // this switch, so turning it off restores the user's own
            // configured colours.
            toggler(state.config.wallpaper_palette.unwrap_or(false))
                .label("Bar colours from the wallpaper")
                .on_toggle(Message::PaletteFromWallpaper),
            Row::new()
                .push(list)
                .push(
                    container(preview)
                        .width(Length::Fixed(360.0))
                        .height(Length::Fixed(240.0))
                        .style(move |_t: &Theme| container::Style {
                            border: Border {
                                color: pal.card_border,
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            ..container::Style::default()
                        }),
                )
                .spacing(10)
                .width(Length::Fill),
            input(
                "/path/to/image.png",
                state.config.wallpaper.as_ref().map(|w| w.path.as_str()).unwrap_or_default(),
                Message::WallpaperPathChanged,
            )
            .width(Length::Fill),
            text("Searched: $XDG_DATA_HOME/wallpapers, $XDG_DATA_DIRS/wallpapers and your pictures folder (freedesktop).")
                .size(BODY),
        ]
        .spacing(10)
        .width(Length::Fill),
    )
    .into()
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
