//! Window layouts (roadmap layout-automatici): everything lives in one
//! file, `piuma-layouts.json` next to piuma.json — while the file is
//! absent the layout system is ignored, and the first click on a
//! `layout` sensor creates it with the current workspace's window
//! arrangement. Layouts are named automatically: the JSON encoding of
//! the (class, title) pairs sorted alphabetically — the same windows
//! always produce the same name, so re-saving replaces instead of
//! duplicating, and any character a title carries is serializer-
//! escaped. The activation check runs only when a window is created
//! or closed (the address set differs from the last check): then the
//! bar applies, once per window set, the most specific saved layout
//! whose windows are all open with the exact saved count — manual
//! moves are never fought, unrelated windows never veto.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::hypr;
use crate::windows::{self, WinRow};

/// How long the sensor keeps showing "✓ saved" after a click.
const FLASH: std::time::Duration = std::time::Duration::from_secs(2);

/// One saved window: identified by class + title (the match keys),
/// placed by workspace and — for floating windows — by pixel
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
    /// The relative size change a tiled window accepts (the same
    /// split change the manual drag does): `dw`/`dh` are the target
    /// minus the current size.
    TiledResize { address: String, dw: i32, dh: i32 },
}

/// `piuma-layouts.json`: automatic layout name → its windows. A
/// BTreeMap so the file is always written in a stable order.
type LayoutFile = BTreeMap<String, Vec<LayoutEntry>>;

/// The engine's session state, kept on the panel.
#[derive(Default)]
pub struct LayoutState {
    /// The parsed file (`None` = absent: the layout system is ignored
    /// until the first save creates it).
    /// shortcut: a file created or edited by hand is picked up on the
    /// next bar restart, not per event.
    file: Option<LayoutFile>,
    /// Set when the one-time load has run, so an absent file is not
    /// re-read (and a broken one not re-logged) on every pass.
    loaded: bool,
    /// Window addresses at the last check: a differing set (a window
    /// was created or closed) is the only event that re-runs it.
    set: HashSet<String>,
    /// Whether the last check found a matching layout — the text
    /// between checks (`✓`/`⋯`).
    matched: bool,
    /// Last save, for the "✓ saved" flash.
    flash: Option<Instant>,
}

/// `~/.config/piuma/piuma-layouts.json`, next to piuma.json.
fn file_path() -> Option<PathBuf> {
    let cfg = crate::CONFIG_PATH.get()?;
    Some(PathBuf::from(cfg).parent()?.join("piuma-layouts.json"))
}

/// The automatic layout name: the JSON encoding of the (class, title)
/// pairs sorted alphabetically. The same windows in any position give
/// the same name (a re-save replaces that layout), different windows
/// give a different one, and every title character is escaped by the
/// serializer — the key stays a valid, unique string whatever the
/// windows are called.
fn auto_name(entries: &[LayoutEntry]) -> String {
    let mut keys: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| (e.class.as_str(), e.title.as_str()))
        .collect();
    keys.sort_unstable();
    serde_json::to_string(&keys).unwrap_or_default()
}

/// Save the visible windows of the current workspace under their
/// automatic name — replacing any layout saved from the same window
/// set — and create the file on the first save. Returns how many
/// windows it saved.
pub fn save(
    state: &mut LayoutState,
    rows: &[WinRow],
    active_workspace: Option<&str>,
) -> Result<usize, String> {
    let Some(ws) = active_workspace else {
        return Err("no active workspace to save".to_string());
    };
    let entries = entries_for(rows, ws);
    if entries.is_empty() {
        return Err("no visible windows in the current workspace".to_string());
    }
    let path = file_path().ok_or_else(|| "cannot locate the config directory".to_string())?;
    let count = entries.len();
    let mut file = state.file.take().unwrap_or_default();
    file.insert(auto_name(&entries), entries);
    // Text-only refresh: the sensor shows ✓ for the set just saved
    // without dispatching anything (the check itself only runs on a
    // window being created or closed).
    state.matched = best_match(&file, rows).is_some();
    // tmp + rename so a crash can never leave a half-written file; the
    // in-memory copy is kept even when the disk write fails.
    let written = serde_json::to_string_pretty(&file)
        .map_err(|e| e.to_string())
        .and_then(|json| {
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
        });
    state.file = Some(file);
    state.loaded = true;
    written?;
    state.flash = Some(Instant::now());
    Ok(count)
}

