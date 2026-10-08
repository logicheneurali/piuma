//! DBusMenu (`com.canonical.dbusmenu`) — the menu half of
//! appindicator-style tray items (Steam, OpenRGB, …).
//!
//! Those items expose no `Activate`/`ContextMenu` on their
//! `org.kde.StatusNotifierItem` interface (the registration probe in
//! `tray.rs` proves it by introspection): per the appindicator model the
//! *host* renders the item's menu from its `Menu` object path and
//! reports clicks back as `Event(id, "clicked", …)`. This module fetches
//! and flattens that layout — submenus become indented rows, always
//! expanded — and sends the events; `main.rs` draws the menu window.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use zbus::proxy::{Builder, CacheProperties};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, Proxy};

use crate::tray;

/// One menu row as the view wants it: a flattened tree node.
#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    /// DBusMenu `toggle-state`: 0 = off, 1 = on, 2 = inconsistent.
    pub toggle: i32,
    /// Indentation level: 0 = root, +1 per submenu.
    pub depth: u32,
    pub kind: EntryKind,
}

/// Row flavour: a clickable entry, a non-clickable submenu header or a
/// separator line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Item,
    Header,
    Separator,
}

/// Wire form of one node: `(id, properties, children)` — a tuple struct,
/// so serde maps it straight onto the D-Bus structure (the same trick
/// `tray.rs` uses for `ToolTip`); `Type` gives it the matching wire
/// signature for the body decode.
#[derive(Deserialize, zbus::zvariant::Type)]
struct WireNode(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// A node with the variant wrappers already stripped and the children
/// parsed: what `flatten` walks (and what tests build by hand).
struct Node {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<Node>,
}

// ── property readers (spec defaults; a bad property never fails a menu) ──

/// Read one node property, unwrapping the `a{sv}` variant layer (some
/// serializers keep it, some don't — both must read).
fn prop(props: &HashMap<String, OwnedValue>, key: &str) -> Option<Value<'static>> {
    let raw: Value<'static> = Value::from(props.get(key)?.clone());
    Some(match raw {
        Value::Value(inner) => *inner,
        other => other,
    })
}

fn prop_str(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    match prop(props, key) {
        Some(Value::Str(s)) => s.as_str().to_string(),
        _ => String::new(),
    }
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str, default: bool) -> bool {
    match prop(props, key) {
        Some(Value::Bool(b)) => b,
        _ => default,
    }
}

fn prop_i32(props: &HashMap<String, OwnedValue>, key: &str) -> i32 {
    match prop(props, key) {
        Some(Value::I32(i)) => i,
        _ => 0,
    }
}

// ── wire → tree ────────────────────────────────────────────────────

/// One `av` child of a node: a variant wrapping the child's
/// `(id, props, children)` structure (or the structure directly, for
/// serializers that skip the wrapper). Recurses into the child's own
/// children; `None` only for a malformed entry, which is dropped.
fn child_node(ov: OwnedValue) -> Option<Node> {
    let raw: Value<'static> = Value::from(ov);
    let value = match raw {
        Value::Value(inner) => *inner,
        other => other,
    };
    let Value::Structure(s) = value else {
        return None;
    };
    let fields = s.fields();
    let id = match fields.first()? {
        Value::I32(i) => *i,
        _ => return None,
    };
    let props = match fields.get(1) {
        Some(Value::Dict(d)) => d
            .iter()
            .filter_map(|(k, v)| {
                let Value::Str(k) = k else { return None };
                Some((k.as_str().to_string(), v.try_to_owned().ok()?))
            })
            .collect(),
        _ => HashMap::new(),
    };
    let children = match fields.get(2) {
        Some(Value::Array(a)) => a
            .inner()
            .iter()
            .filter_map(|v| child_node(v.clone().try_to_owned().ok()?))
            .collect(),
        _ => Vec::new(),
    };
    Some(Node {
        id,
        props,
        children,
    })
}

/// The `GetLayout` root (already decoded by serde) → the recursive form.
fn root_node(root: WireNode) -> Node {
    Node {
        id: root.0,
        props: root.1,
        children: root.2.into_iter().filter_map(child_node).collect(),
    }
}

// ── tree → rows ────────────────────────────────────────────────────

/// Depth-first flatten: a submenu becomes a non-clickable `Header` row
/// followed by its entries one level deeper (always expanded — the view
/// indents them), an invisible entry drops its whole subtree, a
/// separator becomes its own row.
fn flatten(node: Node, depth: u32, out: &mut Vec<MenuEntry>) {
    if !prop_bool(&node.props, "visible", true) {
        return;
    }
    if prop_bool(&node.props, "separator", false) {
        out.push(MenuEntry {
            id: node.id,
            label: String::new(),
            enabled: false,
            toggle: 0,
            depth,
            kind: EntryKind::Separator,
        });
        return;
    }
    let kind = if node.children.is_empty() {
        EntryKind::Item
    } else {
        EntryKind::Header
    };
    out.push(MenuEntry {
        id: node.id,
        label: tray::strip_markup(&prop_str(&node.props, "label")),
        enabled: prop_bool(&node.props, "enabled", true),
        toggle: prop_i32(&node.props, "toggle-state"),
        depth,
        kind,
    });
    for child in node.children {
        flatten(child, depth + 1, out);
    }
}

