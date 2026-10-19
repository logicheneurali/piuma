//! System tray (StatusNotifierItem) — the D-Bus half of the `tray` sensor.
//!
//! Implements the freedesktop StatusNotifierItem specification (the `org.kde`
//! names are what every real implementation speaks — the interfaces are
//! identical, `org.freedesktop.StatusNotifierItem` is tried as a fallback):
//!
//! * piuma registers `org.kde.StatusNotifierHost-<pid>` (a visualization
//!   exists, so items never fall back to the legacy XEmbed tray) and takes
//!   `org.kde.StatusNotifierWatcher` when it is free;
//! * when another tray already owns the watcher, piuma observes it instead:
//!   same item list, same registration signals (observer mode);
//! * per item we read the properties the spec mandates (Status, IconName /
//!   IconPixmap, AttentionIcon*, ToolTip), listen to the `New*` signals to
//!   re-read immediately, and drop the item when its bus name dies;
//! * clicks and the wheel are forwarded as `Activate` / `SecondaryActivate` /
//!   `ContextMenu` / `Scroll` with screen-coordinate hints; an item that
//!   exposes none of the press methods (appindicator style: Steam,
//!   OpenRGB, …) opens its `com.canonical.dbusmenu` menu instead — see
//!   `dbusmenu` and the registration probe (`probe_click`).
//!
//! Item state lives in `STATE`; the view takes a snapshot through `items()`
//! and is poked with `Message::TrayItemsChanged` so changes show up at once.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::Sender;
use iced::futures::channel::oneshot;
use iced::futures::stream::StreamExt;
use iced::widget::container::Style as ContainerStyle;
use iced::widget::{container, image, MouseArea, Row, text};
use iced::{Alignment, Element, Length, Subscription};
use iced_exwlshell::reexport::Anchor;
use zbus::fdo::RequestNameFlags;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::proxy::{Builder, CacheProperties};
use zbus::{Connection, Proxy};

use crate::Message;

// ── constants ──────────────────────────────────────────────────────

/// Interface every item implements (`org.kde` is the de-facto standard;
/// the freedesktop spelling is the spec-literal fallback).
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const ITEM_IFACE_FALLBACK: &str = "org.freedesktop.StatusNotifierItem";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
/// Paths apps use to call the watcher; all real watchers serve the first
/// one, the others cover spec-literal implementations.
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_PATHS: &[&str] = &[
    "/StatusNotifierWatcher",
    "/org/kde/StatusNotifierWatcher",
    "/org/freedesktop/StatusNotifierWatcher",
];
/// Conventional item path when registration carries a bus name only.
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

/// Default icon edge in pixels, before the bar knows its geometry;
/// `icon_target()` starts here.
pub const ICON_SIZE: u16 = 16;
/// Current icon edge: main.rs stores the usable height of the tray
/// sensor box on every config load, so icons fill their sensor and never
/// overflow it. The widgets (`row`) and the asset resolution
/// (`best_pixmap`, `resolve_image`) all read it. Relaxed ordering: a
/// stale read only means an icon scales slightly off until the next tick.
static ICON_TARGET: AtomicU16 = AtomicU16::new(ICON_SIZE);

/// The icon edge in px the bar renders at right now.
pub fn icon_target() -> u16 {
    ICON_TARGET.load(Ordering::Relaxed)
}

/// Store the icon edge (main computes it from the sensor's usable height).
/// Already-resolved items are not re-fetched: svg/named art scales with
/// the widget anyway, raster art scales at render time and is picked at
/// the new size on the item's next refresh.
pub fn set_icon_target(size: u16) {
    ICON_TARGET.store(size, Ordering::Relaxed);
}

/// Gap the row puts between two icons — part of one slot's width.
const ROW_GAP: u16 = 4;

/// One icon's total width in the row: the icon plus the gap after it —
/// the number `main` needs to cap the row to the sensor's room.
pub fn slot_width() -> u32 {
    u32::from(icon_target()) + u32::from(ROW_GAP)
}
/// Qt scroll convention: 120 units per wheel notch; this stack reports
/// 15 px per notch, so pixels are scaled by 8.
const SCROLL_UNITS_PER_PIXEL: f32 = 8.0;
/// Highlight drawn around items whose status is `NeedsAttention`.
/// Frame colour for `NeedsAttention` items: clearly red, not orange —
/// attention icons are typically yellow/orange (E2E: an orange frame was
/// invisible against them).
pub const ATTENTION_COLOR: iced::Color = iced::Color::from_rgb(1.0, 0.25, 0.25);

// ── view-facing data ───────────────────────────────────────────────

/// How an icon is rendered; picked by the view (svg through the svg
/// widget, everything else through the image widget).
#[derive(Clone)]
pub enum TrayImage {
    File(PathBuf),
    Rgba(image::Handle),
}

impl TrayImage {
    pub fn is_svg(&self) -> bool {
        match self {
            TrayImage::File(p) => p
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("svg")),
            TrayImage::Rgba(_) => false,
        }
    }
}

/// One tray icon as handed to the view (identified by bus + path, so a
/// click can never hit a stale index after the list changed).
#[derive(Clone)]
pub struct TrayIcon {
    pub bus: String,
    pub path: String,
    pub image: TrayImage,
    pub tooltip: String,
    pub attention: bool,
}

/// Mouse button forwarded to an item.
#[derive(Debug, Clone, Copy)]
pub enum TrayButton {
    Left,
    Middle,
    Right,
}

/// What a press on an item must do: the item's DBusMenu first when the
/// press is the right button and a menu exists, else the spec method
/// when the item implements it, else the menu (appindicator items
/// expose neither `Activate` nor `ContextMenu` — see `dbusmenu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTarget {
    Action,
    Menu,
}

/// Click-related methods an item actually exposes, read once from its
/// introspection at registration (methods never appear or disappear on
/// a live item).
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    pub activate: bool,
    pub context_menu: bool,
}

// ── state ──────────────────────────────────────────────────────────

