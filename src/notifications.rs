//! Freedesktop Desktop Notifications server — the D-Bus half of the
//! `notifications` sensor (and, from F3, of the popup card).
//!
//! Implements the core of the Desktop Notifications specification (the
//! four methods `Notify` / `CloseNotification` / `GetCapabilities` /
//! `GetServerInformation` plus the `NotificationClosed` and
//! `ActionInvoked` signals). piuma claims `org.freedesktop.Notifications`
//! with `DoNotQueue`: when another server already owns the name — or its
//! activator gets there first — piuma logs it and skips; it never queues
//! and never replaces a name (per the agreed v1 decisions).
//!
//! * every notification goes into a bounded history (`history_size`);
//!   expiry (reason 1), a user dismissal from the popup (reason 2) and
//!   `CloseNotification` (reason 3) only mark the entry closed and emit
//!   the signal — closed entries stay in the history, which is what the
//!   sensor scroll (F2) browses;
//! * `replaces_id` updates in place — same id, fresh deadline, active
//!   again — unless `replace_existing` is disabled;
//! * urgency 2 (critical) never auto-expires; `expire_timeout` `0` never
//!   expires, `-1` falls back to `default_timeout_ms`;
//! * summaries/bodies are markup-stripped (the spec allows filtering and
//!   v1 never claims the `body-markup` capability); sounds, `persistence`
//!   and `ActivationToken` are out of scope for v1.
//!
//! State lives in `HISTORY`; the bar reads a snapshot through
//! `display_value()` and is poked with `Message::NotificationsChanged`
//! (same pattern as the tray).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::Sender;
use iced::futures::stream::StreamExt;
use iced::Subscription;
use zbus::fdo::RequestNameFlags;
use zbus::object_server::SignalEmitter;
use zbus::proxy::{Builder, CacheProperties};
use zbus::Proxy;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::Message;

// ── constants ──────────────────────────────────────────────────────

const NOTIF_NAME: &str = "org.freedesktop.Notifications";
const NOTIF_PATH: &str = "/org/freedesktop/Notifications";
const NOTIF_IFACE: &str = "org.freedesktop.Notifications";

/// Spec reason codes (Table 8): 1 = expired, 2 = dismissed by the user,
/// 3 = closed by a `CloseNotification` call.
const REASON_EXPIRED: u32 = 1;
const REASON_DISMISSED: u32 = 2;
const REASON_CLOSED: u32 = 3;
/// Longest summary the sensor prints (bars must not stretch).
const MAX_SUMMARY: usize = 60;
/// Longest body line in the list popup (the sensor never shows one).
const MAX_BODY: usize = 120;

// ── settings & state ───────────────────────────────────────────────

/// Live server settings, taken from the `panel.notifications` block.
#[derive(Clone, Copy)]
pub struct Settings {
    /// Timeout used when an app passes `-1` (spec default: -1 → server default).
    pub default_timeout_ms: u64,
    /// Entries kept in the scrollable history.
    pub history_size: usize,
    /// Honour `replaces_id` (false: every call allocates a new id).
    pub replace_existing: bool,
}

const DEFAULTS: Settings = Settings {
    default_timeout_ms: 5_000,
    history_size: 20,
    replace_existing: true,
};

/// Upper bound for `panel.notifications.history_size`: every entry may
/// carry a full `image-data` payload, so an unbounded configured value
/// (the GUI accepts any number) would be an unbounded number of
/// megabytes in the bar.
const HISTORY_MAX: usize = 200;

static SETTINGS: RwLock<Settings> = RwLock::new(DEFAULTS);

/// Image payload from the spec's `image-data` hint, repacked at insert
/// to tight RGBA (row padding stripped, RGB expanded), so the popup can
/// draw it with `Handle::from_rgba` directly. Cards resolve priority
/// image-data > image-path > app_icon.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

/// One history entry: `summary` drives the sensor today, while the other
/// fields are the popup's payload (F2 actions / F3 card + image) — stored
/// now so the history is complete and the server never drops spec data.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    /// 0 low, 1 normal, 2 critical (2 never auto-expires).
    pub urgency: u8,
    pub image: Option<ImageData>,
    /// `image-path` hint, or `app_icon` when the hint is absent.
    pub icon: String,
    /// Flat key/label pairs; the `"default"` key is the primary action.
    pub actions: Vec<String>,
    pub received: Instant,
    /// Reason once closed (1/2/3); `None` while active.
    pub closed: Option<u32>,
    expires_at: Option<Instant>,
}