/// One pass of the engine: the check runs only when a window was
/// created or closed since the last one (the address set differs) —
/// then the most specific saved layout whose windows are all open
/// with the exact saved count applies, once for that set (the next
/// check waits for the next create/close). Returns the sensor's text
/// — empty while the file is absent (system ignored).
pub fn tick(state: &mut LayoutState, rows: &[WinRow]) -> String {
    if !state.loaded {
        state.loaded = true;
        state.file = load();
    }
    if window_set_changed(state, rows) {
        match state.file.as_ref().and_then(|f| best_match(f, rows)) {
            Some((_, entries)) => {
                state.matched = true;
                apply(entries);
            }
            None => state.matched = false,
        }
    }
    status_text(state)
}

/// Whether a window was created or closed since the last check: the
/// only event that makes the check run — manual moves, resizes and
/// workspace switches change no address.
fn window_set_changed(state: &mut LayoutState, rows: &[WinRow]) -> bool {
    let set: HashSet<String> = rows.iter().map(|r| r.address.clone()).collect();
    if set == state.set {
        return false;
    }
    state.set = set;
    true
}

/// The most specific saved layout whose windows are all open with the
/// exact saved count: a subset loses to the layout saved from this
/// very window set; a length tie keeps the earlier (sorted) name.
fn best_match<'a>(
    file: &'a LayoutFile,
    rows: &[WinRow],
) -> Option<(&'a String, &'a Vec<LayoutEntry>)> {
    let mut best: Option<(&String, &Vec<LayoutEntry>)> = None;
    for (name, entries) in file {
        if matches(rows, entries) && best.map_or(true, |(_, b)| entries.len() > b.len()) {
            best = Some((name, entries));
        }
    }
    best
}

/// The sensor's text: the saved flash, ✓ for a matched (and applied)
/// window set, ⋯ while armed — and nothing at all while the file is
/// absent, when the layout system is ignored.
fn status_text(state: &LayoutState) -> String {
    if let Some(at) = &state.flash {
        if at.elapsed() < FLASH {
            return "✓ saved".to_string();
        }
    }
    match (state.file.is_some(), state.matched) {
        (true, true) => "✓".to_string(),
        (true, false) => "⋯".to_string(),
        (false, _) => String::new(),
    }
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
/// the tests build rows by hand. The saved geometry is the target: a
/// floating window that lost it is floated back to the saved pixels;
/// a tiled one is resized inside the tiler (a relative resize, the
/// same the manual drag does) and is never floated. A window already
/// at the target is not touched at all; a fullscreen window gets no
/// geometry (move/resize are ignored on it). `invert` flips the sign
/// of the tiled resizes (the apply loop's retry when the first sign
/// pushed the split the wrong way).
pub fn plan(rows: &[WinRow], entries: &[LayoutEntry], invert: bool) -> Vec<Action> {
    let mut taken = vec![false; rows.len()];
    let mut actions = Vec::new();
    // Positional pairing: same (class, title) twins are matched by
    // where they sit, not by array order (`clients -j` lists creation
    // order, which can differ between the save and the re-apply).
    let mut rows_order: Vec<usize> = (0..rows.len()).collect();
    rows_order.sort_by_key(|&i| (rows[i].x, rows[i].y));
    let mut wanted: Vec<&LayoutEntry> = entries.iter().collect();
    wanted.sort_by_key(|e| (e.x, e.y));
    for e in wanted {
        let mut found = None;
        for &i in &rows_order {
            if !taken[i] && rows[i].class == e.class && rows[i].title == e.title {
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
        let geom_ok = r.x == e.x && r.y == e.y && r.w == e.w && r.h == e.h;
        if geom_ok || r.fullscreen {
            // At the saved pixels: nothing to do (the floating flag is
            // not worth a dispatch on its own); a fullscreen window is
            // never resized either.
        } else if e.floating {
            if !r.floating {
                actions.push(Action::ToggleFloat {
                    address: r.address.clone(),
                });
            }
            actions.push(Action::Geometry {
                address: r.address.clone(),
                x: e.x,
                y: e.y,
                w: e.w,
                h: e.h,
            });
        } else if r.floating {
            // Saved as tiled, floating now: back into the tiler; the
            // apply loop resizes the new split on the next pass.
            actions.push(Action::ToggleFloat {
                address: r.address.clone(),
            });
        } else {
            // Saved as tiled and tiled now: the relative resize is the
            // only size a tiler accepts (absolute pixels are ignored
            // on tiled windows) — the window keeps its state.
            let (dw, dh) = if invert {
                (r.w - e.w, r.h - e.h)
            } else {
                (e.w - r.w, e.h - r.h)
            };
            if dw != 0 || dh != 0 {
                actions.push(Action::TiledResize {
                    address: r.address.clone(),
                    dw,
                    dh,
                });
            }
        }
    }
    actions
}

/// Fire the actions, waiting for each dispatch to land: a window that
/// must float first and size/place after cannot race its own toggle.
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
            Action::TiledResize { address, dw, dh } => {
                windows::dispatch_waited(&windows::resize_rel_call(address, *dw, *dh));
            }
        }
    }
}