/// A tracked item's raw state; `image`/`tooltip` are the resolved forms.
struct Item {
    /// Item id as exposed in the property list and in the registered /
    /// unregistered signals: `bus + path` (the KDE watcher's form).
    reg: String,
    bus: String,
    path: String,
    status: String,
    image: Option<TrayImage>,
    tooltip: String,
    /// Spec methods this item implements (introspection probe).
    caps: Caps,
    /// Its `com.canonical.dbusmenu` object path, when it has one.
    menu_path: Option<String>,
}

static STATE: RwLock<Vec<Item>> = RwLock::new(Vec::new());

/// Short poison-tolerant lock: one panicking update must not kill the tray.
fn with_items<T>(f: impl FnOnce(&mut Vec<Item>) -> T) -> T {
    let mut items = STATE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut items)
}

/// Snapshot for the view: only items with a renderable icon.
pub fn items() -> Vec<TrayIcon> {
    with_items(|items| {
        items
            .iter()
            .filter_map(|i| {
                i.image.as_ref().map(|image| TrayIcon {
                    bus: i.bus.clone(),
                    path: i.path.clone(),
                    image: image.clone(),
                    tooltip: i.tooltip.clone(),
                    attention: i.status == "NeedsAttention",
                })
            })
            .collect()
    })
}

fn upsert(items: &mut Vec<Item>, reg: &str, bus: &str, path: &str) -> bool {
    match items.iter_mut().find(|i| i.bus == bus && i.path == path) {
        Some(existing) => {
            existing.reg = reg.to_string();
            false
        }
        None => {
            items.push(Item {
                reg: reg.to_string(),
                bus: bus.to_string(),
                path: path.to_string(),
                status: String::new(),
                image: None,
                tooltip: String::new(),
                // Filled by the registration probe right after; until
                // then a click falls back to the spec methods (their
                // failure is logged, exactly like before the probe).
                caps: Caps::default(),
                menu_path: None,
            });
            true
        }
    }
}

fn remove(items: &mut Vec<Item>, bus: &str, path: &str) -> Option<Item> {
    let idx = items.iter().position(|i| i.bus == bus && i.path == path)?;
    Some(items.remove(idx))
}

fn remove_by_reg(items: &mut Vec<Item>, reg: &str) -> Option<Item> {
    let idx = items.iter().position(|i| i.reg == reg)?;
    Some(items.remove(idx))
}

/// Tell the UI to rebuild (`try_send`: a full channel only costs the
/// rebuild until the next tick, while blocking here would stall D-Bus).
fn notify(sender: &Sender<Message>) {
    let mut sender = sender.clone();
    let _ = sender.try_send(Message::TrayItemsChanged);
}

// ── icon / tooltip resolution ──────────────────────────────────────

// The icon theme rule itself lives with the sensors' icons in
// `icons::icon_theme` (one rule, one `gsettings` spawn per process).

/// ARGB32 in network byte order (the spec's wire format) → RGBA bytes.
pub fn argb_to_rgba(data: &[u8]) -> Vec<u8> {
    data.chunks_exact(4)
        .flat_map(|px| [px[1], px[2], px[3], px[0]])
        .collect()
}

/// Strip the spec's XHTML markup subset (filtering tags is explicitly
/// allowed for hosts); everything between `<…>` is dropped.
pub fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn tooltip_string(title: &str, text: &str) -> String {
    let title = strip_markup(title);
    let text = strip_markup(text);
    match (title.is_empty(), text.is_empty()) {
        (true, true) => String::new(),
        (false, true) => title,
        (true, false) => text,
        (false, false) => format!("{title}\n{text}"),
    }
}

/// Prefer the declared size closest to what the bar renders.
fn best_pixmap(pixmaps: &[(i32, i32, Vec<u8>)]) -> Option<&(i32, i32, Vec<u8>)> {
    let target = i32::from(icon_target());
    pixmaps
        .iter()
        .filter(|(w, h, data)| *w > 0 && *h > 0 && data.len() == (*w as usize * *h as usize * 4))
        .min_by_key(|(w, _, _)| (*w - target).abs())
}

/// Icon per the spec's preference: name from the theme first, pixmap
/// (decoded from ARGB wire format) as fallback. The theme walk goes
/// through `icons::resolve` — the sensors' own resolver — so both share
/// one lookup policy and its cache (misses included: an unknown name is
/// not re-searched on every property change).
fn resolve_image(name: &str, pixmaps: &[(i32, i32, Vec<u8>)]) -> Option<TrayImage> {
    if !name.is_empty() {
        if let Some(path) = crate::icons::resolve(crate::icons::icon_theme(), name, icon_target()) {
            return Some(TrayImage::File(path));
        }
    }
    best_pixmap(pixmaps).map(|(w, h, data)| {
        TrayImage::Rgba(image::Handle::from_rgba(*w as u32, *h as u32, argb_to_rgba(data)))
    })
}

// ── item property refresh ──────────────────────────────────────────

/// Build a non-caching proxy for an item's SNI interface, or None when
/// the item exposes no known StatusNotifierItem interface at all. The
/// returned proxy borrows `bus`/`path` (it keeps them internally).
async fn item_proxy<'p>(
    conn: &Connection,
    bus: &'p str,
    path: &'p str,
) -> Option<(Proxy<'p>, &'static str)> {
    for iface in [ITEM_IFACE, ITEM_IFACE_FALLBACK] {
        let built = Builder::<Proxy>::new(conn)
            .destination(bus)
            .ok()?
            .path(path)
            .ok()?
            .interface(iface)
            .ok()?
            .cache_properties(CacheProperties::No)
            .build()
            .await;
        if let Ok(proxy) = built {
            // `Status` is mandatory in the spec: if it reads, the interface works.
            if proxy.get_property::<String>("Status").await.is_ok() {
                return Some((proxy, iface));
            }
        }
    }
    None
}

