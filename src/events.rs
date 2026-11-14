//! The subscriptions the daemon runs: the tick, the raw iced events and
//! the per-feature streams (tray, notifications, effects, menus, popups,
//! screensaver).

use iced::{event, time, Subscription};

use crate::{config, effects, idle, notifications, popup_anim, screensaver, tray, window_menu, App, Message};

pub(crate) fn subscription(app: &App) -> Subscription<Message> {
    let panel = &app.state;
    // The tray's D-Bus side runs only while a tray sensor is configured;
    // dropping it releases the watcher/host bus names automatically.
    let tray_sub = panel
        .config
        .panel
        .sections
        .iter()
        .flat_map(|section| section.sensors.iter())
        .any(|s| matches!(s.source, config::SensorSource::Tray {}))
        .then(tray::subscription);
    // The notifications D-Bus server runs only while the block is present
    // (its subscription claims the bus name at startup, before any client
    // can activate the name's activator); dropping it releases the name.
    // `configure` keeps the live settings in step on every (re)build.
    let notif_sub = panel
        .config
        .panel
        .notifications
        .as_ref()
        .map(|cfg| {
            notifications::configure(cfg);
            notifications::subscription()
        });
    // Animation frames exist only while an effect is alive: the idle
    // bar never pays for a 30 fps subscription.
    let effect_sub = (!panel.value_effects.is_empty())
        .then(|| time::every(effects::FRAME).map(|_| Message::EffectFrame));
    // The floating window menu's event thread runs only while the tweak
    // is on; its glide frames only while a move is in progress.
    let menu_sub = window_menu::enabled(&panel.config).then(window_menu::subscription);
    // The idle notification exists only while "Hide when idle" is on:
    // its timeout is the configured threshold.
    let menu_idle = window_menu::idle_timeout(&panel.config).map(idle::menu_subscription);
    let menu_frames = panel
        .window_menu
        .as_ref()
        .is_some_and(window_menu::WindowMenu::animating)
        .then(|| time::every(window_menu::FRAME).map(|_| Message::WindowMenuFrame));
    // The cursor popups' opening slide: frames only while one is alive.
    let popup_frames = popup_anim::active()
        .then(|| time::every(popup_anim::FRAME).map(|_| Message::PopupFrame));
    // The screensaver: its idle notification exists only while the
    // tweak is on (disabled = no thread, no wake), its frames only while
    // the overlay is up, its SIGUSR1 trigger only when it can be shown.
    let saver_minutes = panel.config.screensaver();
    let saver_idle = saver_minutes.map(idle::subscription);
    // The SIGUSR1 handler is installed even when the tweak is off: the
    // default disposition of that signal would terminate the bar (the
    // setup's button and a stray `kill -USR1` must never do that).
    let saver_signal = Some(screensaver::signal_subscription());
    // The runtime's surface events: the session-lock windows it creates
    // for a lock request (see `screensaver::on_shell`).
    let saver_shell = Some(panel.shell.listen().map(Message::Shell));
    let saver_frames = panel
        .screensaver_on
        .then(|| time::every(screensaver::FRAME).map(|_| Message::ScreensaverFrame));
    Subscription::batch(
        [
            time::every(std::time::Duration::from_secs(1)).map(|_| Message::Tick),
            event::listen_with(|event, _status, id| Some(Message::IcedEvent(id, event))),
        ]
        .into_iter()
        .chain(tray_sub)
        .chain(notif_sub)
        .chain(effect_sub)
        .chain(menu_sub)
        .chain(menu_idle)
        .chain(menu_frames)
        .chain(popup_frames)
        .chain(saver_idle)
        .chain(saver_signal)
        .chain(saver_shell)
        .chain(saver_frames),
    )
}