/// History in arrival order (back = newest), capped at `history_size`.
static HISTORY: RwLock<VecDeque<Notification>> = RwLock::new(VecDeque::new());
/// Popup image handles by notification id: `Handle::from_rgba` mints a
/// unique renderer id on every call, so building it inside `view` made
/// the bar re-decode and re-upload the same bytes on every frame. The
/// entry lives exactly as long as its card: `record` (replace),
/// `dismiss` and `prune_icon_handles` all part with it. Lock order is
/// HISTORY before ICONS, everywhere.
static ICONS: std::sync::LazyLock<RwLock<HashMap<u32, iced::widget::image::Handle>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));
/// Monotonic ids: spec allows any scheme, 1-based and never reused.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
/// Sensor scroll position: 0 = live (newest active entry), n > 0 = the
/// n-th newest received entry (closed ones included). Every arrival
/// resets it to 0, so the sensor follows what is new by default.
static SELECTED: AtomicU32 = AtomicU32::new(0);

/// Copy the `panel.notifications` block into the live settings; called
/// from the subscription gate every time the block is present (cheap and
/// idempotent, so hot reloads keep the values in step).
pub fn configure(cfg: &crate::config::NotificationsConfig) {
    let settings = Settings {
        default_timeout_ms: cfg
            .default_timeout_ms
            .unwrap_or(DEFAULTS.default_timeout_ms),
        history_size: cfg
            .history_size
            .map(|n| n as usize)
            .unwrap_or(DEFAULTS.history_size)
            .clamp(1, HISTORY_MAX),
        replace_existing: cfg.replace_existing.unwrap_or(DEFAULTS.replace_existing),
    };
    if let Ok(mut live) = SETTINGS.write() {
        *live = settings;
    }
}

// ── bar-facing snapshots ───────────────────────────────────────────

/// Text for the `notifications` sensor: the newest active notification's
/// summary, with the number of active ones appended when there are
/// several; empty while nothing is active (a closed notification only
/// comes back through the scroll, F2). Truncated to `MAX_SUMMARY`.
pub fn display_value() -> String {
    let Ok(hist) = HISTORY.read() else {
        return String::new();
    };
    // Browse mode (wheel moved off the live position): show that entry
    // with its position in the history; an evicted entry falls back live.
    let sel = SELECTED.load(Ordering::Relaxed);
    if sel > 0 {
        if let Some(n) = hist.iter().rev().nth(sel as usize - 1) {
            return format!("{} ({sel}/{})", truncate(&n.summary, MAX_SUMMARY), hist.len());
        }
    }
    let active = hist.iter().filter(|n| n.closed.is_none());
    let mut count = 0usize;
    let mut last: Option<&Notification> = None;
    for n in active {
        count += 1;
        last = Some(n);
    }
    let Some(newest) = last else {
        return String::new();
    };
    let mut out = truncate(&newest.summary, MAX_SUMMARY);
    if count > 1 {
        out.push_str(&format!(" ({count})"));
    }
    out
}

/// Wheel over the sensor: down walks towards the oldest entry, up back
/// towards live (list convention: newest on top).
pub fn scroll(up: bool) {
    if up {
        SELECTED
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |s| {
                Some(s.saturating_sub(1))
            })
            .ok();
    } else {
        // Cap at the history length: index n addresses rev().nth(n - 1).
        let cap = HISTORY.read().map(|h| h.len()).unwrap_or(0) as u32;
        SELECTED
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |s| {
                Some((s + 1).min(cap))
            })
            .ok();
    }
}

/// A history entry as the list popup renders it: the summary/body are
/// already truncated, so the UI measures and renders the strings as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifRow {
    pub id: u32,
    pub summary: String,
    pub body: String,
    /// A recorded close: the row renders dimmed.
    pub closed: bool,
}

/// Snapshot of the newest `max` entries, newest first, for the list
/// popup the notifications sensor opens; strings are cloned so the UI
/// never holds the lock.
pub fn history_rows(max: usize) -> Vec<NotifRow> {
    let Ok(hist) = HISTORY.read() else {
        return Vec::new();
    };
    hist.iter()
        .rev()
        .take(max)
        .map(|n| NotifRow {
            id: n.id,
            summary: truncate(&n.summary, MAX_SUMMARY),
            body: truncate(&n.body, MAX_BODY),
            closed: n.closed.is_some(),
        })
        .collect()
}

/// Click on a popup card: the same for the entry with `id`; a stale id
/// (evicted since the snapshot) is a no-op.
pub fn invoke_id(id: u32) {
    let n = HISTORY
        .read()
        .ok()
        .and_then(|hist| hist.iter().find(|n| n.id == id).cloned());
    if let Some(n) = n {
        invoke(&n);
    }
}