/// Re-read every property the visualization needs and store the resolved
/// forms; None means the item is gone or unreadable.
async fn refresh_item(conn: &Connection, bus: &str, path: &str) -> Option<()> {
    let (proxy, _iface) = item_proxy(conn, bus, path).await?;

    let status = proxy
        .get_property::<String>("Status")
        .await
        .unwrap_or_else(|_| "Active".to_string());
    let icon_name = proxy.get_property::<String>("IconName").await.unwrap_or_default();
    let icon_pixmaps: Vec<(i32, i32, Vec<u8>)> =
        proxy.get_property("IconPixmap").await.unwrap_or_default();
    let att_name = proxy.get_property::<String>("AttentionIconName").await.unwrap_or_default();
    let att_pixmaps: Vec<(i32, i32, Vec<u8>)> =
        proxy.get_property("AttentionIconPixmap").await.unwrap_or_default();
    let (tip_name, tip_pixmaps, tip_title, tip_text): (
        String,
        Vec<(i32, i32, Vec<u8>)>,
        String,
        String,
    ) = proxy.get_property("ToolTip").await.unwrap_or_default();

    // NeedsAttention swaps in the attention icon when the item provides one.
    let (main_name, main_pixmaps) = if status == "NeedsAttention"
        && !(att_name.is_empty() && att_pixmaps.is_empty())
    {
        (att_name.as_str(), att_pixmaps.as_slice())
    } else {
        (icon_name.as_str(), icon_pixmaps.as_slice())
    };
    let resolved = resolve_image(main_name, main_pixmaps)
        .or_else(|| resolve_image(&icon_name, &icon_pixmaps))
        .or_else(|| resolve_image(&tip_name, &tip_pixmaps));
    let tooltip = tooltip_string(&tip_title, &tip_text);

    with_items(|items| {
        items
            .iter_mut()
            .find(|i| i.bus == bus && i.path == path)
            .map(|item| {
                item.status = status.clone();
                item.image = resolved;
                item.tooltip = tooltip.clone();
            })
            .is_some()
    })
    .then_some(())
}

// ── click capability probe ─────────────────────────────────────────

/// The item's own `Introspect` document, or "" when unreadable (the
/// probe then simply reports "no methods", i.e. the pre-probe behavior).
async fn introspect_xml(conn: &Connection, bus: &str, path: &str) -> String {
    let Ok(builder) = Builder::<Proxy>::new(conn)
        .destination(bus)
        .and_then(|b| b.path(path))
        .and_then(|b| b.interface("org.freedesktop.DBus.Introspectable"))
        .map(|b| b.cache_properties(CacheProperties::No))
    else {
        return String::new();
    };
    let Ok(proxy) = builder.build().await else {
        return String::new();
    };
    proxy
        .call_method("Introspect", &())
        .await
        .ok()
        .and_then(|msg| msg.body().deserialize::<String>().ok())
        .unwrap_or_default()
}

/// The slice of an introspection document that belongs to one
/// interface: method names of other interfaces on the same object must
/// not count as the item's. Empty when the interface is absent.
fn iface_block<'a>(xml: &'a str, iface: &str) -> &'a str {
    let open = format!("<interface name=\"{iface}\">");
    let Some(start) = xml.find(&open) else {
        return "";
    };
    let rest = &xml[start + open.len()..];
    match rest.find("</interface>") {
        Some(end) => &rest[..end],
        None => "",
    }
}

/// Read what a press can do on this item: the spec methods it exposes
/// (introspection) and its DBusMenu path (the `Menu` property — "/" or
/// an error means the item has no menu of its own). Runs on the
/// connection executor from `track_recorded`, never inline in a method
/// handler (the same rule as `refresh_item`).
async fn probe_click(conn: &Connection, bus: &str, path: &str) {
    let Some((proxy, _)) = item_proxy(conn, bus, path).await else {
        return;
    };
    // `Menu` is an ObjectPath (`o`): zvariant refuses to decode it into
    // String, so take the typed form and stringify after.
    let menu_path = proxy
        .get_property::<zbus::zvariant::OwnedObjectPath>("Menu")
        .await
        .ok()
        .map(|p| p.to_string())
        .filter(|m| m != "/" && !m.is_empty());
    let xml = introspect_xml(conn, bus, path).await;
    let kde = iface_block(&xml, ITEM_IFACE);
    // The freedesktop spelling is an identical interface: either block
    // proves the method (an item serving only it still counts).
    let freed = if kde.is_empty() {
        iface_block(&xml, ITEM_IFACE_FALLBACK)
    } else {
        kde
    };
    let caps = Caps {
        activate: freed.contains("name=\"Activate\""),
        context_menu: freed.contains("name=\"ContextMenu\""),
    };
    with_items(|items| {
        if let Some(item) = items.iter_mut().find(|i| i.bus == bus && i.path == path) {
            item.caps = caps;
            item.menu_path = menu_path;
        }
    });
}

/// How a press must be handled (see `ClickTarget`): a right press
/// opens the item's own menu whenever it has one (its `ContextMenu` is
/// often a silent no-op — OpenRGB answers `()` and does nothing, while
/// its dbusmenu carries the whole UI); otherwise the spec method when
/// the item exposes it, its DBusMenu when it has one, and a plain spec
/// call as the last resort (whose `UnknownMethod` failure is logged —
/// still the best guess while the probe has not run yet).
pub fn click_target(bus: &str, path: &str, button: TrayButton) -> ClickTarget {
    with_items(|items| {
        let Some(item) = items.iter().find(|i| i.bus == bus && i.path == path) else {
            return ClickTarget::Action;
        };
        // The right press prefers the item's own menu: apps serving
        // their whole UI from the dbusmenu leave the spec's
        // `ContextMenu` as a dead end, which made the icon look menuless.
        if item.menu_path.is_some() && matches!(button, TrayButton::Right) {
            return ClickTarget::Menu;
        }
        let method_exists = match button {
            TrayButton::Left => item.caps.activate,
            TrayButton::Right => item.caps.context_menu,
            // Middle is `SecondaryActivate` with no menu fallback: the
            // wheel-middle press has no "open the menu" tradition.
            TrayButton::Middle => true,
        };
        if method_exists || item.menu_path.is_none() {
            ClickTarget::Action
        } else {
            ClickTarget::Menu
        }
    })
}