/// Apply a matched layout: the floating windows get one dispatch pass
/// (exact pixels), the tiled ones a re-measure loop — a relative
/// resize moves the neighbouring tile too, so the check repeats while
/// the leftover (`|dw| + |dh|`) shrinks. A pass that makes it worse
/// is retried once with the sign flipped (the resize pushes the split
/// edge towards the tile's sibling, so the sign depends on the tree);
/// if that fails too, it stops — the tiler's own partition stays and
/// a window is never floated to force its geometry.
/// shortcut: a partition needing both signs at the same time can't
/// converge and is left as the tiler left it (the common
/// kill + reopen case re-splits to the same geometry by itself and
/// costs one empty pass). The passes block the bar a few hundred ms
/// each (waited dispatches + a 200 ms settle).
fn apply(entries: &[LayoutEntry]) {
    let Some(mut rows) = hypr::fresh_clients() else {
        return;
    };
    let mut prev = usize::MAX;
    let mut inverted = false;
    for _ in 0..6 {
        let actions = plan(&rows, entries, inverted);
        let leftover: usize = actions
            .iter()
            .map(|a| match a {
                Action::TiledResize { dw, dh, .. } => (dw.abs() + dh.abs()) as usize,
                _ => 0,
            })
            .sum();
        if leftover > 0 && leftover >= prev {
            if inverted {
                return;
            }
            inverted = true;
        }
        prev = leftover;
        // One tiled resize per pass: a resize moves the split edge
        // shared with the sibling tile, so two resizes at once undo
        // each other (the typical case: the split between two stacked
        // tiles) — the next pass re-measures and takes the next one.
        let actions = first_resize_only(actions);
        if actions.is_empty() {
            return;
        }
        run(&actions);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let Some(fresh) = hypr::fresh_clients() else {
            return;
        };
        rows = fresh;
    }
}

/// The actions one pass fires: every non-resize action, but only the
/// first tiled resize (see the loop above; extracted for a test).
fn first_resize_only(mut actions: Vec<Action>) -> Vec<Action> {
    let mut resized = false;
    actions.retain(|a| match a {
        Action::TiledResize { .. } => {
            let keep = !resized;
            resized = true;
            keep
        }
        _ => true,
    });
    actions
}

