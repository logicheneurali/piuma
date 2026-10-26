//! The wallpaper surface: sync between config and runtime (plan,
//! view, palette extraction kick-off).

use std::path::{Path, PathBuf};

use crate::config::{self, WallpaperConfig};
use crate::palette;
use crate::{monitor_names, restyle};
use crate::{Message, Panel};
use iced::widget::image::Image;
use iced::widget::{container, text};
use iced::{Background, Element, Length, Task as Command};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;

/// The background the active mode selects. The mode is explicit: an
/// image path is ignored in colour mode and a colour is ignored in image
/// mode, so the setup switch always means what it says.
enum WallpaperBackground<'a> {
    Image(&'a str),
    Color(iced::Color),
}

fn background(w: &WallpaperConfig) -> Option<WallpaperBackground<'_>> {
    match w.mode.as_str() {
        "image" => {
            let path = w.path.trim();
            (!path.is_empty() && Path::new(path).is_file())
                .then_some(WallpaperBackground::Image(path))
        }
        "color" => config::parse_color(&w.color).ok().map(WallpaperBackground::Color),
        _ => None,
    }
}

fn blank<'a>() -> Element<'a, Message> {
    container(text("")).width(Length::Fill).height(Length::Fill).into()
}

/// Open the wallpaper surface (roadmap2 #1) so it matches the config and
/// hide it when the feature is off. The window stays alive while off: a
/// closed wallpaper window makes the renderer/driver retain the decoded
/// image (~8 MB per output), so only the view goes empty. Called on
/// every tick — the comparison is cheap, and the first tick materializes
/// the setting after startup (the daemon's init closure cannot open
/// windows). One wallpaper surface per active output: a layer surface
/// lives on a single output, so the window set tracks the monitor list;
/// the output query only runs while the wallpaper is enabled, a monitor
/// plugged later joins within a tick, and a removed one closes its
/// surface (the only close left, for a surface whose output is gone).
pub(crate) fn sync_wallpaper(panel: &mut Panel) -> Command<Message> {
    let wanted = panel
        .config
        .panel
        .wallpaper
        .as_ref()
        .is_some_and(|w| w.enabled && background(w).is_some());
    if !wanted {
        // Closing a wallpaper window makes the renderer/driver keep the
        // decoded image (~8 MB per output). Keep the windows alive and
        // empty their view instead: re-enabling reuses the same texture.
        panel.wallpaper_hidden = panel.wallpaper.iter().map(|(_, id)| *id).collect();
        return Command::none();
    }
    panel.wallpaper_hidden.clear();
    let desired = monitor_names();
    let (to_open, to_close) = wallpaper_plan(&panel.wallpaper, &desired);
    // Only an output that disappeared is closed now; the feature being
    // off keeps its windows hidden (see above).
    forget_closed(&mut panel.wallpaper, &to_close);
    let mut cmds: Vec<Command<Message>> = to_close.into_iter().map(iced::window::close).collect();
    for name in to_open {
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            // `FILL` = both axes to the compositor: the surface covers
            // this output whatever its size and scale (all four anchors
            // are what `FILL` requires).
            size: LayerSize::FILL,
            layer: Layer::Background,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            // -1 = ignore other surfaces' reservations: the bar's strip
            // must not inset the background (full output, exactly what
            // a wallpaper tool does).
            exclusive_zone: Some(-1),
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::OutputName(name.clone()),
            // Clicks fall through to the bare desktop: the wallpaper
            // never eats input meant for the compositor.
            events_transparent: true,
            namespace: Some("piuma-wallpaper".to_string()),
            ..Default::default()
        });
        panel.wallpaper.push((name, id));
        cmds.push(open);
    }
    Command::batch(cmds)
}

/// The derived palette rides the tick (see `sync_wallpaper`): the
/// extraction re-runs only when the wallpaper file's path or mtime
/// change — or the option just turned on — never every second. With the
/// option (or the wallpaper) off the in-memory palette is dropped, so
/// the next rebuild resolves the user's own configured colours again:
/// nothing is ever written back to the config file.
pub(crate) fn sync_palette(panel: &mut Panel) -> Command<Message> {
    let cfg = &panel.config.panel;
    let wanted = cfg
        .wallpaper_palette
        .unwrap_or(false)
        .then_some(cfg.wallpaper.as_ref())
        .flatten()
        .filter(|w| w.enabled)
        // The palette is extracted from an image only; colour mode drops
        // it and the configured app colours are resolved again.
        .filter(|w| matches!(background(w), Some(WallpaperBackground::Image(_))))
        .and_then(|w| {
            let mtime = std::fs::metadata(&w.path)
                .ok()
                .and_then(|m| m.modified().ok())?;
            PathBuf::from(&w.path)
                .is_file()
                .then_some((w.path.clone(), mtime))
        });
    let Some(key) = wanted else {
        // Dropping a palette that was live re-resolves the chains
        // back to the configured colours (see `restyle`); an already
        // empty palette needs no work.
        if panel.palette.is_some() {
            panel.palette = None;
            panel.palette_key = None;
            restyle(panel);
            // The app themes derive from the same tones: re-render them
            // with the configured colours, or they would keep the
            // wallpaper palette after it was switched off.
            if let Some(path) = crate::CONFIG_PATH.get() {
                crate::app_theme::sync(std::path::Path::new(path), &panel.config.panel, None);
            }
        }
        return Command::none();
    };
    if panel.palette_key.as_ref() == Some(&key) {
        return Command::none();
    }
    // The decode runs off the UI thread (a 4K image is tens of ms of
    // CPU); the key travels with the result, so a failed decode retries
    // only on the next file change, not every tick.
    let task_key = key.clone();
    Command::perform(
        async move {
            let path = task_key.0.clone();
            smol::unblock(move || palette::extract(std::path::Path::new(&path))).await
        },
        move |pal| Message::PaletteReady(pal, key),
    )
}