/// The item's `com.canonical.dbusmenu` object path, when it has one.
pub fn menu_path(bus: &str, path: &str) -> Option<String> {
    with_items(|items| {
        items
            .iter()
            .find(|i| i.bus == bus && i.path == path)
            .and_then(|i| i.menu_path.clone())
    })
}

// ── D-Bus tasks (run on the connection's own executor) ─────────────

/// Per-item task: any `New*` signal means "re-read me now" (spec).
async fn signal_loop(conn: Connection, sender: Sender<Message>, bus: String, path: String) {
    let Some((proxy, _)) = item_proxy(&conn, &bus, &path).await else {
        return;
    };
    let Ok(mut signals) = proxy.receive_all_signals().await else {
        return;
    };
    while signals.next().await.is_some() {
        if refresh_item(&conn, &bus, &path).await.is_none() {
            with_items(|items| remove(items, &bus, &path));
            notify(&sender);
            return;
        }
        notify(&sender);
    }
}

/// Plain (no property cache needed) item proxy for the owner watcher.
async fn owner_proxy<'p>(conn: &Connection, bus: &'p str, path: &'p str) -> Option<Proxy<'p>> {
    Builder::<Proxy>::new(conn)
        .destination(bus)
        .ok()?
        .path(path)
        .ok()?
        .interface(ITEM_IFACE)
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()
}

/// Per-item task: remove the item — and announce it — only when its bus
/// name is really gone. A transfer to another owner keeps the item (the
/// name never left the bus; the new owner registers itself, which
/// refreshes the item) and keeps the watch armed for the next change.
/// `announce` is set only in owner mode: an observer must never emit the
/// watcher's signals.
async fn death_loop(
    conn: Connection,
    sender: Sender<Message>,
    bus: String,
    path: String,
    reg: String,
    announce: bool,
) {
    let Some(proxy) = owner_proxy(&conn, &bus, &path).await else {
        return;
    };
    // Without a stream we cannot watch, so keep the item rather than
    // dropping it on a monitoring failure.
    let Ok(mut owner) = proxy.receive_owner_changed().await else {
        eprintln!("[piuma] tray: cannot watch {} — keeping it listed", bus);
        return;
    };
    // The stream yields the new owner per change: `Some(_)` = the name is
    // (still) owned — transfer, keep watching; `None` = the name is gone.
    // Stream end is treated like a monitoring failure: stop, keep the item.
    let name_lost = loop {
        match owner.next().await {
            Some(Some(_)) => continue,
            Some(None) => break true,
            None => break false,
        }
    };
    if !name_lost {
        return;
    }

    with_items(|items| remove(items, &bus, &path));
    if announce {
        if let Ok(emitter) = SignalEmitter::new(&conn, WATCHER_PATH) {
            let _ = Watcher::status_notifier_item_unregistered(&emitter, &reg).await;
        }
    }
    notify(&sender);
}

/// Record a registration in the shared state (pure, no I/O): safe to run
/// inside a method handler that must reply promptly. Returns whether the
/// item is new (only then do the per-item tasks need starting).
fn record(reg: &str, bus: &str, path: &str) -> bool {
    with_items(|items| upsert(items, reg, bus, path))
}

/// Watch an item already recorded in the state: read its properties once
/// (the first refresh doubles as the interface probe — no usable SNI
/// interface or item already gone → forget it again, announcing the
/// removal when `announce`), then spawn the signal/death tasks
/// (deduplicated per bus+path). Runs on the connection executor, never
/// inline in a method handler (see `register_status_notifier_item`).
async fn track_recorded(
    conn: Connection,
    sender: Sender<Message>,
    reg: String,
    bus: String,
    path: String,
    is_new: bool,
    announce: bool,
) {
    if refresh_item(&conn, &bus, &path).await.is_none() {
        with_items(|items| remove(items, &bus, &path));
        eprintln!("[piuma] tray: {bus} {path} exposes no usable StatusNotifierItem — ignored");
        if announce {
            if let Ok(emitter) = SignalEmitter::new(&conn, WATCHER_PATH) {
                let _ = Watcher::status_notifier_item_unregistered(&emitter, &reg).await;
            }
        }
        notify(&sender);
        return;
    }
    // Methods + menu path before the first notify, so a click landing
    // the instant the icon shows already finds the probe's result.
    probe_click(&conn, &bus, &path).await;
    notify(&sender);
    if is_new {
        conn.executor()
            .spawn(
                signal_loop(conn.clone(), sender.clone(), bus.clone(), path.clone()),
                "piuma-tray-signals",
            )
            .detach();
        conn.executor()
            .spawn(
                death_loop(conn.clone(), sender.clone(), bus, path, reg, announce),
                "piuma-tray-death",
            )
            .detach();
    }
}

// ── watcher service (owner mode) ───────────────────────────────────

/// The `org.kde.StatusNotifierWatcher` interface. `IsStatusNotifierHostRegistered`
/// is unconditionally true: piuma only runs this code as a host.
#[derive(Clone)]
struct Watcher {
    sender: Sender<Message>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    /// Items register either with a bus name (object path then defaults to
    /// `/StatusNotifierItem`) or with an object path (bus = the caller's
    /// unique name, read from the message header).
    async fn register_status_notifier_item(
        &mut self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] header: Header<'_>,
        service: String,
    ) {
        let (bus, path) = if service.starts_with('/') {
            (
                header.sender().map(|s| s.to_string()).unwrap_or_default(),
                service.clone(),
            )
        } else {
            (service.clone(), DEFAULT_ITEM_PATH.to_string())
        };
        if bus.is_empty() {
            eprintln!("[piuma] tray: registration without sender for {service}");
            return;
        }
        // Record + announce first: the reply must not wait on the caller.
        // The caller is blocked in this very call, so it cannot answer our
        // property reads — probing it here would deadlock both sides (and,
        // zbus dispatching calls inline, wedge the whole object server).
        // The property list and the registered/unregistered signals carry
        // the item id as `bus + path` — the exact form KDE's reference
        // watcher emits (notifierItemId), which observers split at the
        // first `/`; the raw `service` argument would drop the sender for
        // path-form registrations (`/StatusNotifierItem` alone is
        // unresolvable for anyone but us).
        let reg = format!("{bus}{path}");
        let is_new = record(&reg, &bus, &path);
        if let Ok(emitter) = SignalEmitter::new(conn, WATCHER_PATH) {
            let _ = Watcher::status_notifier_item_registered(&emitter, &reg).await;
        }
        // Probe, refresh and per-item tasks run as an executor task, i.e.
        // after this method has returned its reply.
        conn.executor()
            .spawn(
                track_recorded(
                    conn.clone(),
                    self.sender.clone(),
                    reg,
                    bus,
                    path,
                    is_new,
                    true,
                ),
                "piuma-tray-track",
            )
            .detach();
    }

    async fn register_status_notifier_host(
        &mut self,
        #[zbus(connection)] conn: &Connection,
        _service: String,
    ) {
        if let Ok(emitter) = SignalEmitter::new(conn, WATCHER_PATH) {
            let _ = Watcher::status_notifier_host_registered(&emitter).await;
        }
    }

    /// The item ids in the `bus + path` form (what KDE's reference watcher
    /// emits); observers split them at the first `/`.
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        with_items(|items| items.iter().map(|i| i.reg.clone()).collect())
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ── owner / observer modes ─────────────────────────────────────────