/// The saved layouts, or `None` when the file is missing or broken (a
/// broken file logs once and reads as "absent": the first save
/// recreates it).
fn load() -> Option<LayoutFile> {
    let path = file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(file) => Some(file),
        Err(e) => {
            eprintln!("[piuma] piuma-layouts.json is unreadable: {e}");
            None
        }
    }
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

    /// The trigger of the check: it runs when a window is created or
    /// closed (the address set differs) and never otherwise — manual
    /// moves, resizes and workspace switches change no address. `tick`
    /// itself is not tested here: it dispatches real `hyprctl` calls,
    /// and the pure pieces it composes (`window_set_changed`,
    /// `best_match`, `plan`) are covered individually.
    #[test]
    fn check_runs_only_when_a_window_is_created_or_closed() {
        let mut state = LayoutState::default();
        let a = row("0xA", "kitty", "shell", "1", false);
        assert!(window_set_changed(&mut state, &[a.clone()]), "the first set is a change");
        assert!(!window_set_changed(&mut state, &[a.clone()]), "same windows: no check");
        let switched = row("0xA", "kitty", "shell", "2", false);
        assert!(!window_set_changed(&mut state, &[switched]), "a workspace switch changes no address");
        let b = row("0xB", "firefox", "Home", "1", false);
        assert!(window_set_changed(&mut state, &[a.clone(), b.clone()]), "a new window is an event");
        assert!(window_set_changed(&mut state, &[b.clone()]), "a closed window is an event");
        assert!(!window_set_changed(&mut state, &[b.clone()]), "same set again: quiet");
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

    /// The automatic name depends only on the window set: saved in any
    /// order it is the same string (a re-save replaces), and a title's
    /// special characters stay inside the escaped JSON key.
    #[test]
    fn auto_name_is_the_sorted_window_set() {
        let a = entry("kitty", "shell", "1", false);
        let b = entry("firefox", "Home", "2", true);
        let name_ab = auto_name(&[a.clone(), b.clone()]);
        let name_ba = auto_name(&[b.clone(), a.clone()]);
        assert_eq!(name_ab, name_ba, "order must not matter");
        let other = auto_name(&[entry("kitty", "shell", "1", true)]);
        assert_ne!(name_ab, other, "different windows, different name");
        // Special characters: the key is JSON, so quotes stay escaped
        // and the set survives as a parseable list of pairs.
        let weird = auto_name(&[entry("wm class", "a \"quoted\" title\nwith newline", "1", false)]);
        let parsed: Vec<(String, String)> = serde_json::from_str(&weird).expect("key is valid JSON");
        assert_eq!(parsed[0].1, "a \"quoted\" title\nwith newline");
    }

    /// The most specific matching layout wins: a saved subset does not
    /// shadow the layout saved from this very window set.
    #[test]
    fn best_match_prefers_the_most_specific_layout() {
        let mut file: LayoutFile = BTreeMap::new();
        file.insert(
            "subset".to_string(),
            vec![entry("kitty", "shell", "1", false)],
        );
        file.insert(
            "full".to_string(),
            vec![
                entry("kitty", "shell", "1", false),
                entry("firefox", "Home", "1", false),
            ],
        );
        let both = vec![
            row("0xA", "kitty", "shell", "1", false),
            row("0xB", "firefox", "Home", "1", false),
        ];
        assert_eq!(best_match(&file, &both).map(|(n, _)| n.as_str()), Some("full"));
        let only_kitty = vec![row("0xA", "kitty", "shell", "1", false)];
        assert_eq!(best_match(&file, &only_kitty).map(|(n, _)| n.as_str()), Some("subset"));
        let other = vec![row("0xC", "mail", "Inbox", "1", false)];
        assert!(best_match(&file, &other).is_none());
    }

    /// The move to the saved workspace comes first, then the size: a
    /// tiled window is resized inside the tiler (relative resize, no
    /// floating), a floating one gets the exact pixels.
    #[test]
    fn plan_moves_and_restores_geometry() {
        let entries = vec![
            entry("kitty", "shell", "1", false), // tiled, wrong workspace
            entry("mpv", "video", "1", true),    // floating, wrong geometry
            entry("slack", "Slack", "1", true),  // tiled now: toggle + pixels
        ];
        let rows = vec![
            row("0xA", "kitty", "shell", "2", false),
            row("0xB", "mpv", "video", "1", true),
            row("0xC", "slack", "Slack", "1", false),
        ];
        let actions = plan(&rows, &entries, false);
        assert!(matches!(
            actions[0],
            Action::Move { ref address, ref workspace } if address == "0xA" && workspace == "1"
        ));
        assert!(matches!(actions[1], Action::TiledResize { ref address, dw: 300, dh: 400 } if address == "0xA"));
        assert!(matches!(actions[2], Action::Geometry { ref address, .. } if address == "0xB"));
        assert!(matches!(actions[3], Action::ToggleFloat { ref address, .. } if address == "0xC"));
        assert!(matches!(actions[4], Action::Geometry { ref address, .. } if address == "0xC"));
    }

    /// A window saved as tiled is never floated back: a floating one
    /// goes back into the tiler (the resize follows on the next pass
    /// of the apply loop), while a floating entry still gets its
    /// exact pixels.
    #[test]
    fn plan_keeps_tiled_windows_tiled() {
        let tiled_entry = vec![entry("kitty", "shell", "1", false)];
        let floating_now = vec![row("0xA", "kitty", "shell", "1", true)];
        assert!(
            matches!(plan(&floating_now, &tiled_entry, false)[..], [Action::ToggleFloat { .. }]),
            "back into the tiler: no pixel geometry on a tiled entry"
        );

        let floating = vec![entry("mpv", "video", "1", true)];
        let tiled_now = vec![row("0xB", "mpv", "video", "1", false)];
        assert!(matches!(
            plan(&tiled_now, &floating, false)[..],
            [Action::ToggleFloat { .. }, Action::Geometry { .. }]
        ));
    }

    /// A tiled window that lost its size is resized in place with the
    /// relative resize the tiler accepts (target minus current), and
    /// with the flipped sign on the apply loop's retry.
    #[test]
    fn plan_resizes_tiled_windows_in_place() {
        let target = vec![LayoutEntry {
            w: 640,
            h: 480,
            ..entry("kitty", "shell", "1", false)
        }];
        let mut row_now = row("0xA", "kitty", "shell", "1", false);
        row_now.w = 960;
        row_now.h = 1080;
        assert!(matches!(
            plan(&[row_now.clone()], &target, false)[..],
            [Action::TiledResize { ref address, dw: -320, dh: -600 }] if address == "0xA"
        ));
        assert!(matches!(
            plan(&[row_now], &target, true)[..],
            [Action::TiledResize { ref address, dw: 320, dh: 600 }] if address == "0xA"
        ));
    }

    /// At the saved geometry nothing is dispatched — tiled or not, and
    /// even when the floating flag differs: no pointless toggles, no
    /// re-applying what is already there.
    #[test]
    fn plan_touches_nothing_at_the_saved_geometry() {
        let mut at_target = row("0xA", "kitty", "shell", "1", false);
        at_target.x = 10;
        at_target.y = 20;
        at_target.w = 300;
        at_target.h = 400;
        assert!(
            plan(&[at_target.clone()], &[entry("kitty", "shell", "1", false)], false).is_empty(),
            "tiled at the target: untouched"
        );
        let mut floating_at_target = at_target;
        floating_at_target.floating = true;
        assert!(
            plan(&[floating_at_target.clone()], &[entry("kitty", "shell", "1", true)], false)
                .is_empty(),
            "floating at the target: untouched"
        );
        assert!(
            plan(&[floating_at_target], &[entry("kitty", "shell", "1", false)], false)
                .is_empty(),
            "pixels already right: the flag is not worth a dispatch"
        );
    }

    /// A fullscreen window gets no geometry at all — Hyprland would
    /// ignore the move/resize.
    #[test]
    fn plan_never_fights_fullscreen() {
        let mut full = row("0xB", "mpv", "video", "1", true);
        full.fullscreen = true;
        let entries = vec![entry("mpv", "video", "1", true)];
        assert!(plan(&[full], &entries, false).is_empty());
    }

    /// Only the first tiled resize fires per pass: two resizes at
    /// once on sibling tiles move the same shared split edge and
    /// cancel each other (the bug that made the loop oscillate
    /// without ever converging).
    #[test]
    fn plan_keeps_one_tiled_resize_per_pass() {
        let rows = vec![
            row("0xA", "kitty", "a", "1", false),
            row("0xB", "kitty", "b", "1", false),
        ];
        let entries = vec![
            LayoutEntry { w: 640, ..entry("kitty", "a", "1", false) },
            LayoutEntry { w: 500, ..entry("kitty", "b", "1", false) },
        ];
        let full = plan(&rows, &entries, false);
        assert_eq!(full.len(), 2, "both tiles need a resize");
        let actions = first_resize_only(full);
        assert!(
            matches!(actions[..], [Action::TiledResize { ref address, .. }] if address == "0xA"),
            "one resize per pass, the first positional match"
        );
    }

    /// Same-title twins pair by where they stand, not by the array
    /// order: `clients -j` lists creation order, which can be reversed
    /// with respect to the saved positions after a kill and reopen.
    #[test]
    fn plan_pairs_twins_by_position_not_array_order() {
        let entries = vec![
            LayoutEntry { x: 100, ..entry("kitty", "dup", "1", true) },
            LayoutEntry { x: 500, ..entry("kitty", "dup", "1", true) },
        ];
        let mut left = row("0xL", "kitty", "dup", "1", true);
        left.x = 100;
        let mut right = row("0xR", "kitty", "dup", "1", true);
        right.x = 500;
        // Array order reversed with respect to the saved positions.
        let actions = plan(&[right, left], &entries, false);
        assert_eq!(actions.len(), 2, "one geometry per twin");
        assert!(matches!(actions[0], Action::Geometry { ref address, x: 100, .. } if address == "0xL"));
        assert!(matches!(actions[1], Action::Geometry { ref address, x: 500, .. } if address == "0xR"));
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
        let actions = plan(&rows, &entries, false);
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