/// Emit `ActionInvoked(id, "default")` on the tray's shared blocking
/// connection in a background thread (UI never blocks on D-Bus).
fn invoke(n: &Notification) {
    if !n.actions.iter().any(|a| a == "default") {
        return;
    }
    let id = n.id;
    std::thread::spawn(move || {
        let Some(conn) = crate::tray::action_connection() else {
            eprintln!("[piuma] notifications: no session connection for ActionInvoked");
            return;
        };
        if let Err(e) = conn.emit_signal(
            None::<&str>,
            NOTIF_PATH,
            NOTIF_IFACE,
            "ActionInvoked",
            &(id, "default"),
        ) {
            eprintln!("[piuma] notifications: ActionInvoked failed: {e}");
        }
    });
}

/// User dismissal from the popup (✕ button or card click): mark closed
/// with the spec's reason 2 and report whether `NotificationClosed` must
/// fire (first close of a known id). The card's icon handle goes with
/// it (the tick-side prune would drop it a second later anyway).
pub fn dismiss(id: u32) -> bool {
    let changed = mark_closed(id, REASON_DISMISSED).1;
    if changed {
        if let Ok(mut icons) = ICONS.write() {
            icons.remove(&id);
        }
    }
    changed
}

/// Emit `NotificationClosed(id, 2)` for a dismissal taken outside the
/// D-Bus method (the UI path) — same background-thread pattern as
/// `invoke`.
pub fn spawn_closed_signal(id: u32) {
    std::thread::spawn(move || {
        let Some(conn) = crate::tray::action_connection() else {
            eprintln!("[piuma] notifications: no session connection for NotificationClosed");
            return;
        };
        if let Err(e) = conn.emit_signal(
            None::<&str>,
            NOTIF_PATH,
            NOTIF_IFACE,
            "NotificationClosed",
            &(id, REASON_DISMISSED),
        ) {
            eprintln!("[piuma] notifications: NotificationClosed failed: {e}");
        }
    });
}

// ── popup-facing snapshots (F3) ────────────────────────────────────

/// Icon a card can render: image-data (the handle comes from
/// `icon_handle`, cached per notification) or a path from
/// `image-path`/`app_icon` (a themed icon *name* simply yields no
/// icon).
pub enum CardIcon {
    /// The entry carries `image-data`; the view fetches the handle by
    /// notification id, so the payload itself is never cloned here.
    Image,
    Path(String),
}

/// One popup card: the fields the view draws, cloned out so the history
/// lock is never held across rendering.
pub struct Card {
    pub id: u32,
    pub summary: String,
    pub body: String,
    /// Priority per spec: image-data first, path as fallback.
    pub icon: Option<CardIcon>,
}

/// The active notifications, newest first, at most `max` (the stack
/// shown by the popup window).
pub fn active_cards(max: usize) -> Vec<Card> {
    let Ok(hist) = HISTORY.read() else {
        return Vec::new();
    };
    hist.iter()
        .rev()
        .filter(|n| n.closed.is_none())
        .take(max)
        .map(|n| Card {
            id: n.id,
            summary: n.summary.clone(),
            body: n.body.clone(),
            icon: n
                .image
                .as_ref()
                .map(|_| CardIcon::Image)
                .or_else(|| (!n.icon.is_empty()).then(|| CardIcon::Path(n.icon.clone()))),
        })
        .collect()
}

/// Number of active notifications (at most `max`), without cloning any
/// payload: the bar's tick only needs the count, and the old path
/// copied every image payload once a second just to drop it.
pub fn active_cards_len(max: usize) -> usize {
    let Ok(hist) = HISTORY.read() else {
        return 0;
    };
    hist.iter()
        .rev()
        .filter(|n| n.closed.is_none())
        .take(max)
        .count()
}

/// The popup's image handle for a card, built once per notification and
/// reused on every frame: the renderer id is stable, so the icon is
/// decoded and uploaded a single time instead of once per frame.
pub fn icon_handle(id: u32) -> Option<iced::widget::image::Handle> {
    if let Ok(icons) = ICONS.read() {
        if let Some(handle) = icons.get(&id) {
            return Some(handle.clone());
        }
    }
    let hist = HISTORY.read().ok()?;
    // Only a visible card may have a handle: a just-closed entry stays
    // in the history but must not resurrect its icon.
    let img = hist
        .iter()
        .find(|n| n.id == id && n.closed.is_none())?
        .image
        .as_ref()?;
    let handle = iced::widget::image::Handle::from_rgba(
        img.width as u32,
        img.height as u32,
        img.data.clone(),
    );
    drop(hist); // the cache write takes no history lock with it
    if let Ok(mut icons) = ICONS.write() {
        icons.insert(id, handle.clone());
    }
    Some(handle)
}

/// Drop the handles whose card left the visible stack (expiry and
/// history eviction leave through here); called from the bar's tick.
pub fn prune_icon_handles() {
    let Ok(hist) = HISTORY.read() else {
        return;
    };
    let Ok(mut icons) = ICONS.write() else {
        return;
    };
    if icons.is_empty() {
        return;
    }
    icons.retain(|id, _| hist.iter().any(|n| n.id == *id && n.closed.is_none()));
}