/// Owner mode: the interface is already served (registered before the
/// name was claimed, so no registration can race the object server) and
/// per-item tasks do the rest — nothing left to drive here.
async fn serve_owner() -> zbus::Result<()> {
    std::future::pending::<()>().await;
    Ok(())
}

/// Observer mode: another tray owns the watcher, so mirror its list and
/// follow its registration signals. Returns when that watcher's name is
/// lost so the caller can try to take over.
async fn serve_observer(conn: &Connection, sender: &Sender<Message>) -> zbus::Result<()> {
    let watcher = Builder::<Proxy>::new(conn)
        .destination(WATCHER_NAME)?
        .path(WATCHER_PATH)?
        .interface(WATCHER_IFACE)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    // Spec: hosts announce themselves to the watcher.
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    let _ = watcher
        .call_method("RegisterStatusNotifierHost", &(host.as_str(),))
        .await;

    // Subscribe FIRST, then read the list: anything registering in
    // between arrives via signal (dedup) or via the list — never lost.
    let mut signals = watcher.receive_all_signals().await?;
    let existing: Vec<String> = watcher
        .get_property("RegisteredStatusNotifierItems")
        .await
        .unwrap_or_default();
    for reg in existing {
        track_remote(conn, sender, &reg).await;
    }

    // Keep the list in step; this loop ends with the connection.
    let c = conn.clone();
    let s = sender.clone();
    conn.executor()
        .spawn(
            async move {
                while let Some(msg) = signals.next().await {
                    let member = msg.header().member().map(|m| m.as_str().to_string());
                    let reg: Option<String> = msg.body().deserialize().ok();
                    match (member.as_deref(), reg) {
                        (Some("StatusNotifierItemRegistered"), Some(reg)) => {
                            track_remote(&c, &s, &reg).await;
                        }
                        (Some("StatusNotifierItemUnregistered"), Some(reg)) => {
                            with_items(|items| remove_by_reg(items, &reg));
                            notify(&s);
                        }
                        _ => {}
                    }
                }
            },
            "piuma-tray-watch-signals",
        )
        .detach();

    let mut owner = watcher.receive_owner_changed().await?;
    // Take over only when the watcher name is actually gone: `Some(Some(_))`
    // = another tray received the name (stay the observer), `Some(None)` =
    // the name left the bus; a stream end falls back to the old behavior
    // (attempt the takeover — the name request decides).
    loop {
        match owner.next().await {
            Some(Some(_)) => continue,
            Some(None) | None => break,
        }
    }
    eprintln!("[piuma] tray: the other watcher left — taking over");
    Ok(())
}

/// Parse an item id (`bus + path`, the form the property list and the
/// registered signal carry) into its destination bus name and object
/// path; the path keeps its leading `/`. A bare bus name (no path suffix,
/// as emitted by some watchers) falls back to the conventional item path.
/// `None` when the id carries no bus name (path-only) or is empty: only
/// the watcher knows the sender that registered such a form.
fn split_reg(reg: &str) -> Option<(&str, &str)> {
    if reg.is_empty() {
        return None;
    }
    match reg.find('/') {
        Some(0) => None,
        Some(i) => Some((&reg[..i], &reg[i..])),
        None => Some((reg, DEFAULT_ITEM_PATH)),
    }
}

/// Observer-mode tracking: split the item id and track it like owner mode
/// (properties, signals, death), never emitting the watcher's signals.
async fn track_remote(conn: &Connection, sender: &Sender<Message>, reg: &str) {
    let Some((bus, path)) = split_reg(reg) else {
        eprintln!(
            "[piuma] tray: cannot resolve path-only registration {reg} in observer mode — skipped"
        );
        return;
    };
    // Our own loop is the caller here (nothing waits on us), so awaiting
    // the track inline is fine.
    let is_new = record(reg, bus, path);
    track_recorded(
        conn.clone(),
        sender.clone(),
        reg.to_string(),
        bus.to_string(),
        path.to_string(),
        is_new,
        false,
    )
    .await;
}

// ── subscription / actions ─────────────────────────────────────────

/// The whole D-Bus side as one iced subscription (runs only while a tray
/// sensor is configured — dropping it releases the bus names).
pub fn subscription() -> Subscription<Message> {
    Subscription::run(|| iced::stream::channel(64, run))
}

/// Own the bus, pick the mode, retry forever: a bus error must not kill
/// the tray for good.
async fn run(sender: Sender<Message>) {
    loop {
        if let Err(e) = drive(&sender).await {
            eprintln!("[piuma] tray: D-Bus error: {e}; retrying in 5 s");
            delay(Duration::from_secs(5)).await;
        }
        // Ok = the foreign watcher died: retry ownership immediately.
    }
}

