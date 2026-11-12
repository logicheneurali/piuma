//! Idle detection for the screensaver: `ext-idle-notify-v1` straight
//! from the compositor. No hypridle and no polling.
//!
//! On a v2 compositor the *input* notification is used: it tracks real
//! user input and ignores idle inhibitors. The inhibitor-aware variant
//! sounds better, but a single client pinning an inhibitor (some
//! browsers and media apps do it for as long as they run) would leave
//! the screensaver dead for the whole session; the fullscreen check in
//! the app is what keeps games and full-screen videos uncovered.

use std::os::fd::AsRawFd;

use iced::futures::channel::mpsc::Sender;
use iced::Subscription;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use crate::Message;

/// Which component a notification feeds: the screensaver (minutes) or
/// the window menu's idle hide (seconds).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Screensaver,
    WindowMenu,
}

/// One idle subscription, keyed on the timeout: a reload that changes
/// `minutes` drops the old stream (its thread sees the closed channel
/// within a second) and starts a new notification.
pub(crate) fn subscription(minutes: u32) -> Subscription<Message> {
    let timeout = minutes.clamp(1, 120) * 60_000;
    subscribe(timeout, Kind::Screensaver)
}

/// The window menu's threshold: `seconds` after the last input it
/// reports `WindowMenuIdle(true)`, and `false` on the next input.
pub(crate) fn menu_subscription(seconds: u32) -> Subscription<Message> {
    let timeout = seconds.clamp(10, 120) * 1000;
    subscribe(timeout, Kind::WindowMenu)
}

fn subscribe(timeout: u32, kind: Kind) -> Subscription<Message> {
    Subscription::run_with((timeout, kind), |&(timeout, kind)| {
        iced::stream::channel(4, move |sender| run(sender, timeout, kind))
    })
}

async fn run(sender: Sender<Message>, timeout: u32, kind: Kind) {
    smol::unblock(move || listen(sender, timeout, kind)).await;
}

/// The dispatch state: the notification's last event, forwarded by the
/// loop (the `Dispatch` callback only sees `&Sender`, and the bounded
/// channel needs `&mut` to send).
#[derive(Default)]
struct Idle {
    idled: bool,
    dirty: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Idle {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for Idle {
    fn event(
        _state: &mut Self,
        _proxy: &ExtIdleNotifierV1,
        _event: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Idle {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for Idle {
    /// `idled` → the overlay, `resumed` → any input, close it.
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.idled = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };
        state.dirty = true;
    }
}

/// Own the notification until the subscription is dropped. Wayland
/// dispatch runs on this thread: `poll` on the connection's fd keeps a
/// one-second cadence, enough to notice the closed channel (the app
/// stopping or disabling the feature) without a wake storm. A
/// compositor without the protocol leaves the feature off.
fn listen(mut sender: Sender<Message>, timeout: u32, kind: Kind) {
    let Ok(conn) = Connection::connect_to_env() else {
        return;
    };
    let Ok((globals, mut queue)) = registry_queue_init::<Idle>(&conn) else {
        return;
    };
    let qh = queue.handle();
    let Ok(notifier) = globals.bind::<ExtIdleNotifierV1, _, _>(&qh, 1..=2, ()) else {
        eprintln!("[piuma] screensaver: ext-idle-notify missing on this compositor");
        return;
    };
    let Ok(seat) = globals.bind::<WlSeat, _, _>(&qh, 1..=9, ()) else {
        return;
    };
    // v2 adds the input-only variant; on v1 there is only the
    // inhibitor-aware one (see the module comment).
    let _notification = if notifier.version() >= 2 {
        notifier.get_input_idle_notification(timeout, &seat, &qh, ())
    } else {
        notifier.get_idle_notification(timeout, &seat, &qh, ())
    };
    let _ = conn.flush();

    let fd = conn.backend().poll_fd().as_raw_fd();
    let mut state = Idle::default();
    loop {
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the call's duration.
        let ready = unsafe { libc::poll(&mut pollfd, 1, 1000) };
        if ready < 0 {
            return; // the connection is gone: nothing left to watch
        }
        if ready > 0 {
            if let Some(guard) = queue.prepare_read() {
                let _ = guard.read();
            }
            let _ = queue.dispatch_pending(&mut state);
        }
        if state.dirty {
            state.dirty = false;
            let message = match kind {
                Kind::Screensaver => Message::Idle(state.idled),
                Kind::WindowMenu => Message::WindowMenuIdle(state.idled),
            };
            if sender.try_send(message).is_err() {
                return;
            }
        }
        // The app dropped the subscription (feature off or app exit).
        if sender.is_closed() {
            return;
        }
    }
}