/// Cut at a char boundary and append `…`.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ── record / close logic (unit-tested core) ────────────────────────

/// Insert (or replace) a notification in the history; returns the id the
/// caller must report to the app. `replace` is honoured only when the
/// config allows it and the id is still known (closed ids may be
/// re-activated — the spec lets apps replace their own notifications).
fn record(replace: u32, n: Notification) -> u32 {
    let settings = SETTINGS
        .read()
        .map(|s| *s)
        .unwrap_or(DEFAULTS);
    // Anything new (or updated) makes the sensor follow it again.
    SELECTED.store(0, Ordering::Relaxed);
    let mut hist = HISTORY.write().unwrap_or_else(|e| e.into_inner());
    if settings.replace_existing && replace != 0 {
        if let Some(existing) = hist.iter_mut().find(|n| n.id == replace) {
            let id = existing.id;
            *existing = Notification { id, ..n };
            existing.closed = None;
            existing.received = Instant::now();
            // Same id, fresh pixels: the cached handle is stale.
            if let Ok(mut icons) = ICONS.write() {
                icons.remove(&id);
            }
            return id;
        }
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    hist.push_back(Notification { id, ..n });
    while hist.len() > settings.history_size {
        hist.pop_front();
    }
    id
}

/// Mark an active entry closed; returns `(known, changed)` — the signal
/// must only fire for the first close of a known id.
fn mark_closed(id: u32, reason: u32) -> (bool, bool) {
    let Ok(mut hist) = HISTORY.write() else {
        return (false, false);
    };
    match hist.iter_mut().find(|n| n.id == id) {
        Some(n) if n.closed.is_none() => {
            n.closed = Some(reason);
            n.expires_at = None;
            (true, true)
        }
        Some(_) => (true, false),
        None => (false, false),
    }
}

/// Expiry instant for a delivery: critical (2) and `0` never expire,
/// `-1` uses the configured default, anything else is the app's ms.
fn deadline(urgency: u8, expire_timeout: i32) -> Option<Instant> {
    if urgency == 2 || expire_timeout == 0 {
        return None;
    }
    let ms = if expire_timeout < 0 {
        SETTINGS
            .read()
            .map(|s| s.default_timeout_ms)
            .unwrap_or(DEFAULTS.default_timeout_ms)
    } else {
        expire_timeout as u64
    };
    Some(Instant::now() + Duration::from_millis(ms))
}

// ── hint parsing ───────────────────────────────────────────────────

/// Read one hint as a `Value`; `downcast` unwraps the variant wrapper the
/// bus puts around every `a{sv}` entry.
fn hint<'a>(hints: &'a HashMap<String, OwnedValue>, key: &str) -> Option<Value<'a>> {
    Value::try_from(hints.get(key)?).ok()
}

fn hint_u8(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<u8> {
    hint(hints, key)?.downcast::<u8>().ok()
}

fn hint_string(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    hint(hints, key)?.downcast::<String>().ok()
}

/// Spec `image-data` hint `(iiibiiay)` — seven fields (width, height,
/// rowstride, has_alpha, bits_per_sample, **channels**, data): 8-bit
/// samples over `rowstride * height` rows, repacked here to tight RGBA
/// (padding stripped per row, alpha added when the payload is RGB).
fn parse_image_data(hints: &HashMap<String, OwnedValue>) -> Option<ImageData> {
    let value = hint(hints, "image-data")?;
    let (width, height, rowstride, has_alpha, bits, channels, data) = value
        .downcast::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()
        .ok()?;
    // Spec: channels is 4 with alpha, 3 without; tolerate a lying
    // channels by falling back to has_alpha, but never read past 4.
    let channels = match channels {
        3 | 4 => channels,
        _ if has_alpha => 4,
        _ => 3,
    };
    let channels = i64::from(channels);
    let (width, height, rowstride) = (i64::from(width), i64::from(height), i64::from(rowstride));
    if width <= 0
        || height <= 0
        || i64::from(bits) != 8
        || rowstride < width * channels
        || (data.len() as i64) < rowstride * height
    {
        return None;
    }
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let row = &data[(y * rowstride) as usize..];
        for x in 0..width {
            let px = &row[(x * channels) as usize..];
            rgba.extend_from_slice(&px[..3]);
            rgba.push(if channels == 4 { px[3] } else { 255 });
        }
    }
    Some(ImageData {
        width: width as i32,
        height: height as i32,
        data: rgba,
    })
}

// ── D-Bus interface ────────────────────────────────────────────────