async fn drive(sender: &Sender<Message>) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    // Host name first: items check it to decide between SNI and XEmbed.
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    if let Err(e) = conn.request_name(host.as_str()).await {
        eprintln!("[piuma] tray: host name failed: {e}");
    }
    // Serve the interface before claiming the name: a registration that
    // arrives the instant we own it must find the object already there.
    let watcher = Watcher {
        sender: sender.clone(),
    };
    for p in WATCHER_PATHS {
        conn.object_server().at(*p, watcher.clone()).await?;
    }
    // Another tray owns the watcher → observe it: zbus maps the DoNotQueue
    // `Exists` reply to `Error::NameTaken`, so that error is the observer
    // trigger, not a failure (anything else still retries).
    let taken = match conn
        .request_name_with_flags(WATCHER_NAME, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(
            zbus::fdo::RequestNameReply::PrimaryOwner
            | zbus::fdo::RequestNameReply::AlreadyOwner,
        ) => false,
        Ok(_) => true,
        Err(zbus::Error::NameTaken) => true,
        Err(e) => return Err(e),
    };
    if taken {
        eprintln!("[piuma] tray: {WATCHER_NAME} owned by another tray — observing it");
        serve_observer(&conn, sender).await
    } else {
        serve_owner().await
    }
}

/// Pending delays with their wakeups: one sleeping thread serves the
/// whole queue. The previous implementation spawned — and destroyed —
/// a fresh OS thread per sleep, which meant one thread every second for
/// as long as the notification expiry loop ran.
static TIMERS: std::sync::LazyLock<std::sync::Mutex<Vec<(Instant, oneshot::Sender<()>)>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// The shared timer thread, started on the first delay asked for.
static TIMER_THREAD: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Sleep without pulling in a timer dependency: the shared timer thread
/// wakes the future (the notification expiry loop and the retry paths —
/// both sleep forever, one call at a time each).
pub(crate) async fn delay(d: Duration) {
    let (tx, rx) = oneshot::channel::<()>();
    match TIMERS.lock() {
        Ok(mut timers) => timers.push((Instant::now() + d, tx)),
        // A poisoned queue must never hang the caller.
        Err(_) => {
            let _ = tx.send(());
        }
    }
    TIMER_THREAD.get_or_init(|| {
        std::thread::Builder::new()
            .name("piuma-timers".to_string())
            .spawn(timer_thread)
            .expect("Failed to spawn the piuma timer thread");
    });
    let _ = rx.await;
}

/// Complete every deadline that is due, then sleep until the nearest
/// one — capped at 100 ms so a freshly registered delay is picked up
/// promptly even while the queue sleeps (and an empty queue is a cheap
/// 10 Hz poll instead of a spinning thread).
fn timer_thread() {
    loop {
        let wait = match TIMERS.lock() {
            Ok(mut timers) => {
                let now = Instant::now();
                // `oneshot::Sender::send` consumes it, so the queue is
                // partitioned by drain instead of `retain` (which only
                // sees shared references).
                let mut rest = Vec::with_capacity(timers.len());
                for (due, tx) in timers.drain(..) {
                    if due <= now {
                        let _ = tx.send(());
                    } else {
                        rest.push((due, tx));
                    }
                }
                *timers = rest;
                timers
                    .iter()
                    .map(|(due, _)| due.saturating_duration_since(now))
                    .min()
                    .unwrap_or(Duration::from_millis(100))
                    .min(Duration::from_millis(100))
            }
            Err(_) => Duration::from_millis(100),
        };
        std::thread::sleep(wait);
    }
}

/// Shared blocking session connection for outgoing traffic (tray clicks
/// and scroll, notifications `ActionInvoked` emission).
pub(crate) fn action_connection() -> Option<&'static zbus::blocking::Connection> {
    static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();
    match CONN.get() {
        Some(c) => Some(c),
        None => {
            let conn = zbus::blocking::Connection::session().ok()?;
            Some(CONN.get_or_init(|| conn))
        }
    }
}

/// Call one method on an item, trying the de-facto interface first and
/// the spec-literal one when the item only implements that.
fn call_item(
    conn: &zbus::blocking::Connection,
    bus: &str,
    path: &str,
    method: &str,
    body: &(impl serde::ser::Serialize + zbus::zvariant::DynamicType),
) -> Result<(), zbus::Error> {
    let mut last_err = None;
    for iface in [ITEM_IFACE, ITEM_IFACE_FALLBACK] {
        match zbus::blocking::Proxy::new(conn, bus, path, iface) {
            Ok(proxy) => match proxy.call_method(method, body) {
                Ok(_) => return Ok(()),
                Err(e) => last_err = Some(e),
            },
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| zbus::Error::Failure("no proxy".into())))
}

/// Left/middle/right press → `Activate`/`SecondaryActivate`/`ContextMenu`
/// with screen coordinates as hints (spec); runs in its own thread so the
/// UI never blocks on D-Bus.
pub fn spawn_action(bus: &str, path: &str, button: TrayButton, x: i32, y: i32) {
    let (bus, path) = (bus.to_string(), path.to_string());
    std::thread::spawn(move || {
        let Some(conn) = action_connection() else {
            eprintln!("[piuma] tray: no session bus for {button:?}");
            return;
        };
        let method = match button {
            TrayButton::Left => "Activate",
            TrayButton::Middle => "SecondaryActivate",
            TrayButton::Right => "ContextMenu",
        };
        if let Err(e) = call_item(conn, &bus, &path, method, &(x, y)) {
            eprintln!("[piuma] tray: {method} failed: {e}");
        }
    });
}

/// Wheel over an item → `Scroll(units, orientation)`; units follow the
/// Qt convention (120 per notch) that app-side libraries expect.
pub fn spawn_scroll(bus: &str, path: &str, units: i32, horizontal: bool) {
    let (bus, path) = (bus.to_string(), path.to_string());
    std::thread::spawn(move || {
        let Some(conn) = action_connection() else {
            return;
        };
        let orientation = if horizontal { "horizontal" } else { "vertical" };
        if let Err(e) = call_item(conn, &bus, &path, "Scroll", &(units, orientation)) {
            eprintln!("[piuma] tray: Scroll failed: {e}");
        }
    });
}