/// The toggle gutter: a checked entry shows ✓. Both the row and the
/// width estimate use it, so the menu surface never clips the column.
pub fn toggle_glyph(entry: &MenuEntry) -> &'static str {
    match entry.toggle {
        1 => "✓ ",
        _ => "",
    }
}

// ── D-Bus ──────────────────────────────────────────────────────────

/// The item's menu rows: `AboutToShow` first (the spec lets the app
/// rebuild its layout in response), then the complete `GetLayout` tree,
/// flattened for the view. `main.rs` runs it through `Command::perform`,
/// so a silent app only leaves its own menu unopened — no UI state is
/// held while waiting.
pub async fn fetch(bus: String, menu_path: String) -> Result<Vec<MenuEntry>, String> {
    let conn = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = Builder::<Proxy>::new(&conn)
        .destination(bus.as_str())
        .map_err(|e| format!("destination: {e}"))?
        .path(menu_path.as_str())
        .map_err(|e| format!("path: {e}"))?
        .interface("com.canonical.dbusmenu")
        .map_err(|e| format!("interface: {e}"))?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(|e| format!("proxy: {e}"))?;
    // Best effort: an app ignoring AboutToShow still answers GetLayout.
    let _ = proxy.call_method("AboutToShow", &(0i32,)).await;
    let reply = proxy
        .call_method("GetLayout", &(0i32, -1i32, Vec::<String>::new()))
        .await
        .map_err(|e| format!("GetLayout: {e}"))?;
    let (_revision, root): (u32, WireNode) = reply
        .body()
        .deserialize()
        .map_err(|e| format!("layout decode: {e}"))?;
    let mut entries = Vec::new();
    flatten(root_node(root), 0, &mut entries);
    Ok(entries)
}

