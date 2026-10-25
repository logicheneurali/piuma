//! The wallpaper surface: sync between config and runtime (plan,
//! view, palette extraction kick-off).

use std::path::PathBuf;

use crate::palette;
use crate::{monitor_names, restyle};
use crate::{Message, Panel};
use iced::widget::image::Image;
use iced::widget::{container, text};
use iced::{Element, Length, Task as Command};
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::LayerSize;
/// Open or close the wallpaper surface (roadmap2 #1) so it matches the
/// config: the window exists while `panel.wallpaper` is enabled with an
/// existing file and closes otherwise. Called on every tick — the
/// comparison is cheap, and the first tick materializes the setting
/// after startup (the daemon's init closure cannot open windows).
/// One wallpaper surface per active output: a layer surface lives on a
/// single output, so the window set tracks the monitor list. Runs on
/// every tick — the output query only executes while something must be
/// shown (the disabled case costs nothing), a monitor plugged later
/// joins within a tick, and a removed one closes its surface.
pub(crate) fn sync_wallpaper(panel: &mut Panel) -> Command<Message> {
    let wanted = panel
        .config
        .panel
        .wallpaper
        .as_ref()
        .is_some_and(|w| w.enabled && !w.path.is_empty() && PathBuf::from(&w.path).is_file());
    let desired = if wanted { monitor_names() } else { Vec::new() };
    let (to_open, to_close) = wallpaper_plan(&panel.wallpaper, &desired);
    // Forget the closed surfaces in the same breath (as the corner masks
    // do): keeping the id would make the plan believe the surface is
    // still placed, so toggling the wallpaper off and on again would
    // never reopen it.
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
        .filter(|w| !w.path.is_empty())
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
/// and windows to close (their output is gone, or the whole feature
/// switched off — an empty wanted list then closes everything).
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
/// surface whose output (or whole feature) comes back: an id kept here
/// would match the plan's "already placed" and never reopen.
pub(crate) fn forget_closed<T: PartialEq>(current: &mut Vec<(String, T)>, closed: &[T]) {
    current.retain(|(_, id)| !closed.contains(id));
}

/// The wallpaper surface: the configured image covering the whole output
/// (cover fit keeps the aspect ratio and crops what spills over). The
/// file is re-checked here — a path removed behind the config's back
/// renders the surface empty instead of breaking the window.
pub(crate) fn wallpaper_view(panel: &Panel) -> Element<'_, Message> {
    let path = panel
        .config
        .panel
        .wallpaper
        .as_ref()
        .map(|w| w.path.as_str())
        .unwrap_or_default();
    if path.is_empty() || !PathBuf::from(path).is_file() {
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    Image::new(iced::widget::image::Handle::from_path(path))
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(iced::ContentFit::Cover)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roadmap2 #1: the sync plan opens one window per wanted output,
    /// keeps the ones already placed there and closes the rest — an
    /// empty wanted list (feature off) closes everything.
    /// Toggling the wallpaper (or corners) off then on must reopen the
    /// surfaces: the closed ids are forgotten, so the plan no longer
    /// counts them as placed.
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
}