/// Wheel delta → spec units. Only pixel deltas act: this stack sends the
/// same notch once as pixels and once as lines (wayland protocol), so
/// acting on both would fire `Scroll` twice per notch (same rule the
/// volume sensor uses since round 12).
pub fn scroll_units(delta: iced::mouse::ScrollDelta) -> Option<(i32, bool)> {
    match delta {
        iced::mouse::ScrollDelta::Pixels { x, y } if x != 0.0 || y != 0.0 => {
            let horizontal = x.abs() > y.abs();
            let px = if horizontal { x } else { y };
            Some(((px * SCROLL_UNITS_PER_PIXEL).round() as i32, horizontal))
        }
        _ => None,
    }
}

// ── view ────────────────────────────────────────────────────────────

/// The tray sensor's content: one widget per item at the current icon
/// target (`icon_target()`, sized by main to the sensor's usable
/// height), with its own mouse handling (left/middle/right press →
/// `Activate`/`SecondaryActivate`/`ContextMenu`, wheel → `Scroll`,
/// hover → the tooltip window, see `TooltipShow`).
/// Built from a snapshot, so messages carry the item's bus name + path —
/// never an index that could go stale before the click lands.
/// `style` is the sensor's own look; the tooltip reuses it via
/// `crate::styled_box` (the 5px tooltip wrapper gap stays outside it).
/// `max` caps the visible icons to what the sensor box holds (a "+N"
/// text counts the hidden rest); icons are vertically centred so art of
/// different aspects sits on one mid-line.
pub fn row<'a>(icons: &'a [TrayIcon], max: u16, font_size: u32) -> Element<'a, Message> {
    let mut row = Row::new()
        .spacing(f32::from(ROW_GAP))
        .align_y(Alignment::Center);
    let mut shown = 0usize;
    for icon in icons.iter().take(usize::from(max)) {
        shown += 1;
        let size = Length::Fixed(f32::from(icon_target()));
        let art: Element<'a, Message> = match &icon.image {
            TrayImage::File(p) if icon.image.is_svg() => {
                iced::widget::svg(iced::widget::svg::Handle::from_path(p.clone()))
                    .width(size)
                    .height(size)
                    .into()
            }
            TrayImage::File(p) => iced::widget::image(image::Handle::from_path(p.clone()))
                .width(size)
                .height(size)
                .into(),
            TrayImage::Rgba(h) => iced::widget::image(h.clone()).width(size).height(size).into(),
        };
        // Spec: items in `NeedsAttention` must be emphasized — framed.
        let framed: Element<'a, Message> = if icon.attention {
            container(art)
                .padding(1)
                .style(move |_theme: &iced::Theme| {
                    let mut style = ContainerStyle::default();
                    style.border.color = ATTENTION_COLOR;
                    style.border.width = 1.0;
                    style.border.radius = 2.0.into();
                    style
                })
                .into()
        } else {
            art
        };

        let (bus, path) = (icon.bus.clone(), icon.path.clone());
        // Hover opens the tooltip as its own window (like the
        // taskbar's): the bar surface is one `thickness` tall, an
        // in-widget overlay would only land on the bar itself. The id
        // rides both messages, so a stale leave cannot close the next
        // item's tooltip; an empty text is ignored on the open side.
        let tip_id = format!("{bus}:{path}");
        let interactive = MouseArea::new(framed)
            .on_press(Message::TrayIconPressed(
                bus.clone(),
                path.clone(),
                TrayButton::Left,
            ))
            .on_middle_press(Message::TrayIconPressed(
                bus.clone(),
                path.clone(),
                TrayButton::Middle,
            ))
            .on_right_press(Message::TrayIconPressed(
                bus.clone(),
                path.clone(),
                TrayButton::Right,
            ))
            .on_scroll(move |delta| {
                Message::TrayIconScrolled(bus.clone(), path.clone(), scroll_units(delta))
            })
            .on_enter(Message::TooltipShow {
                id: tip_id.clone(),
                text: icon.tooltip.clone(),
            })
            .on_exit(Message::TooltipHide { id: tip_id })
            .interaction(iced::mouse::Interaction::Pointer);

        row = row.push(interactive);
    }
    if let Some(rest) = icons.len().checked_sub(shown).filter(|n| *n > 0) {
        row = row.push(text(format!("+{rest}")).size(font_size as f32));
    }
    row.into()
}

/// The cursor point for cursor-anchored surfaces (tray menu, sensor
/// graph popup) in output-relative coordinates (what layer-shell
/// margins need): window-relative cursor + the bar's offset inside its
/// output (from the anchor).
/// The bar surface's corner inside the output (the bar sits at its
/// anchor): lifts window-relative coordinates to output space.
pub fn bar_origin(panel: &crate::Panel) -> (i32, i32) {
    let thickness = crate::panel_thickness(&panel.config) as i32;
    let (sw, sh) = (
        unsafe { crate::SCREEN_WIDTH } as i32,
        unsafe { crate::SCREEN_HEIGHT } as i32,
    );
    // The bar spans the output along its long axis, so only the anchored
    // edge shifts it inside the output.
    if panel.last_anchor == Anchor::Right {
        (sw - thickness, 0)
    } else if panel.last_anchor == Anchor::Bottom {
        (0, sh - thickness)
    } else {
        (0, 0)
    }
}

pub fn cursor_output_point(panel: &crate::Panel) -> Option<(i32, i32)> {
    let (cx, cy) = panel.last_cursor?;
    let (wx, wy) = bar_origin(panel);
    Some((wx + cx as i32, wy + cy as i32))
}

/// Screen coordinates for an SNI activation hint (hints are advisory in
/// the spec): the global form of `cursor_output_point` — the output's
/// position in the global layout added on top.
pub fn hint_point(panel: &crate::Panel) -> Option<(i32, i32)> {
    let (mx, my) = cursor_output_point(panel)?;
    let (ox, oy) = output_origin();
    Some((ox + mx, oy + my))
}