/// Pure part of the wallpaper sync: outputs that still need a window
/// and windows to close because their output is gone. The feature
/// toggle hides the windows instead of calling this with an empty list.
pub(crate) fn wallpaper_plan<T: Clone>(
    current: &[(String, T)],
    desired: &[String],
) -> (Vec<String>, Vec<T>) {
    let to_open = desired
        .iter()
        .filter(|name| !current.iter().any(|(n, _)| n == *name))
        .cloned()
        .collect();
    let to_close = current
        .iter()
        .filter(|(name, _)| !desired.contains(name))
        .map(|(_, id)| id.clone())
        .collect();
    (to_open, to_close)
}

/// Drop the closed surfaces from the list, so a later plan can reopen a
/// surface whose output comes back: an id kept here would match the
/// plan's "already placed" and never reopen.
pub(crate) fn forget_closed<T: PartialEq>(current: &mut Vec<(String, T)>, closed: &[T]) {
    current.retain(|(_, id)| !closed.contains(id));
}

/// The wallpaper surface: the configured image covering the whole output
/// (cover fit keeps the aspect ratio and crops what spills over). The
/// file is re-checked here — a path removed behind the config's back
/// renders the surface empty instead of breaking the window. Hidden
/// while the feature is off, so the window (and its image cache) lives.
pub(crate) fn wallpaper_view(panel: &Panel, window: iced::window::Id) -> Element<'_, Message> {
    // Hidden while the feature is off: keep the renderer (and its image
    // cache) alive, draw nothing.
    let Some(w) = panel.config.panel.wallpaper.as_ref() else {
        return blank();
    };
    if panel.wallpaper_hidden.contains(&window) {
        return blank();
    }
    match background(w) {
        // Cover is the wallpaper fit: fill the output, keep the aspect
        // ratio and crop the overhang. Each output is covered on its own,
        // so a 16:9 and a 16:10 screen crop differently — by design.
        Some(WallpaperBackground::Image(path)) => {
            Image::new(iced::widget::image::Handle::from_path(path))
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Cover)
                .into()
        }
        Some(WallpaperBackground::Color(color)) => container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(color)),
                ..container::Style::default()
            })
            .into(),
        None => blank(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roadmap2 #1: the sync plan opens one window per wanted output,
    /// keeps the ones already placed there and closes a surface whose
    /// output disappeared. When the output comes back the plan reopens
    /// it — the closed ids are forgotten, so it no longer counts them as
    /// placed. (The feature toggle hides the windows instead, so this
    /// path is only for a lost output.)
    #[test]
    fn closed_surfaces_are_forgotten_and_reopen() {
        let mut current = vec![("HDMI-A-2".to_string(), 1u32)];
        let (open, close) = wallpaper_plan(&current, &[]);
        assert!(open.is_empty());
        forget_closed(&mut current, &close);
        assert!(current.is_empty(), "closed surface must leave the list");

        let (open, _) = wallpaper_plan(&current, &["HDMI-A-2".to_string()]);
        assert_eq!(open, vec!["HDMI-A-2"], "re-enabling reopens the surface");
    }

    #[test]
    fn wallpaper_plan_opens_keeps_and_closes() {
        let current = vec![("HDMI-A-2".to_string(), 1u32), ("GONE-1".to_string(), 2u32)];
        let desired = vec!["HDMI-A-2".to_string(), "eDP-1".to_string()];
        let (open, close) = wallpaper_plan(&current, &desired);
        assert_eq!(open, vec!["eDP-1"]);
        assert_eq!(close, vec![2]);

        let (open, close) = wallpaper_plan(&current, &[]);
        assert!(open.is_empty());
        assert_eq!(close, vec![1, 2]);
    }

    /// The mode is explicit: image mode ignores the colour, colour mode
    /// ignores the path, and a missing file makes image mode empty.
    #[test]
    fn background_respects_the_mode() {
        let dir = std::env::temp_dir().join(format!("piuma-wall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let image = dir.join("image.png");
        std::fs::write(&image, b"not decoded here").expect("temp image");

        let mut w = WallpaperConfig {
            enabled: true,
            mode: "image".to_string(),
            path: "/definitely/missing.png".to_string(),
            color: "#112233".to_string(),
        };
        assert!(background(&w).is_none(), "image mode does not fall back to colour");

        w.mode = "color".to_string();
        assert!(matches!(background(&w), Some(WallpaperBackground::Color(_))));

        w.mode = "image".to_string();
        w.path = image.to_string_lossy().to_string();
        assert!(matches!(background(&w), Some(WallpaperBackground::Image(_))));

        // In colour mode the (valid) image path is ignored.
        w.mode = "color".to_string();
        assert!(matches!(background(&w), Some(WallpaperBackground::Color(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
