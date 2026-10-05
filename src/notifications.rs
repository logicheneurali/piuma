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
//!   expiry (reason 1) and `CloseNotification` (reason 2) only mark the
//!   entry closed and emit the signal — closed entries stay in the
//!   history, which is what the sensor scroll (F2) browses;
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
use iced::Subscription;
use zbus::fdo::RequestNameFlags;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::Message;

// ── constants ──────────────────────────────────────────────────────

const NOTIF_NAME: &str = "org.freedesktop.Notifications";
const NOTIF_PATH: &str = "/org/freedesktop/Notifications";

/// Spec reason codes: 1 = expired, 2 = closed by `CloseNotification`.
const REASON_EXPIRED: u32 = 1;
const REASON_CLOSED: u32 = 2;
/// Longest summary the sensor prints (bars must not stretch).
const MAX_SUMMARY: usize = 60;

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

static SETTINGS: RwLock<Settings> = RwLock::new(DEFAULTS);

/// Image payload from the spec's `image-data` hint, validated at insert
/// (8-bit samples, rows cover `rowstride * height`). The popup resolves
/// priority image-data > image-path > app_icon (F3).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub has_alpha: bool,
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
    /// Reason once closed (1/2); `None` while active.
    pub closed: Option<u32>,
    expires_at: Option<Instant>,
}

/// History in arrival order (back = newest), capped at `history_size`.
static HISTORY: RwLock<VecDeque<Notification>> = RwLock::new(VecDeque::new());
/// Monotonic ids: spec allows any scheme, 1-based and never reused.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

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
            .max(1),
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
    let mut hist = HISTORY.write().unwrap_or_else(|e| e.into_inner());
    if settings.replace_existing && replace != 0 {
        if let Some(existing) = hist.iter_mut().find(|n| n.id == replace) {
            let id = existing.id;
            *existing = Notification { id, ..n };
            existing.closed = None;
            existing.received = Instant::now();
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

/// Spec `image-data` hint `(iiibiiay)`, validated: 8-bit samples and a
/// buffer that covers `rowstride * height` rows (padding allowed).
fn parse_image_data(hints: &HashMap<String, OwnedValue>) -> Option<ImageData> {
    let value = hint(hints, "image-data")?;
    let (width, height, rowstride, has_alpha, bits, data) = value
        .downcast::<(i32, i32, i32, bool, i32, Vec<u8>)>()
        .ok()?;
    let channels: i64 = if has_alpha { 4 } else { 3 };
    let (width, height, rowstride) = (i64::from(width), i64::from(height), i64::from(rowstride));
    if width <= 0
        || height <= 0
        || i64::from(bits) != 8
        || rowstride < width * channels
        || (data.len() as i64) < rowstride * height
    {
        return None;
    }
    Some(ImageData {
        width: width as i32,
        height: height as i32,
        has_alpha,
        data,
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

    /// Close a known notification (reason 2); unknown ids are an error.
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

/// Own the name or give up for good (the activator must not be fought);
/// a transport error retries after 5 s like the tray does.
async fn run(sender: Sender<Message>) {
    loop {
        match drive(&sender).await {
            Ok(()) => return,
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
        // Another server owns it (or its activator already started one):
        // log and skip — v1 never replaces the name.
        Err(zbus::Error::NameTaken) => {
            eprintln!("[piuma] notifications: {NOTIF_NAME} owned elsewhere — server skipped");
            return Ok(());
        }
        Err(e) => return Err(e),
        Ok(_) => return Ok(()),
    }
    eprintln!("[piuma] notifications: server up ({NOTIF_NAME})");
    serve(&conn, sender).await
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
        NEXT_ID.store(1, Ordering::Relaxed);
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
    fn image_data_is_validated() {
        let _g = fresh();
        use zbus::zvariant::Structure;
        // Build the `(iiibiiay)` hint the way the bus carries it: a
        // structure value wrapped as OwnedValue.
        let hint_of = |w: i32, h: i32, stride: i32, alpha: bool, bits: i32, data: Vec<u8>| {
            OwnedValue::try_from(Value::Structure(Structure::from((w, h, stride, alpha, bits, data))))
                .unwrap()
        };
        let mut hints = HashMap::new();
        // Valid 2x2 RGBA (rowstride 8).
        hints.insert("image-data".to_string(), hint_of(2, 2, 8, true, 8, vec![0u8; 16]));
        assert_eq!(
            parse_image_data(&hints),
            Some(ImageData {
                width: 2,
                height: 2,
                has_alpha: true,
                data: vec![0u8; 16],
            })
        );
        // Wrong sample depth.
        hints.insert("image-data".to_string(), hint_of(2, 2, 8, true, 16, vec![0u8; 16]));
        assert_eq!(parse_image_data(&hints), None, "bits != 8");
        // Truncated buffer (rowstride*height not covered).
        hints.insert("image-data".to_string(), hint_of(2, 2, 8, true, 8, vec![0u8; 8]));
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
}