/// The focused output's position in the global layout: the cursor point
/// is output-relative and SNI hints are global. (Was "the last monitor
/// of `hyprctl monitors -j`", which aimed the hint at the other output
/// once a second monitor joined the session.)
pub fn output_origin() -> (i32, i32) {
    crate::focused_output()
        .map(|(_, x, y)| (x, y))
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The state is a process-wide static: serialize tests that touch it.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn argb_wire_order_becomes_rgba() {
        // One red pixel (A=FF R=AA G=BB B=CC) followed by a transparent one.
        let wire = [0xFF, 0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22, 0x33];
        assert_eq!(argb_to_rgba(&wire), [0xAA, 0xBB, 0xCC, 0xFF, 0x11, 0x22, 0x33, 0x00]);
    }

    #[test]
    fn markup_is_filtered_from_tooltips() {
        assert_eq!(strip_markup("<b>Bold</b> and <a href='x'>link</a>"), "Bold and link");
        assert_eq!(strip_markup("plain text"), "plain text");
        assert_eq!(
            tooltip_string("<i>Tip</i>", "Body <b>text</b>"),
            "Tip\nBody text"
        );
    }

    #[test]
    fn registry_upserts_removes_and_snapshots() {
        let _g = TEST_LOCK.lock().unwrap();
        with_items(|items| items.clear());

        // First registration creates, second one only refreshes the reg string.
        assert!(with_items(|i| upsert(i, ":1.50", ":1.50", "/StatusNotifierItem")));
        assert!(!with_items(|i| upsert(i, ":1.50-bis", ":1.50", "/StatusNotifierItem")));
        assert_eq!(with_items(|i| i.len()), 1);
        assert_eq!(with_items(|i| i[0].reg.clone()), ":1.50-bis");

        // No image yet → the view snapshot stays empty.
        assert!(items().is_empty());

        with_items(|i| i[0].image = Some(TrayImage::Rgba(image::Handle::from_rgba(1, 1, vec![0u8; 4]))));
        assert_eq!(items().len(), 1);

        // Removal by registration string (observer mode) and by coordinates.
        with_items(|i| remove_by_reg(i, ":1.50-bis"));
        assert_eq!(with_items(|i| i.len()), 0);
        with_items(|i| remove(i, ":1.60", "/StatusNotifierItem"));
        assert!(items().is_empty());
    }

    #[test]
    fn item_ids_split_into_bus_and_path() {
        // KDE reference id (both registration forms resolve to bus + path).
        assert_eq!(
            split_reg(":1.50/StatusNotifierItem"),
            Some((":1.50", "/StatusNotifierItem"))
        );
        assert_eq!(
            split_reg("org.kde.StatusNotifierItem-9-1/StatusNotifierItem"),
            Some(("org.kde.StatusNotifierItem-9-1", "/StatusNotifierItem"))
        );
        // Bare bus name (no path suffix): the conventional item path.
        assert_eq!(
            split_reg("org.kde.StatusNotifierItem-9-1"),
            Some(("org.kde.StatusNotifierItem-9-1", DEFAULT_ITEM_PATH))
        );
        // Path-only or empty ids carry no bus name — unresolvable here.
        assert_eq!(split_reg("/StatusNotifierItem"), None);
        assert_eq!(split_reg(""), None);
    }

    #[test]
    fn scroll_units_follow_the_notch_conventions() {
        // This stack reports 15 px per notch → 120 units (Qt convention).
        let up = scroll_units(iced::mouse::ScrollDelta::Pixels { x: 0.0, y: 15.0 });
        assert_eq!(up, Some((120, false)));
        let left = scroll_units(iced::mouse::ScrollDelta::Pixels { x: -20.0, y: -3.0 });
        assert_eq!(left, Some((-160, true)));
        // The line form of the SAME notch must not act (it would double
        // every wheel event), and zero deltas are dropped too.
        assert_eq!(
            scroll_units(iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 }),
            None
        );
        assert_eq!(scroll_units(iced::mouse::ScrollDelta::Pixels { x: -0.0, y: -0.0 }), None);
    }

    /// A right press opens the item's own menu even when the spec's
    /// `ContextMenu` exists (OpenRGB answers the method and does
    /// nothing); left/middle keep the spec-first rule, and without a
    /// menu the right press falls back to the spec call.
    #[test]
    fn right_press_prefers_the_items_menu() {
        let _g = TEST_LOCK.lock().unwrap();
        with_items(|items| {
            items.clear();
            assert!(upsert(items, ":1.99", ":1.99", "/StatusNotifierItem"));
            items[0].caps = Caps { activate: true, context_menu: true };
            items[0].menu_path = Some("/MenuBar".to_string());
        });
        let path = "/StatusNotifierItem";
        assert_eq!(click_target(":1.99", path, TrayButton::Right), ClickTarget::Menu);
        assert_eq!(click_target(":1.99", path, TrayButton::Left), ClickTarget::Action);
        assert_eq!(click_target(":1.99", path, TrayButton::Middle), ClickTarget::Action);
        with_items(|items| items[0].menu_path = None);
        assert_eq!(click_target(":1.99", path, TrayButton::Right), ClickTarget::Action);
        with_items(|items| items.clear());
    }

    /// Only the named interface's slice comes back, so a method another
    /// interface declares (or a missing interface) never counts as the
    /// item's own capability.
    #[test]
    fn iface_block_scopes_methods_to_one_interface() {
        let xml = "<node>\n  <interface name=\"org.freedesktop.DBus.Introspectable\">\n    <method name=\"Introspect\"/>\n  </interface>\n  <interface name=\"org.kde.StatusNotifierItem\">\n    <method name=\"Activate\"/>\n    <property name=\"Id\" type=\"s\" access=\"read\"/>\n  </interface>\n</node>";
        let block = iface_block(xml, "org.kde.StatusNotifierItem");
        assert!(block.contains("Activate"));
        assert!(!block.contains("Introspect"), "leaks past </interface>");
        // An interface the document doesn't carry yields nothing.
        assert_eq!(iface_block(xml, "org.freedesktop.StatusNotifierItem"), "");
    }
}