struct Notifications {
    sender: Sender<Message>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    /// Spec `Notify`: record the notification (in place when
    /// `replaces_id` is honoured) and return the id to report.
    async fn notify(
        &mut self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        let urgency = hint_u8(&hints, "urgency").unwrap_or(1).min(2);
        let image = parse_image_data(&hints);
        let icon = hint_string(&hints, "image-path")
            .filter(|p| !p.is_empty())
            .unwrap_or(app_icon);
        let n = Notification {
            id: 0,
            app: app_name,
            // The spec allows filtering markup and v1 never claims
            // `body-markup`, so strip tags defensively.
            summary: crate::tray::strip_markup(&summary),
            body: crate::tray::strip_markup(&body),
            urgency,
            image,
            icon,
            actions,
            received: Instant::now(),
            closed: None,
            expires_at: deadline(urgency, expire_timeout),
        };
        let id = record(replaces_id, n);
        let _ = self.sender.try_send(Message::NotificationsChanged);
        Ok(id)
    }

    /// Close a known notification (reason 3); unknown ids are an error.
    async fn close_notification(
        &mut self,
        #[zbus(connection)] conn: &Connection,
        id: u32,
    ) -> zbus::fdo::Result<()> {
        let (known, changed) = mark_closed(id, REASON_CLOSED);
        if !known {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "no notification with id {id}"
            )));
        }
        if changed {
            if let Ok(emitter) = SignalEmitter::new(conn, NOTIF_PATH) {
                let _ = Notifications::notification_closed(&emitter, id, REASON_CLOSED).await;
            }
            let _ = self.sender.try_send(Message::NotificationsChanged);
        }
        Ok(())
    }

    /// Exactly what v1 honours: plain body text and actions (the
    /// `"default"` key becomes the click action in F2/F3).
    async fn get_capabilities(&self) -> Vec<String> {
        vec!["body".to_string(), "actions".to_string()]
    }

    async fn get_server_information(&self) -> (String, String, String, String) {
        (
            "piuma".to_string(),
            "piuma".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
            "1.2".to_string(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action: &str,
    ) -> zbus::Result<()>;
}

// ── subscription (connection + expiry loop) ────────────────────────

pub fn subscription() -> Subscription<Message> {
    Subscription::run(|| iced::stream::channel(64, run))
}

/// Own the name, or watch the current owner and take over the moment
/// the name leaves the bus (the previous piuma, mid-replacement, still
/// holds it for the few milliseconds of its death — skipping for good,
/// as v1 did, left the notification server dead until the next bar
/// restart); a transport error retries after 5 s like the tray does.
async fn run(sender: Sender<Message>) {
    loop {
        match drive(&sender).await {
            // The watched owner left the bus: retry the claim right away.
            Ok(()) => {}
            Err(e) => {
                eprintln!("[piuma] notifications: D-Bus error: {e}; retrying in 5 s");
                crate::tray::delay(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn drive(sender: &Sender<Message>) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    // Serve before claiming the name: a call arriving the instant we own
    // it must find the object already registered.
    let iface = Notifications {
        sender: sender.clone(),
    };
    conn.object_server().at(NOTIF_PATH, iface).await?;
    match conn
        .request_name_with_flags(NOTIF_NAME, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(
            zbus::fdo::RequestNameReply::PrimaryOwner
            | zbus::fdo::RequestNameReply::AlreadyOwner,
        ) => {}
        // Another server owns it (often the previous piuma, mid-replacement):
        // watch for the name to leave the bus and let `run` retry; a
        // surviving owner is never fought.
        Err(zbus::Error::NameTaken) => {
            eprintln!("[piuma] notifications: {NOTIF_NAME} owned elsewhere — waiting");
            return watch_name(&conn).await;
        }
        Err(e) => return Err(e),
        Ok(_) => return Ok(()),
    }
    eprintln!("[piuma] notifications: server up ({NOTIF_NAME})");
    serve(&conn, sender).await
}

/// Resolves when the notification name's owner leaves the bus (or the
/// owner stream ends) — `run` then retries the claim.
async fn watch_name(conn: &Connection) -> zbus::Result<()> {
    let proxy = Builder::<Proxy>::new(conn)
        .destination(NOTIF_NAME)?
        .path(NOTIF_PATH)?
        .interface(NOTIF_IFACE)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut owner = proxy.receive_owner_changed().await?;
    loop {
        match owner.next().await {
            Some(Some(_)) => continue,
            Some(None) | None => return Ok(()),
        }
    }
}

/// Keep the nearest deadline armed: expire what is due (reason 1 +
/// signal + UI poke), then wait again. Never returns except on
/// connection failure (the caller retries).
async fn serve(conn: &Connection, sender: &Sender<Message>) -> zbus::Result<()> {
    let mut ui = sender.clone();
    loop {
        crate::tray::delay(next_wait()).await;
        let expired = expire_due();
        if expired.is_empty() {
            continue;
        }
        if let Ok(emitter) = SignalEmitter::new(conn, NOTIF_PATH) {
            for id in expired {
                let _ = Notifications::notification_closed(&emitter, id, REASON_EXPIRED).await;
            }
        }
        let _ = ui.try_send(Message::NotificationsChanged);
    }
}

/// Sleep until the nearest deadline, bounded so settings changes and
/// shutdown stay responsive (1 s idle poll when nothing expires).
fn next_wait() -> Duration {
    let now = Instant::now();
    let nearest = HISTORY
        .read()
        .ok()
        .and_then(|hist| {
            hist.iter()
                .filter(|n| n.closed.is_none())
                .filter_map(|n| n.expires_at)
                .map(|t| t.saturating_duration_since(now))
                .min()
        })
        .unwrap_or(Duration::from_secs(1));
    nearest.clamp(Duration::from_millis(100), Duration::from_secs(1))
}

/// Mark every due entry closed (reason 1) and return its ids.
fn expire_due() -> Vec<u32> {
    let now = Instant::now();
    let Ok(mut hist) = HISTORY.write() else {
        return Vec::new();
    };
    let mut expired = Vec::new();
    for n in hist.iter_mut() {
        if n.closed.is_none() && n.expires_at.is_some_and(|t| t <= now) {
            n.closed = Some(REASON_EXPIRED);
            n.expires_at = None;
            expired.push(n.id);
        }
    }
    expired
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// All tests share the statics: one at a time, like the tray's tests.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn fresh() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        HISTORY.write().unwrap().clear();
        ICONS.write().unwrap().clear();
        NEXT_ID.store(1, Ordering::Relaxed);
        SELECTED.store(0, Ordering::Relaxed);
        *SETTINGS.write().unwrap() = DEFAULTS;
        guard
    }

    fn sample(summary: &str, urgency: u8, expires_at: Option<Instant>) -> Notification {
        Notification {
            id: 0,
            app: "t".into(),
            summary: summary.into(),
            body: String::new(),
            urgency,
            image: None,
            icon: String::new(),
            actions: Vec::new(),
            received: Instant::now(),
            closed: None,
            expires_at,
        }
    }

    #[test]
    fn ids_are_monotonic_and_replace_updates_in_place() {
        let _g = fresh();
        let a = record(0, sample("a", 1, None));
        let b = record(0, sample("b", 1, None));
        assert_eq!((a, b), (1, 2));
        // Same id back, content swapped, active again after a close.
        mark_closed(a, REASON_EXPIRED);
        let again = record(a, sample("a2", 2, None));
        assert_eq!(again, a);
        let hist = HISTORY.read().unwrap();
        assert_eq!(hist.len(), 2, "replace must not grow the history");
        let a_entry = hist.iter().find(|n| n.id == a).unwrap();
        assert_eq!(a_entry.summary, "a2");
        assert_eq!(a_entry.closed, None);
        assert_eq!(a_entry.urgency, 2);
    }

    /// The list popup's snapshot: newest first, capped by `max`, a
    /// closed entry still present and flagged.
    #[test]
    fn history_rows_newest_first_capped_and_flagged() {
        let _g = fresh();
        let a = record(0, sample("a", 1, None));
        let _b = record(0, sample("b", 1, None));
        mark_closed(a, REASON_DISMISSED);
        let rows = history_rows(10);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].summary, "b", "newest first");
        assert!(!rows[0].closed);
        assert_eq!(rows[1].summary, "a");
        assert!(rows[1].closed, "a closed row stays in the history");
        assert_eq!(history_rows(1).len(), 1, "the cap bounds the snapshot");
    }

    #[test]
    fn replace_disabled_and_unknown_id_allocate_new_ids() {
        let _g = fresh();
        let a = record(0, sample("a", 1, None));
        SETTINGS.write().unwrap().replace_existing = false;
        assert_ne!(record(a, sample("b", 1, None)), a, "replace disabled");
        *SETTINGS.write().unwrap() = DEFAULTS;
        // Unknown replaces_id falls through to a fresh id too.
        assert_ne!(record(99, sample("c", 1, None)), 99);
    }

    #[test]
    fn history_is_capped_at_history_size() {
        let _g = fresh();
        SETTINGS.write().unwrap().history_size = 3;
        for i in 0..5 {
            record(0, sample(&format!("s{i}"), 1, None));
        }
        let hist = HISTORY.read().unwrap();
        assert_eq!(hist.len(), 3);
        assert_eq!(hist.front().unwrap().summary, "s2", "oldest evicted");
        assert_eq!(hist.back().unwrap().summary, "s4");
    }

    #[test]
    fn close_only_signals_the_first_close_of_a_known_id() {
        let _g = fresh();
        assert_eq!(mark_closed(7, REASON_CLOSED), (false, false), "unknown");
        let id = record(0, sample("x", 1, None));
        assert_eq!(mark_closed(id, REASON_CLOSED), (true, true));
        assert_eq!(
            mark_closed(id, REASON_CLOSED),
            (true, false),
            "already closed: known, but no second signal"
        );
    }

    #[test]
    fn deadline_rules_follow_the_spec() {
        let _g = fresh();
        assert_eq!(deadline(2, 1_000), None, "critical never auto-expires");
        assert_eq!(deadline(1, 0), None, "0 never expires");
        let d = deadline(1, -1).expect("default timeout applies");
        assert!(d > Instant::now());
        assert!(d <= Instant::now() + Duration::from_millis(DEFAULTS.default_timeout_ms));
        let d = deadline(0, 2_500).expect("app timeout applies");
        assert!(d <= Instant::now() + Duration::from_millis(2_500));
    }

    #[test]
    fn close_reasons_match_the_spec() {
        assert_eq!(REASON_EXPIRED, 1, "expired");
        assert_eq!(REASON_DISMISSED, 2, "dismissed by the user");
        assert_eq!(REASON_CLOSED, 3, "closed by CloseNotification");
    }

    #[test]
    fn image_data_is_validated_and_repacked_to_tight_rgba() {
        let _g = fresh();
        use zbus::zvariant::Structure;
        // Build the `(iiibiiay)` hint the way the bus carries it: a
        // structure value wrapped as OwnedValue.
        let hint_of = |w: i32, h: i32, stride: i32, alpha: bool, bits: i32, ch: i32, data: Vec<u8>| {
            OwnedValue::try_from(Value::Structure(Structure::from((
                w, h, stride, alpha, bits, ch, data,
            ))))
            .unwrap()
        };
        let mut hints = HashMap::new();
        // Valid 2x2 RGBA with a tight rowstride: bytes survive as-is.
        hints.insert(
            "image-data".to_string(),
            hint_of(2, 2, 8, true, 8, 4, vec![0u8; 16]),
        );
        assert_eq!(
            parse_image_data(&hints),
            Some(ImageData {
                width: 2,
                height: 2,
                data: vec![0u8; 16],
            })
        );
        // Row padding is stripped: rowstride 12 over 8 bytes of pixels.
        let padded: Vec<u8> = (0..24u8).collect();
        hints.insert(
            "image-data".to_string(),
            hint_of(2, 2, 12, true, 8, 4, padded),
        );
        let out = parse_image_data(&hints).expect("padded rows accepted");
        assert_eq!(out.data.len(), 16, "tight RGBA");
        assert_eq!(&out.data[0..8], &[0, 1, 2, 3, 4, 5, 6, 7], "row 0");
        assert_eq!(&out.data[8..16], &[12, 13, 14, 15, 16, 17, 18, 19], "row 1");
        // RGB gains an opaque alpha channel.
        hints.insert(
            "image-data".to_string(),
            hint_of(2, 1, 6, false, 8, 3, vec![10, 20, 30, 40, 50, 60]),
        );
        let out = parse_image_data(&hints).expect("rgb accepted");
        assert_eq!(out.data, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        // channels wins over a contradictory has_alpha (spec: 4 ⇔ alpha).
        hints.insert(
            "image-data".to_string(),
            hint_of(1, 1, 4, false, 8, 4, vec![1, 2, 3, 4]),
        );
        assert_eq!(
            parse_image_data(&hints).map(|i| i.data),
            Some(vec![1, 2, 3, 4]),
            "4 channels keep the alpha byte"
        );
        // Wrong sample depth.
        hints.insert(
            "image-data".to_string(),
            hint_of(2, 2, 8, true, 16, 4, vec![0u8; 16]),
        );
        assert_eq!(parse_image_data(&hints), None, "bits != 8");
        // Truncated buffer (rowstride*height not covered).
        hints.insert(
            "image-data".to_string(),
            hint_of(2, 2, 8, true, 8, 4, vec![0u8; 8]),
        );
        assert_eq!(parse_image_data(&hints), None, "short data");
        // Absent hint.
        hints.clear();
        assert_eq!(parse_image_data(&hints), None);
    }

    #[test]
    fn display_shows_the_newest_active_with_count_and_truncates() {
        let _g = fresh();
        assert_eq!(display_value(), String::new(), "empty history");
        let a = record(0, sample("first", 1, None));
        assert_eq!(display_value(), "first");
        record(0, sample("second", 1, None));
        assert_eq!(display_value(), "second (2)", "count appended");
        // Closing the newest reveals the older one again.
        let newest_id = HISTORY.read().unwrap().back().unwrap().id;
        mark_closed(newest_id, REASON_EXPIRED);
        assert_eq!(display_value(), "first");
        mark_closed(a, REASON_EXPIRED);
        assert_eq!(display_value(), String::new(), "all closed");
        // Long summaries are cut at a char boundary.
        record(0, sample(&"x".repeat(200), 1, None));
        let out = display_value();
        assert_eq!(out.chars().count(), MAX_SUMMARY);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn wheel_walks_the_history_and_new_arrivals_reset_to_live() {
        let _g = fresh();
        record(0, sample("one", 1, None));
        record(0, sample("two", 1, None));
        assert_eq!(display_value(), "two (2)", "live");
        // Down: newest received (position marker), then the older one.
        scroll(false);
        assert_eq!(display_value(), "two (1/2)");
        scroll(false);
        assert_eq!(display_value(), "one (2/2)");
        scroll(false);
        assert_eq!(display_value(), "one (2/2)", "capped at the oldest");
        // Up walks back to live.
        scroll(true);
        scroll(true);
        assert_eq!(display_value(), "two (2)", "live again");
        scroll(true);
        scroll(true);
        assert_eq!(display_value(), "two (2)", "saturated at live");
        // A new notification resets the selection to live.
        scroll(false);
        scroll(false);
        record(0, sample("three", 1, None));
        assert_eq!(display_value(), "three (3)", "arrival resets to live");
    }

    #[test]
    fn dismiss_closes_once_with_reason_2_and_cards_stack_newest_first() {
        let _g = fresh();
        // Six actives: the stack caps at 5.
        let mut ids = Vec::new();
        for i in 0..6 {
            ids.push(record(0, sample(&format!("n{i}"), 1, None)));
        }
        let cards = active_cards(5);
        assert_eq!(cards.len(), 5, "capped at the stack size");
        assert_eq!(cards[0].id, ids[5], "newest first");
        assert_eq!(cards[4].id, ids[1]);
        assert!(
            cards.iter().all(|c| c.id != ids[0]),
            "the 6th (oldest) is not shown"
        );
        // Dismiss signals once, with the user's reason, and leaves the stack.
        assert!(dismiss(ids[5]), "first close signals");
        assert!(!dismiss(ids[5]), "second close is a no-op");
        let closed = HISTORY
            .read()
            .unwrap()
            .iter()
            .find(|n| n.id == ids[5])
            .unwrap()
            .closed;
        assert_eq!(closed, Some(REASON_DISMISSED));
        assert!(active_cards(5).iter().all(|c| c.id != ids[5]));
        // Anything else closed (spec reason 3) is off the stack too.
        mark_closed(ids[4], REASON_CLOSED);
        assert!(active_cards(5).iter().all(|c| c.id != ids[4]));
    }

    /// The popup handle is built once per notification (stable renderer
    /// id = one decode, not one per frame), changes when the entry is
    /// replaced under the same id, and leaves with a dismissal.
    #[test]
    fn icon_handle_is_cached_per_notification() {
        let _g = fresh();
        let mut n = sample("img", 1, None);
        n.image = Some(ImageData {
            width: 2,
            height: 2,
            data: vec![0; 16],
        });
        let id = record(0, n);
        let first = icon_handle(id).expect("image-data card");
        assert_eq!(first, icon_handle(id).expect("second fetch"), "same handle");
        // Same id, fresh pixels: the stale handle must not survive.
        let mut replaced = sample("img2", 1, None);
        replaced.image = Some(ImageData {
            width: 2,
            height: 2,
            data: vec![255; 16],
        });
        assert_eq!(record(id, replaced), id, "replace honours the id");
        let third = icon_handle(id).expect("replaced card");
        assert_ne!(first, third, "replaced content gets a fresh handle");
        // A dismissal takes the card (and its handle) off the stack.
        assert!(dismiss(id));
        assert!(icon_handle(id).is_none());
    }

    /// The tick-side count matches the visible stack without building
    /// (let alone cloning) any card.
    #[test]
    fn active_cards_len_counts_without_payloads() {
        let _g = fresh();
        let with_img = |s: &str| {
            let mut n = sample(s, 1, None);
            n.image = Some(ImageData {
                width: 2,
                height: 2,
                data: vec![7; 16],
            });
            n
        };
        let ids: Vec<u32> = (0..3)
            .map(|i| record(0, with_img(&format!("n{i}"))))
            .collect();
        assert_eq!(active_cards_len(5), 3);
        // Both handles are built while the cards are visible …
        assert!(icon_handle(ids[0]).is_some());
        assert!(icon_handle(ids[1]).is_some());
        // … and the prune keeps them only for what is still visible.
        mark_closed(ids[1], REASON_CLOSED);
        assert_eq!(active_cards_len(5), 2);
        prune_icon_handles();
        assert!(ICONS.read().unwrap().contains_key(&ids[0]));
        assert!(!ICONS.read().unwrap().contains_key(&ids[1]));
    }
}