/// A row was clicked → the spec `Event(id, "clicked", value, ts)` on the
/// item's menu object; its own thread, exactly like the icon actions
/// (the UI must never block on D-Bus). `value` carries state only for
/// toggle events — a wire variant cannot be empty, so a neutral bool
/// rides along for `clicked`.
pub fn spawn_event(bus: &str, menu_path: &str, id: i32) {
    let (bus, menu_path) = (bus.to_string(), menu_path.to_string());
    std::thread::spawn(move || {
        let Some(conn) = tray::action_connection() else {
            eprintln!("[piuma] tray: no session bus for menu event");
            return;
        };
        let Ok(proxy) = zbus::blocking::Proxy::new(
            conn,
            bus.as_str(),
            menu_path.as_str(),
            "com.canonical.dbusmenu",
        ) else {
            eprintln!("[piuma] tray: no menu proxy for {bus}");
            return;
        };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        if let Err(e) = proxy.call_method("Event", &(id, "clicked", Value::from(false), ts)) {
            eprintln!("[piuma] tray: menu event failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Array, Dict, Signature, Structure, StructureBuilder};

    fn leaf(id: i32, label: &str) -> Node {
        let mut props = HashMap::new();
        props.insert(
            "label".to_string(),
            Value::from(label).try_to_owned().unwrap(),
        );
        Node {
            id,
            props,
            children: Vec::new(),
        }
    }

    fn with_prop<'a>(node: &'a mut Node, key: &str, value: Value<'static>) -> &'a mut Node {
        node.props
            .insert(key.to_string(), value.try_to_owned().unwrap());
        node
    }

    /// Rebuild a node in its exact wire shape — `(id, a{sv}, av)` with
    /// variant-wrapped children — so the roundtrip through `child_node`
    /// is exercised too.
    fn wire_structure<'a>(node: &'a Node) -> Structure<'a> {
        let mut props = Dict::new(&Signature::Str, &Signature::Variant);
        for (k, v) in &node.props {
            // `Dict::add` wraps non-variant values itself (`Value::new`
            // would double-wrap an already-wrapped one): strip first.
            let value = match Value::from(v.clone()) {
                Value::Value(inner) => *inner,
                other => other,
            };
            props.add(k.as_str(), value).unwrap();
        }
        let mut children = Array::new(&Signature::Variant);
        for c in &node.children {
            children
                .append(Value::Value(Box::new(Value::Structure(wire_structure(c)))))
                .unwrap();
        }
        StructureBuilder::new()
            .add_field(node.id)
            .append_field(Value::Dict(props))
            .append_field(Value::Array(children))
            .build()
            .unwrap()
    }

    #[test]
    fn a_flat_menu_reads_labels_and_states() {
        let mut on = leaf(7, "Toggle me");
        with_prop(&mut on, "toggle-state", Value::from(1));
        let mut off = leaf(8, "Disabled");
        with_prop(&mut off, "enabled", Value::from(false));
        let mut root = Node {
            id: 0,
            props: HashMap::new(),
            children: vec![on, off],
        };
        with_prop(&mut root, "label", Value::from("root label"));
        let mut rows = Vec::new();
        flatten(root, 0, &mut rows);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].kind, rows[0].depth), (EntryKind::Header, 0));
        assert_eq!(
            (rows[1].label.as_str(), rows[1].toggle, rows[1].kind),
            ("Toggle me", 1, EntryKind::Item)
        );
        assert_eq!(rows[1].depth, 1);
        assert!(rows[1].enabled);
        assert!(!rows[2].enabled);
        assert_eq!(toggle_glyph(&rows[1]), "✓ ");
    }

    #[test]
    fn markup_and_visibility_follow_the_spec() {
        let mut hidden = leaf(5, "secret");
        with_prop(&mut hidden, "visible", Value::from(false));
        let mut root = Node {
            id: 0,
            props: HashMap::new(),
            children: vec![hidden, leaf(6, "<b>Bold</b> label")],
        };
        with_prop(&mut root, "label", Value::from("<i>menu</i>"));
        let mut rows = Vec::new();
        flatten(root, 0, &mut rows);
        // Header + the visible child only: the hidden subtree is dropped.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "menu");
        assert_eq!(rows[1].label, "Bold label");
    }

    #[test]
    fn a_separator_row_has_no_label_or_action() {
        let mut sep = leaf(9, "should be ignored");
        with_prop(&mut sep, "separator", Value::from(true));
        let mut rows = Vec::new();
        flatten(sep, 0, &mut rows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, EntryKind::Separator);
        assert!(rows[0].label.is_empty());
    }

    #[test]
    fn nested_children_survive_the_wire_roundtrip() {
        // The exact wire shape — a variant-wrapped structure in the
        // `av` child array — parsed back through `child_node`.
        let parent = Node {
            id: 2,
            props: {
                let mut p = HashMap::new();
                p.insert(
                    "label".to_string(),
                    Value::from("outer").try_to_owned().unwrap(),
                );
                p
            },
            children: vec![leaf(3, "inner")],
        };
        let owned = Value::Structure(wire_structure(&parent))
            .try_to_owned()
            .unwrap();
        let parsed = child_node(owned).expect("wire node parses");
        assert_eq!(parsed.id, 2);
        assert_eq!(parsed.children.len(), 1);
        let mut rows = Vec::new();
        flatten(parsed, 0, &mut rows);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, EntryKind::Header);
        assert_eq!((rows[1].label.as_str(), rows[1].depth), ("inner", 1));
    }

    /// Dev-time check of the whole wire path against a real app (Steam
    /// when it runs): the first registered item that owns a `Menu` goes
    /// through the real `fetch()` — AboutToShow, GetLayout, `WireNode`
    /// decode, flatten — and must yield non-empty labelled rows. Skips
    /// silently when no menu-bearing item is registered; panics if the
    /// `Menu` property itself cannot be read as an `OwnedObjectPath`
    /// (the probe's typed read).
    #[test]
    fn a_registered_menu_decodes_through_fetch() {
        let conn = match zbus::blocking::Connection::session() {
            Ok(conn) => conn,
            Err(_) => return,
        };
        let watcher = match zbus::blocking::Proxy::new(
            &conn,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        ) {
            Ok(w) => w,
            Err(_) => return,
        };
        let items: Vec<String> = match watcher.get_property("RegisteredStatusNotifierItems") {
            Ok(items) => items,
            Err(_) => return,
        };
        let mut menu_err = 0;
        for id in items {
            let Some((bus, path)) = id.split_once('/') else {
                continue;
            };
            // split_once ate the path's leading slash.
            let path = format!("/{path}");
            let Ok(item) = zbus::blocking::Proxy::new(
                &conn,
                bus,
                path.as_str(),
                "org.kde.StatusNotifierItem",
            ) else {
                continue;
            };
            // `Menu` is an ObjectPath: zvariant won't decode it into
            // String (the probe's typed read must work too — the final
            // assert panics if it doesn't).
            let menu = match item.get_property::<zbus::zvariant::OwnedObjectPath>("Menu") {
                Ok(menu) => menu.to_string(),
                Err(e) => {
                    menu_err += 1;
                    eprintln!("[piuma] Menu as OwnedObjectPath failed: {e:?}");
                    continue;
                }
            };
            if menu == "/" || menu.is_empty() {
                continue;
            }
            let entries = iced::futures::executor::block_on(fetch(bus.to_string(), menu))
                .unwrap_or_else(|e| panic!("fetch failed for {id}: {e}"));
            assert!(!entries.is_empty(), "empty menu rows for {id}");
            assert!(
                entries.iter().any(|e| !e.label.is_empty()),
                "no labelled row for {id}"
            );
            return;
        }
        assert_eq!(
            menu_err, 0,
            "Menu property unreadable as ObjectPath on {menu_err} items"
        );
    }
}

