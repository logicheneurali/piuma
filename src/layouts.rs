//! Window layouts (roadmap layout-automatici): a `layout` sensor saves
//! the current workspace's window arrangement on click, and every tick
//! the bar re-applies a saved layout once — as soon as every saved
//! window (class + title) is open again with the exact saved count.
//! A set that moves away re-arms the layout, so opening and closing
//! windows is the only trigger: manual moves between two sets are
//! never fought, and unrelated windows never veto.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::is_valid_layout_name;
use crate::windows::{self, WinRow};

/// How long the sensor keeps showing "✓ saved" after a click.
const FLASH: std::time::Duration = std::time::Duration::from_secs(2);

/// One saved window: identified by class + title (the auto-apply match
/// keys), placed by workspace and — for floating windows — by pixel
/// geometry. The address is deliberately absent: it changes every run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutEntry {
    pub class: String,
    pub title: String,
    pub workspace: String,
    pub floating: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One dispatch that re-applies a saved window (`Move` reuses the
/// taskbar's own call builder).
pub enum Action {
    Move { address: String, workspace: String },
    ToggleFloat { address: String },
    Geometry { address: String, x: i32, y: i32, w: i32, h: i32 },
}

/// The engine's session state, kept on the panel.
#[derive(Default)]
pub struct LayoutState {
    /// Layout name → file contents (`None` = no file): read once per
    /// session and refreshed on save.
    /// shortcut: a layout file edited or deleted by hand is not picked
    /// up until the bar restarts; watch the directory if that matters.
    cache: HashMap<String, Option<Vec<LayoutEntry>>>,
    /// Layouts whose plan already fired for the current window set:
    /// the trigger is the set's transition, not the tick.
    done: HashSet<String>,
    /// Last save, for the "✓ saved" flash.
    flash: Option<(String, Instant)>,
}

/// The sensor's layout name: an absent name means the "default" layout.
/// (Invalid names are rejected by the config validator already; the
/// filesystem helpers double-check at the trust boundary.)
pub fn resolve_name(name: Option<&str>) -> String {
    name.unwrap_or("default").to_string()
}

/// `~/.config/piuma/layouts`, next to piuma.json.
fn dir() -> Option<PathBuf> {
    let cfg = crate::CONFIG_PATH.get()?;
    Some(PathBuf::from(cfg).parent()?.join("layouts"))
}

/// Save the visible windows of the current workspace as `name`'s
/// layout; returns how many windows it saved. Written tmp + rename so
/// a crash can never leave a half-written layout behind.
pub fn save(
    state: &mut LayoutState,
    name: &str,
    rows: &[WinRow],
    active_workspace: Option<&str>,
) -> Result<usize, String> {
    if !is_valid_layout_name(name) {
        return Err(format!("invalid layout name: {name}"));
    }
    let Some(ws) = active_workspace else {
        return Err("no active workspace to save".to_string());
    };
    let entries = entries_for(rows, ws);
    if entries.is_empty() {
        return Err("no visible windows in the current workspace".to_string());
    }
    let dir = dir().ok_or_else(|| "cannot locate the config directory".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.json"));
    let tmp = dir.join(format!(".{name}.json.tmp"));
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    // Cache and latch follow the click: the saved set is by definition
    // matched and already in place — nothing to re-apply.
    state.cache.insert(name.to_string(), Some(entries));
    state.done.insert(name.to_string());
    state.flash = Some((name.to_string(), Instant::now()));
    Ok(rows.iter().filter(|r| r.visible && r.workspace == ws).count())
}

/// The layout entries for one save: the visible windows of the active
/// workspace, in compositor order.
fn entries_for(rows: &[WinRow], active_workspace: &str) -> Vec<LayoutEntry> {
    rows.iter()
        .filter(|r| r.visible && r.workspace == active_workspace)
        .map(|r| LayoutEntry {
            class: r.class.clone(),
            title: r.title.clone(),
            workspace: r.workspace.clone(),
            floating: r.floating,
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        })
        .collect()
}

/// Exact count on the layout's own (class, title) keys: every saved
/// window must be open, and no extra twin of it — while windows the
/// layout doesn't mention (another workspace's session, an unrelated
/// app) never veto the trigger. An empty layout never matches (there
/// is nothing to restore).
pub fn matches(rows: &[WinRow], entries: &[LayoutEntry]) -> bool {
    if entries.is_empty() {
        return false;
    }
    let mut cur: HashMap<(&str, &str), usize> = HashMap::new();
    for r in rows {
        *cur.entry((&r.class, &r.title)).or_insert(0) += 1;
    }
    let mut saved: HashMap<(&str, &str), usize> = HashMap::new();
    for e in entries {
        *saved.entry((&e.class, &e.title)).or_insert(0) += 1;
    }
    saved.iter().all(|(key, n)| cur.get(key) == Some(n))
}

/// What to dispatch to re-apply `entries` to `rows`, in order. Pure —
/// the tests build rows by hand. Tiled windows only go back to their
/// workspace (the tiler owns their geometry); a fullscreen window gets
/// no geometry either (Hyprland ignores move/resize while fullscreen).
pub fn plan(rows: &[WinRow], entries: &[LayoutEntry]) -> Vec<Action> {
    let mut taken = vec![false; rows.len()];
    let mut actions = Vec::new();
    for e in entries {
        // Same class + title, first not-yet-taken window: the order
        // pairs duplicates deterministically.
        let mut found = None;
        for (i, r) in rows.iter().enumerate() {
            if !taken[i] && r.class == e.class && r.title == e.title {
                found = Some(i);
                break;
            }
        }
        let Some(i) = found else { continue };
        taken[i] = true;
        let r = &rows[i];
        if r.workspace != e.workspace {
            actions.push(Action::Move {
                address: r.address.clone(),
                workspace: e.workspace.clone(),
            });
        }
        if r.floating != e.floating {
            actions.push(Action::ToggleFloat {
                address: r.address.clone(),
            });
        }
        if e.floating && !r.fullscreen {
            actions.push(Action::Geometry {
                address: r.address.clone(),
                x: e.x,
                y: e.y,
                w: e.w,
                h: e.h,
            });
        }
    }
    actions
}

/// Fire the actions, waiting for each dispatch to land: a window that
/// must float first and move after cannot race its own toggle.
pub fn run(actions: &[Action]) {
    for a in actions {
        match a {
            Action::Move { address, workspace } => {
                windows::dispatch_waited(&windows::move_call(address, workspace));
            }
            Action::ToggleFloat { address } => {
                windows::dispatch_waited(&windows::toggle_float_call(address));
            }
            Action::Geometry { address, x, y, w, h } => {
                // Resize first: Hyprland's resize preserves the window
                // centre, so it displaces the position — the move must
                // be the last word on where the window ends up.
                windows::dispatch_waited(&windows::resize_call(address, *w, *h));
                windows::dispatch_waited(&windows::move_px_call(address, *x, *y));
            }
        }
    }
}

/// One pass of the engine for one layout: load the file (once per
/// session), and fire the plan the first time the window set matches
/// it — a set that moves away re-arms it. Returns the sensor's text.
pub fn tick(state: &mut LayoutState, name: &str, rows: &[WinRow]) -> String {
    if !state.cache.contains_key(name) {
        state.cache.insert(name.to_string(), load(name));
    }
    if let Some(entries) = state.cache.get(name).and_then(|c| c.as_ref()) {
        if matches(rows, entries) {
            // `insert` is true only the first time: one fire per set.
            if state.done.insert(name.to_string()) {
                run(&plan(rows, entries));
            }
        } else {
            state.done.remove(name);
        }
    }
    status_text(state, name)
}

/// The sensor's text: the layout name plus a state suffix — just
/// saved (flash), ✗ no file, ✓ applied, ⋯ armed and waiting.
pub fn status_text(state: &LayoutState, name: &str) -> String {
    if let Some((n, at)) = &state.flash {
        if n == name && at.elapsed() < FLASH {
            return format!("{name} ✓ saved");
        }
    }
    match state.cache.get(name) {
        Some(None) | None => format!("{name} ✗"),
        Some(Some(_)) if state.done.contains(name) => format!("{name} ✓"),
        Some(Some(_)) => format!("{name} ⋯"),
    }
}

/// The saved layout, or `None` when the file is missing or broken (a
/// broken file reads as "no layout": the sensor shows ✗).
fn load(name: &str) -> Option<Vec<LayoutEntry>> {
    if !is_valid_layout_name(name) {
        return None;
    }
    let path = dir()?.join(format!("{name}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(class: &str, title: &str, ws: &str, floating: bool) -> LayoutEntry {
        LayoutEntry {
            class: class.to_string(),
            title: title.to_string(),
            workspace: ws.to_string(),
            floating,
            x: 10,
            y: 20,
            w: 300,
            h: 400,
        }
    }

    fn row(address: &str, class: &str, title: &str, ws: &str, floating: bool) -> WinRow {
        WinRow {
            address: address.to_string(),
            class: class.to_string(),
            title: title.to_string(),
            workspace: ws.to_string(),
            focused: false,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            floating,
            fullscreen: false,
            visible: true,
        }
    }

    /// The trigger: every saved window (by class + title) open with the
    /// exact saved count, no twin extra — windows outside the layout
    /// are ignored.
    #[test]
    fn matches_requires_exact_counts_on_layout_keys() {
        let entries = vec![entry("kitty", "shell", "1", false), entry("firefox", "Home", "1", false)];
        let same = vec![
            row("0xA", "kitty", "shell", "2", false),
            row("0xB", "firefox", "Home", "3", false),
        ];
        assert!(matches(&same, &entries), "workspace and order are free");
        let unrelated = vec![
            same[0].clone(),
            same[1].clone(),
            row("0xC", "mail", "Inbox", "1", false),
        ];
        assert!(matches(&unrelated, &entries), "unrelated windows never veto");
        let twin = vec![
            same[0].clone(),
            row("0xD", "kitty", "shell", "1", false),
            same[1].clone(),
        ];
        assert!(!matches(&twin, &entries), "a second twin blocks");
        assert!(!matches(&same[..1], &entries), "a missing window blocks");
        let retitle = vec![row("0xA", "kitty", "vim", "1", false), same[1].clone()];
        assert!(!matches(&retitle, &entries), "the title is part of the key");
        assert!(!matches(&same, &[]), "an empty layout never matches");
    }

    /// Tiled windows only return to their workspace; floating ones get
    /// the pixel geometry; toggles come before the geometry they enable.
    #[test]
    fn plan_moves_tiled_and_places_floating() {
        let entries = vec![
            entry("kitty", "shell", "1", false), // tiled, wrong workspace
            entry("mpv", "video", "1", true),    // floating, wrong geometry
            entry("slack", "Slack", "1", true),  // tiled now: toggle + geometry
        ];
        let rows = vec![
            row("0xA", "kitty", "shell", "2", false),
            row("0xB", "mpv", "video", "1", true),
            row("0xC", "slack", "Slack", "1", false),
        ];
        let actions = plan(&rows, &entries);
        assert!(matches!(
            actions[0],
            Action::Move { ref address, ref workspace } if address == "0xA" && workspace == "1"
        ));
        assert!(matches!(actions[1], Action::Geometry { ref address, .. } if address == "0xB"));
        assert!(matches!(actions[2], Action::ToggleFloat { ref address } if address == "0xC"));
        assert!(matches!(actions[3], Action::Geometry { ref address, .. } if address == "0xC"));
    }

    /// A floating window turned tiled gets only the toggle — its size
    /// back to the tiler — and a fullscreen one no geometry at all.
    #[test]
    fn plan_never_fights_the_tiler_or_fullscreen() {
        let entries = vec![entry("kitty", "shell", "1", false)];
        let rows = vec![row("0xA", "kitty", "shell", "1", true)];
        assert!(matches!(plan(&rows, &entries)[0], Action::ToggleFloat { .. }));

        let entries = vec![entry("mpv", "video", "1", true)];
        let mut full = row("0xB", "mpv", "video", "1", true);
        full.fullscreen = true;
        assert!(plan(&[full], &entries).is_empty());
    }

    /// Two same-class windows pair with the entries in order, and each
    /// window is consumed once.
    #[test]
    fn plan_pairs_duplicates_in_order() {
        let entries = vec![
            LayoutEntry { x: 1, ..entry("kitty", "a", "1", true) },
            LayoutEntry { x: 2, ..entry("kitty", "a", "1", true) },
        ];
        let rows = vec![
            row("0xA", "kitty", "a", "1", true),
            row("0xB", "kitty", "a", "1", true),
        ];
        let actions = plan(&rows, &entries);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::Geometry { ref address, x: 1, .. } if address == "0xA"));
        assert!(matches!(actions[1], Action::Geometry { ref address, x: 2, .. } if address == "0xB"));
    }

    /// The save keeps only visible windows of the active workspace.
    #[test]
    fn entries_for_filters_to_the_active_workspace() {
        let rows = vec![
            row("0xA", "kitty", "shell", "1", false),
            row("0xB", "firefox", "Home", "2", false),
            {
                let mut hidden = row("0xC", "mail", "Inbox", "1", false);
                hidden.visible = false;
                hidden
            },
        ];
        let entries = entries_for(&rows, "1");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].class, "kitty");
    }

    /// Entries round-trip through the file format.
    #[test]
    fn entries_round_trip_through_json() {
        let entries = vec![entry("kitty", "shell", "1", true)];
        let json = serde_json::to_string(&entries).unwrap();
        let back: Vec<LayoutEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entries);
    }
}
