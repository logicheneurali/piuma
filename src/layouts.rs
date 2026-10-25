//! Window layouts (roadmap layout-automatici): everything lives in one
//! file, `piuma-layouts.json` next to piuma.json — while the file is
//! absent the layout system is ignored, and the first click on a
//! `layout` sensor creates it with the current workspace's window
//! arrangement. Layouts are named automatically from the (class,
//! workspace) pairs sorted alphabetically — titles are deliberately
//! absent: a terminal follows its directory and a browser its tab, so
//! keying on them multiplied near-duplicate layouts. The same windows
//! always produce the same name, so re-saving replaces instead of
//! duplicating. The activation check runs when a window is created or
//! closed, or moved to another workspace: then the bar applies, once
//! per window set, the saved layout whose windows are all open.
//! Matching is by class only — a title change never blocks a layout;
//! titles guide the pairing of same-class windows (exact, then
//! resemblance, then position). A workspace move only re-evaluates the
//! match: a layout already applied for the current window set is never
//! re-applied, so moving windows by hand is never fought. A layout
//! owns its workspaces exclusively: an extra window there keeps it
//! unapplied; other workspaces never veto. Tiled sizes are restored
//! through a measured probe; a window is never floated to force it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::hypr;
use crate::windows::{self, WinRow};

/// How long the sensor keeps showing "✓ saved" after a click.
const FLASH: std::time::Duration = std::time::Duration::from_secs(2);

/// One saved window: identified by its class (the match key), with the
/// saved title kept for the pairing of same-class windows, placed by
/// workspace and — for floating windows — by pixel geometry. The
/// address is deliberately absent: it changes every run.
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
    /// A tiled window's target size in px — absolute, so the measured
    /// apply loop can recompute the leftover after every dispatch,
    /// once the sibling's edge has already moved.
    TiledResize { address: String, w: i32, h: i32 },
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
    /// Address + workspace of every window at the last check: a window
    /// created or closed, or moved to another workspace, re-runs the
    /// check (manual resizes, raises and workspace switches do not).
    arrangement: HashSet<(String, String)>,
    /// Addresses at the last check, to tell a create/close from a mere
    /// workspace move: only the former starts a new window set.
    addresses: HashSet<String>,
    /// Whether the last check found a matching layout — the text
    /// between checks (`✓`/`⋯`).
    matched: bool,
    /// The layout applied for the current address set: a later manual
    /// move may change the match, but the same layout is not applied
    /// again — moving windows by hand is never fought.
    applied_layout: Option<String>,
    /// Last save, for the "✓ saved" flash.
    flash: Option<Instant>,
    /// Last apply, for the "✓ applied" flash.
    applied_flash: Option<Instant>,
}

/// `~/.config/piuma/piuma-layouts.json`, next to piuma.json.
fn file_path() -> Option<PathBuf> {
    let cfg = crate::CONFIG_PATH.get()?;
    Some(PathBuf::from(cfg).parent()?.join("piuma-layouts.json"))
}

/// The automatic layout name: the JSON encoding of the (class,
/// workspace) pairs sorted alphabetically. Titles are left out on
/// purpose — they change with a terminal's directory or a browser's
/// tab, and keying on them created near-duplicate layouts that then
/// tied against each other. The same windows in any position give the
/// same name (a re-save replaces that layout); a different window set
/// gives a different one.
fn auto_name(entries: &[LayoutEntry]) -> String {
    let mut keys: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| (e.class.as_str(), e.workspace.as_str()))
        .collect();
    keys.sort_unstable();
    serde_json::to_string(&keys).unwrap_or_default()
}

/// Store a layout under its class/workspace signature, dropping any
/// older entry with the same signature: layouts saved by earlier
/// versions were keyed by (class, title), so a retitled window left
/// near-duplicates behind — the first re-save folds them back into one.
fn insert_layout(file: &mut LayoutFile, entries: Vec<LayoutEntry>) {
    let signature = auto_name(&entries);
    file.retain(|key, existing| key == &signature || auto_name(existing) != signature);
    file.insert(signature, entries);
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
    insert_layout(&mut file, entries);
    // Text-only refresh: the sensor shows ✓ for the set just saved
    // without dispatching anything (the check itself only runs on a
    // window being created or closed).
    let matched = best_match(&file, rows).map(|(name, _)| name.clone());
    state.matched = matched.is_some();
    state.applied_layout = matched;
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
    if let Some(trigger) = arrangement_trigger(state, rows) {
        if trigger == Trigger::CreatedOrClosed {
            // A new window set: the previous apply no longer shields it.
            state.applied_layout = None;
        }
        match state.file.as_ref().and_then(|f| best_match(f, rows)) {
            Some((name, entries)) => {
                state.matched = true;
                // A workspace move that still fits the layout already
                // applied for this set must not undo the manual move.
                if applies(state, name) {
                    eprintln!("[piuma] layout applied: {name}");
                    state.applied_flash = Some(Instant::now());
                    state.applied_layout = Some(name.clone());
                    apply(rows, entries);
                }
            }
            None => state.matched = false,
        }
    }
    status_text(state)
}

/// What changed since the last check. A create/close starts a new window
/// set (the applied layout may fire again); a mere workspace move keeps
/// the set, so an already applied layout is not re-applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Trigger {
    CreatedOrClosed,
    WorkspaceOnly,
}

/// Whether the arrangement (address + workspace) differs from the last
/// check — the only event that re-runs it. Manual resizes, raises and
/// workspace switches change neither address nor workspace.
fn arrangement_trigger(state: &mut LayoutState, rows: &[WinRow]) -> Option<Trigger> {
    let addresses: HashSet<String> = rows.iter().map(|r| r.address.clone()).collect();
    let arrangement: HashSet<(String, String)> = rows
        .iter()
        .map(|r| (r.address.clone(), r.workspace.clone()))
        .collect();
    if arrangement == state.arrangement {
        return None;
    }
    let trigger = if addresses == state.addresses {
        Trigger::WorkspaceOnly
    } else {
        Trigger::CreatedOrClosed
    };
    state.addresses = addresses;
    state.arrangement = arrangement;
    Some(trigger)
}

/// Whether this layout still needs applying for the current window set:
/// the one already applied is left alone (manual moves are not undone),
/// a different one (or none yet) may fire.
fn applies(state: &LayoutState, name: &str) -> bool {
    state.applied_layout.as_deref() != Some(name)
}

/// The saved layout that fits the open windows: its classes must all be
/// available, and its own workspaces must hold no extra window. More
/// windows win (the most specific layout); among layouts of the same
/// size the one whose titles still fit the open windows best wins, and
/// a tie keeps the first in file order. A tie is no longer a reason to
/// do nothing: whenever a layout fits, one is applied.
fn best_match<'a>(
    file: &'a LayoutFile,
    rows: &[WinRow],
) -> Option<(&'a String, &'a Vec<LayoutEntry>)> {
    let mut best: Option<(usize, f32, &String, &Vec<LayoutEntry>)> = None;
    for (name, entries) in file {
        if !matches(rows, entries) {
            continue;
        }
        let score = match_score(rows, entries);
        let better = match &best {
            None => true,
            Some((len, s, _, _)) => {
                entries.len() > *len || (entries.len() == *len && score > *s)
            }
        };
        if better {
            best = Some((entries.len(), score, name, entries));
        }
    }
    best.map(|(_, _, name, entries)| (name, entries))
}

/// How well the assigned windows' titles still fit the saved ones — used
/// only to choose between layouts of the same size: an exact title
/// scores 1, a changed one the share of the saved words still present.
fn match_score(rows: &[WinRow], entries: &[LayoutEntry]) -> f32 {
    assign(rows, entries)
        .map(|pairs| {
            pairs
                .iter()
                .map(|(ri, ei)| title_resemble(&entries[*ei].title, &rows[*ri].title))
                .sum()
        })
        .unwrap_or(0.0)
}

/// Pair saved entries with open windows: per class, each entry takes the
/// unused window that fits it best — exact title first, then a window
/// already on the layout's workspace (placement is more reliable than a
/// title that follows a directory or a tab), then the saved title's
/// words still present, then the closest saved position. This is what
/// makes same-class apps usable: a changed title never blocks the
/// layout, it only reorders the pairing.
fn assign(rows: &[WinRow], entries: &[LayoutEntry]) -> Option<Vec<(usize, usize)>> {
    let layout_ws: HashSet<&str> = entries.iter().map(|e| e.workspace.as_str()).collect();
    let mut used = vec![false; rows.len()];
    let mut pairs = Vec::with_capacity(entries.len());
    for (ei, entry) in entries.iter().enumerate() {
        let mut best: Option<((i32, i32, i32, i32), usize)> = None;
        for (ri, row) in rows.iter().enumerate() {
            if used[ri] || row.class != entry.class {
                continue;
            }
            let key = (
                i32::from(row.title == entry.title),
                i32::from(layout_ws.contains(row.workspace.as_str())),
                (title_resemble(&entry.title, &row.title) * 1000.0) as i32,
                -((row.x - entry.x).abs() + (row.y - entry.y).abs()),
            );
            if best.map_or(true, |(b, _)| key > b) {
                best = Some((key, ri));
            }
        }
        let (_, ri) = best?;
        used[ri] = true;
        pairs.push((ri, ei));
    }
    Some(pairs)
}

/// How much an open title still resembles the saved one: 1.0 when
/// equal, otherwise the share of the saved title's words found in it
/// ("~ - fish" vs "~/src - fish" keeps "fish" and drops the path).
fn title_resemble(saved: &str, open: &str) -> f32 {
    if saved == open {
        return 1.0;
    }
    let words = |s: &str| -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect()
    };
    let a = words(saved);
    let b = words(open);
    if a.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f32 / a.len() as f32
}

/// The sensor's text: the saved flash, the applied flash (the same
/// `FLASH` window — a save wins, it is the rarer, explicit action),
/// ✓ for a matched window set, ⋯ while armed — and nothing at all
/// while the file is absent, when the layout system is ignored.
fn status_text(state: &LayoutState) -> String {
    if let Some(at) = &state.flash {
        if at.elapsed() < FLASH {
            return "✓ saved".to_string();
        }
    }
    if let Some(at) = &state.applied_flash {
        if at.elapsed() < FLASH {
            return "✓ applied".to_string();
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

/// A layout fits when every saved window has an open twin of its class
/// (each entry is paired by `assign`) and its own workspaces hold no
/// extra window: unrelated classes and extra same-class twins there
/// veto. Same-class windows on other workspaces are allowed — they are
/// simply not part of the pairing. An empty layout never matches.
pub fn matches(rows: &[WinRow], entries: &[LayoutEntry]) -> bool {
    if entries.is_empty() {
        return false;
    }
    let mut saved: HashMap<&str, usize> = HashMap::new();
    for e in entries {
        *saved.entry(e.class.as_str()).or_insert(0) += 1;
    }
    let mut open: HashMap<&str, usize> = HashMap::new();
    for r in rows {
        *open.entry(r.class.as_str()).or_insert(0) += 1;
    }
    if saved
        .iter()
        .any(|(class, n)| open.get(class).copied().unwrap_or(0) < *n)
    {
        return false;
    }
    let layout_ws: HashSet<&str> = entries.iter().map(|e| e.workspace.as_str()).collect();
    let mut here: HashMap<&str, usize> = HashMap::new();
    for r in rows.iter().filter(|r| layout_ws.contains(r.workspace.as_str())) {
        *here.entry(r.class.as_str()).or_insert(0) += 1;
    }
    here.iter()
        .all(|(class, n)| saved.get(class).is_some_and(|m| n <= m))
}

/// What to dispatch to re-apply `entries` to `rows`, in order. Pure —
/// the tests build rows by hand. A floating window that lost its
/// geometry is floated back to the saved pixels; a tiled one gets its
/// target size (fired *measured* by `apply`: probe, exact delta,
/// guardrail) and goes back into the tiler when it was floated —
/// never floated itself. A window already at the target is not
/// touched at all; a fullscreen window gets no geometry (move/resize
/// are ignored on it).
pub fn plan(rows: &[WinRow], entries: &[LayoutEntry]) -> Vec<Action> {
    let Some(pairs) = assign(rows, entries) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    for (i, ei) in pairs {
        let r = &rows[i];
        let e = &entries[ei];
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
            // split after the toggle is the tiler's own.
            actions.push(Action::ToggleFloat {
                address: r.address.clone(),
            });
        } else if r.w != e.w || r.h != e.h {
            // Saved as tiled and tiled now, size off: the absolute
            // target — `apply` fires it measured (probe + guardrail).
            actions.push(Action::TiledResize {
                address: r.address.clone(),
                w: e.w,
                h: e.h,
            });
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
            // Never fired here: apply() splits the plan and sends the
            // tiled targets through the measured probe loop.
            Action::TiledResize { .. } => {}
        }
    }
}

/// The probe's step in px, and the smallest tiled window a resize may
/// leave standing (anything under it means a tile was strangled
/// against the wall — see `newly_strangled`).
const PROBE: i32 = 8;
const MIN_TILE: i32 = 80;

/// Apply a matched layout: the moves, toggles and floating geometry
/// fire in one pass, then every tiled target runs *measured* — one
/// window and one axis at a time (height, then width): an `PROBE` px
/// probe shows which way that edge really answers (a bottom window
/// grows by asking up), the exact ask follows from that sign, and
/// anything that strangles a tile (under `MIN_TILE`) or fails to
/// shrink the leftover reverts at once.
/// shortcut: an edge the probe cannot move (a full-width tile's x is
/// the monitor's own border) or a tree that cannot hold the saved
/// shape at all is left to the tiler — without the tree's shape there
/// is no second way to push, and guessing is what strangled a tile to
/// 31 px in the live test. The pass blocks the bar ~150 ms per
/// resize, a few hundred ms in the usual case.
fn apply(rows: &[WinRow], entries: &[LayoutEntry]) {
    let mut simple = Vec::new();
    let mut resizes = Vec::new();
    for a in plan(rows, entries) {
        match a {
            Action::TiledResize { address, w, h } => resizes.push((address, w, h)),
            other => simple.push(other),
        }
    }
    run(&simple);
    // Every axis is guarded on its own (a rejected attempt reverts to
    // its own measured start) and the targets are absolute: one
    // unreachable axis never invalidates the next window's.
    for (address, w, h) in resizes {
        measured_resize(&address, h, 0, 1);
        measured_resize(&address, w, 1, 0);
    }
}

/// One axis of one window, measured: probe `PROBE` px towards the
/// target to learn which way this edge really answers, fire the ask
/// that learning implies, verify nothing got strangled and the
/// leftover shrank (revert when not) — at most two rounds, for an
/// edge that moves slower than asked. A silent edge or a rejected
/// ask leaves that axis to the tiler; nothing else is affected.
fn measured_resize(address: &str, target: i32, sx: i32, sy: i32) {
    for _round in 0..2 {
        let Some(before) = hypr::fresh_clients() else {
            return;
        };
        let axis = |r: &WinRow| sx * r.w + sy * r.h;
        let Some(start) = before.iter().find(|r| r.address == address) else {
            return; // nothing left to resize
        };
        let leftover = target - axis(start);
        if leftover == 0 {
            return;
        }
        // Probe in the direction the target lies: it both covers the
        // first few px and teaches the edge's response sign.
        let asked = PROBE * leftover.signum();
        windows::dispatch_waited(&windows::resize_rel_call(address, sx * asked, sy * asked));
        let Some(probe_rows) = hypr::fresh_clients() else {
            return;
        };
        let probed = probe_rows.iter().find(|r| r.address == address);
        let moved = probed.map_or(0, |p| axis(p) - axis(start));
        let Some(ask) = probed.and_then(|p| probe_ask(asked, moved, axis(p), target)) else {
            return; // edge silent: nothing happened, nothing to undo
        };
        if ask != 0 {
            windows::dispatch_waited(&windows::resize_rel_call(address, sx * ask, sy * ask));
        }
        let Some(after) = hypr::fresh_clients() else {
            return;
        };
        let now = after.iter().find(|r| r.address == address);
        let bad = newly_strangled(&before, &after)
            || now.map_or(true, |m| (target - axis(m)).abs() >= leftover.abs());
        if bad {
            // Back to this round's start with the learned sign: the ask
            // chain is relative, so one opposite move of the same
            // magnitude undoes it (a clamp that swallowed the ask leaves
            // at most the probe's px — bounded, and re-measured next
            // pass).
            if let Some(cur) = now {
                let back = (axis(start) - axis(cur)) * asked.signum() * moved.signum();
                windows::dispatch_waited(&windows::resize_rel_call(address, sx * back, sy * back));
            }
            return;
        }
    }
}

/// The ask (px along the axis) the probe taught for `target`: the
/// edge's response may be inverted (a bottom window grows through its
/// top border — asking down moved it away), so the ask follows the
/// measured sign; magnitude stays 1:1 (live probes moved exactly
/// their ask). `None` when the edge did not move at all: clamped, and
/// that axis is left to the tiler — with nothing to undo.
/// shortcut: sign only — an edge that answers slower than asked makes
/// the ask overshoot in proportion; the strangle guard reverts it.
fn probe_ask(asked: i32, moved: i32, probed: i32, target: i32) -> Option<i32> {
    if moved == 0 {
        return None;
    }
    Some((target - probed) * asked.signum() * moved.signum())
}

/// A tiled window that stood at least `MIN_TILE` on both sides and
/// fell under one during the resize: a strangle in progress, and the
/// caller reverts. Floating windows are no business of the probe.
fn newly_strangled(before: &[WinRow], after: &[WinRow]) -> bool {
    after.iter().any(|r| {
        !r.floating
            && (r.w < MIN_TILE || r.h < MIN_TILE)
            && before.iter().any(|b| {
                b.address == r.address && !b.floating && b.w >= MIN_TILE && b.h >= MIN_TILE
            })
    })
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

    /// The trigger of the check: a window created or closed starts a new
    /// window set, a window moved to another workspace keeps it and still
    /// re-runs the check (the workspace veto depends on placement).
    /// Manual resizes, raises and workspace switches change no address
    /// and no workspace, so nothing runs. `tick` itself is not tested
    /// here: it dispatches real `hyprctl` calls, and the pure pieces it
    /// composes (`arrangement_trigger`, `applies`, `best_match`, `plan`)
    /// are covered individually.
    #[test]
    fn check_runs_on_creation_closure_or_workspace_move() {
        let mut state = LayoutState::default();
        let a = row("0xA", "kitty", "shell", "1", false);
        assert_eq!(
            arrangement_trigger(&mut state, &[a.clone()]),
            Some(Trigger::CreatedOrClosed),
            "the first set is a change"
        );
        assert_eq!(arrangement_trigger(&mut state, &[a.clone()]), None, "same windows: no check");
        let moved = row("0xA", "kitty", "shell", "2", false);
        assert_eq!(
            arrangement_trigger(&mut state, &[moved.clone()]),
            Some(Trigger::WorkspaceOnly),
            "a workspace move re-runs the check"
        );
        assert_eq!(arrangement_trigger(&mut state, &[moved]), None, "same workspace: no check");
        let b = row("0xB", "firefox", "Home", "1", false);
        assert_eq!(
            arrangement_trigger(&mut state, &[a.clone(), b.clone()]),
            Some(Trigger::CreatedOrClosed),
            "a new window is an event"
        );
        assert_eq!(
            arrangement_trigger(&mut state, &[b.clone()]),
            Some(Trigger::CreatedOrClosed),
            "a closed window is an event"
        );
        assert_eq!(arrangement_trigger(&mut state, &[b]), None, "same set again: quiet");
    }

    /// The applied layout is not fired again for the same window set: a
    /// manual move may change the match, but it is never undone. A
    /// different layout (or none applied yet) may fire.
    #[test]
    fn an_already_applied_layout_is_not_reapplied() {
        let mut state = LayoutState::default();
        assert!(applies(&state, "A"), "nothing applied yet");
        state.applied_layout = Some("A".to_string());
        assert!(!applies(&state, "A"), "the same layout is left alone");
        assert!(applies(&state, "B"), "another layout may apply");
    }

    /// The applied flash: `✓ applied` for the `FLASH` window after an
    /// apply, then the plain match sign — and an explicit save still
    /// outranks it (both flashes use the same window).
    #[test]
    fn status_shows_the_applied_flash_then_the_match_sign() {
        let mut state = LayoutState::default();
        state.file = Some(BTreeMap::new());
        state.matched = true;
        state.applied_flash = Some(Instant::now());
        assert_eq!(status_text(&state), "✓ applied", "fresh apply");
        state.applied_flash = Some(Instant::now() - FLASH);
        assert_eq!(status_text(&state), "✓", "the flash fades to the match sign");
        state.flash = Some(Instant::now());
        assert_eq!(status_text(&state), "✓ saved", "a save outranks an old apply");
    }

    /// The trigger: every saved window needs an open twin of its class,
    /// and the layout's own workspaces must hold no extra window. The
    /// title is no longer part of the key — a terminal that followed
    /// its directory keeps its layout.
    #[test]
    fn matches_requires_the_classes_and_a_clean_workspace() {
        let entries = vec![entry("kitty", "shell", "1", false), entry("firefox", "Home", "1", false)];
        let same = vec![
            row("0xA", "kitty", "shell", "2", false),
            row("0xB", "firefox", "Home", "3", false),
        ];
        assert!(matches(&same, &entries), "workspace and order are free");
        let other_ws = vec![
            same[0].clone(),
            same[1].clone(),
            row("0xC", "mail", "Inbox", "5", false),
        ];
        assert!(matches(&other_ws, &entries), "another workspace's window never vetoes");
        let extra_here = vec![
            same[0].clone(),
            same[1].clone(),
            row("0xC", "mail", "Inbox", "1", false),
        ];
        assert!(
            !matches(&extra_here, &entries),
            "an extra window on the layout's own workspace vetoes"
        );
        let twin = vec![
            same[0].clone(),
            row("0xD", "kitty", "shell", "1", false),
            same[1].clone(),
        ];
        assert!(
            matches(&twin, &entries),
            "one same-class window on the layout's workspace fits the saved count"
        );
        let twins = vec![
            same[0].clone(),
            row("0xD", "kitty", "shell", "1", false),
            row("0xE", "kitty", "shell", "1", false),
            same[1].clone(),
        ];
        assert!(
            !matches(&twins, &entries),
            "more same-class windows than saved on the layout's workspace block"
        );
        assert!(!matches(&same[..1], &entries), "a missing window blocks");
        let retitle = vec![row("0xA", "kitty", "vim", "1", false), same[1].clone()];
        assert!(matches(&retitle, &entries), "a changed title no longer blocks");
        let twin_elsewhere = vec![
            same[0].clone(),
            row("0xD", "kitty", "other", "7", false),
            same[1].clone(),
        ];
        assert!(
            matches(&twin_elsewhere, &entries),
            "a same-class window elsewhere is not part of the layout"
        );
        assert!(!matches(&same, &[]), "an empty layout never matches");
    }

    /// Volatile titles in a multi-window app: the layout still applies
    /// (the plan produces the geometry) instead of being dropped by a
    /// tie between same-class windows.
    #[test]
    fn volatile_titles_do_not_block_a_multi_window_layout() {
        let entries = vec![
            LayoutEntry { x: 100, y: 0, w: 300, h: 400, ..entry("kitty", "~ - fish", "1", true) },
            LayoutEntry { x: 500, y: 0, w: 300, h: 400, ..entry("kitty", "~/src - fish", "1", true) },
        ];
        let mut left = row("0xA", "kitty", "~/Documents - fish", "1", true);
        left.x = 100;
        let mut right = row("0xB", "kitty", "OC | something", "1", true);
        right.x = 500;
        assert!(matches(&[left.clone(), right.clone()], &entries), "the classes fit");
        let actions = plan(&[left, right], &entries);
        assert_eq!(actions.len(), 2, "both twins get their geometry");
    }

    /// Same-class windows pair by title when it is stable, then by the
    /// saved position: the pairing always succeeds.
    #[test]
    fn assign_pairs_same_class_by_title_then_position() {
        let entries = vec![
            LayoutEntry { x: 500, ..entry("kitty", "~/src - fish", "1", true) },
            LayoutEntry { x: 100, ..entry("kitty", "~ - fish", "1", true) },
        ];
        let rows = vec![
            row("0xA", "kitty", "zzz", "1", true),
            row("0xB", "kitty", "qqq", "1", true),
        ];
        let pairs = assign(&rows, &entries).expect("enough windows");
        assert_eq!(pairs, vec![(0, 0), (1, 1)], "row order decides when titles are alien");
        // The titles are stable: they pick the rows regardless of order.
        let titled = vec![
            row("0xA", "kitty", "~ - fish", "1", true),
            row("0xB", "kitty", "~/src - fish", "1", true),
        ];
        let pairs = assign(&titled, &entries).expect("enough windows");
        assert_eq!(pairs, vec![(1, 0), (0, 1)], "titles guide the pairing");
        // All titles are volatile: the saved position breaks the tie.
        let mut far = row("0xA", "kitty", "zzz", "1", true);
        far.x = 490;
        let mut near = row("0xB", "kitty", "qqq", "1", true);
        near.x = 90;
        let pairs = assign(&[near, far], &entries).expect("enough windows");
        assert_eq!(pairs, vec![(1, 0), (0, 1)], "closest saved position wins");
    }

    /// A same-class window already on the layout's workspace is preferred
    /// over one sitting elsewhere.
    #[test]
    fn extra_same_class_elsewhere_is_not_selected() {
        let entries = vec![entry("kitty", "shell", "1", true)];
        let mut elsewhere = row("0xB", "kitty", "other", "7", true);
        elsewhere.x = 900;
        let mut here = row("0xA", "kitty", "shell", "1", true);
        here.x = 10;
        let pairs = assign(&[elsewhere, here], &entries).expect("enough windows");
        assert_eq!(pairs, vec![(1, 0)], "the window already there wins");
    }

    /// The automatic name is the (class, workspace) signature: titles do
    /// not take part, so a retitled window re-saves the same layout
    /// instead of creating a near-duplicate.
    #[test]
    fn auto_name_is_the_class_workspace_signature() {
        let a = entry("kitty", "shell", "1", false);
        let b = entry("firefox", "Home", "2", true);
        let name_ab = auto_name(&[a.clone(), b.clone()]);
        let name_ba = auto_name(&[b.clone(), a.clone()]);
        assert_eq!(name_ab, name_ba, "order must not matter");
        let retitled = vec![
            LayoutEntry { title: "~/src - fish".to_string(), ..a.clone() },
            LayoutEntry { title: "Other tab".to_string(), ..b.clone() },
        ];
        assert_eq!(auto_name(&retitled), name_ab, "titles are not part of the name");
        let other = auto_name(&[entry("kitty", "shell", "2", false)]);
        assert_ne!(name_ab, other, "a different workspace is a different layout");
        let more = auto_name(&[a.clone(), b.clone(), entry("kitty", "shell", "1", false)]);
        assert_ne!(name_ab, more, "a different window count is a different layout");
        let weird = auto_name(&[entry("wm class", "ignored", "1", false)]);
        let parsed: Vec<(String, String)> = serde_json::from_str(&weird).expect("key is valid JSON");
        assert_eq!(parsed[0].0, "wm class");
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

    /// Same classes, titles changed: both layouts fit by class, the one
    /// whose titles still resemble the open windows wins; when neither
    /// resembles, a layout is still applied — a tie no longer means
    /// "do nothing".
    #[test]
    fn best_match_uses_titles_only_to_choose_between_equal_layouts() {
        let mut file: LayoutFile = BTreeMap::new();
        file.insert(
            "fish".to_string(),
            vec![
                entry("kitty", "shell", "1", false),
                entry("kitty", "~ - fish", "1", false),
                entry("kitty", "~/src - fish", "1", false),
            ],
        );
        file.insert(
            "tools".to_string(),
            vec![
                entry("kitty", "shell", "1", false),
                entry("kitty", "~: btop", "1", false),
                entry("kitty", "~: journalctl", "1", false),
            ],
        );
        let open = vec![
            row("0xA", "kitty", "shell down", "1", false),
            row("0xB", "kitty", "~/Downloads - fish", "1", false),
            row("0xC", "kitty", "fish", "1", false),
        ];
        assert_eq!(
            best_match(&file, &open).map(|(n, _)| n.as_str()),
            Some("fish"),
            "the layout whose titles still resemble wins"
        );
        let alien = vec![
            row("0xA", "kitty", "zzz", "1", false),
            row("0xB", "kitty", "qqq", "1", false),
            row("0xC", "kitty", "www", "1", false),
        ];
        assert!(
            best_match(&file, &alien).is_some(),
            "no resemblance still applies a fitting layout"
        );
        let other_class = vec![
            row("0xA", "konsole", "shell down", "1", false),
            row("0xB", "konsole", "fish", "1", false),
            row("0xC", "konsole", "fish", "1", false),
        ];
        assert_eq!(best_match(&file, &other_class).map(|(n, _)| n.as_str()), None);
    }

    /// The move to the saved workspace comes first; a tiled window
    /// then gets its measured size target, a floating one the exact
    /// pixels.
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
        let actions = plan(&rows, &entries);
        assert_eq!(
            actions.len(),
            5,
            "move + size target, geometry, toggle + geometry"
        );
        assert!(matches!(
            actions[0],
            Action::Move { ref address, ref workspace } if address == "0xA" && workspace == "1"
        ));
        assert!(matches!(
            actions[1],
            Action::TiledResize { ref address, w: 300, h: 400 } if address == "0xA"
        ));
        assert!(matches!(actions[2], Action::Geometry { ref address, .. } if address == "0xB"));
        assert!(matches!(actions[3], Action::ToggleFloat { ref address, .. } if address == "0xC"));
        assert!(matches!(actions[4], Action::Geometry { ref address, .. } if address == "0xC"));
    }

    /// A window saved as tiled is never floated back: a floating one
    /// goes back into the tiler (its split is then the tiler's own),
    /// while a floating entry still gets its exact pixels.
    #[test]
    fn plan_keeps_tiled_windows_tiled() {
        let tiled_entry = vec![entry("kitty", "shell", "1", false)];
        let floating_now = vec![row("0xA", "kitty", "shell", "1", true)];
        assert!(
            matches!(plan(&floating_now, &tiled_entry)[..], [Action::ToggleFloat { .. }]),
            "back into the tiler: no pixel geometry on a tiled entry"
        );

        let floating = vec![entry("mpv", "video", "1", true)];
        let tiled_now = vec![row("0xB", "mpv", "video", "1", false)];
        assert!(matches!(
            plan(&tiled_now, &floating)[..],
            [Action::ToggleFloat { .. }, Action::Geometry { .. }]
        ));
    }

    /// A tiled window that lost its size gets its absolute target as
    /// the action — fired measured by `apply`, never a float and
    /// never the saved position: the tiler owns the split, the probe
    /// owns the size.
    #[test]
    fn plan_targets_tiled_window_size() {
        let target = vec![LayoutEntry {
            w: 640,
            h: 480,
            ..entry("kitty", "shell", "1", false)
        }];
        let mut row_now = row("0xA", "kitty", "shell", "1", false);
        row_now.w = 960;
        row_now.h = 1080;
        assert!(
            matches!(
                plan(&[row_now], &target)[..],
                [Action::TiledResize { ref address, w: 640, h: 480 }] if address == "0xA"
            ),
            "absolute target, no float, no position"
        );
    }

    /// The probe teaches the edge's sign: an answer in the asked
    /// direction gives the direct ask, an inverted one (a bottom
    /// window grows through its top border) flips it, a silent edge
    /// gives nothing — that axis stays with the tiler, undamaged.
    #[test]
    fn probe_ask_follows_the_measured_edge() {
        assert_eq!(probe_ask(8, 8, 508, 700), Some(192), "grow: direct answer");
        assert_eq!(probe_ask(-8, -8, 492, 300), Some(-192), "shrink: direct answer");
        assert_eq!(probe_ask(8, 8, 508, 506), Some(-2), "overshoot is corrected");
        assert_eq!(probe_ask(8, 0, 503, 700), None, "silent edge: clamped, no undo needed");
        assert_eq!(
            probe_ask(8, -8, 495, 1022),
            Some(-527),
            "inverted: grows by asking the other way"
        );
        assert_eq!(
            probe_ask(-8, 8, 511, 300),
            Some(211),
            "inverted: shrinks by asking the other way"
        );
    }

    /// The guardrail fires on the window that *fell* under the floor,
    /// not on one that was already small; floating windows are the
    /// tiler's own business.
    #[test]
    fn strangled_tiles_are_the_ones_that_just_fell_under_the_floor() {
        let mut big = row("0xA", "kitty", "t", "1", false);
        big.w = 937;
        big.h = 503;
        let mut strangled = big.clone();
        strangled.w = 31;
        assert!(
            newly_strangled(&[big.clone()], &[strangled.clone()]),
            "937 → 31 px is a strangle"
        );
        let mut grown = big.clone();
        grown.w = 1200;
        assert!(!newly_strangled(&[big.clone()], &[grown]), "growth is fine");
        let mut tiny = big.clone();
        tiny.w = 40;
        assert!(
            !newly_strangled(&[tiny.clone()], &[strangled.clone()]),
            "a tile that was already tiny is no news of ours"
        );
        let mut floating_now = strangled;
        floating_now.floating = true;
        assert!(
            !newly_strangled(&[big.clone()], &[floating_now]),
            "floating windows never count"
        );
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
            plan(&[at_target.clone()], &[entry("kitty", "shell", "1", false)]).is_empty(),
            "tiled at the target: untouched"
        );
        let mut floating_at_target = at_target;
        floating_at_target.floating = true;
        assert!(
            plan(&[floating_at_target.clone()], &[entry("kitty", "shell", "1", true)])
                .is_empty(),
            "floating at the target: untouched"
        );
        assert!(
            plan(&[floating_at_target], &[entry("kitty", "shell", "1", false)])
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
        assert!(plan(&[full], &entries).is_empty());
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
        let actions = plan(&[right, left], &entries);
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
        let actions = plan(&rows, &entries);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::Geometry { ref address, x: 1, .. } if address == "0xA"));
        assert!(matches!(actions[1], Action::Geometry { ref address, x: 2, .. } if address == "0xB"));
    }

    /// Older layouts were keyed by (class, title): re-saving the same
    /// windows folds those near-duplicates into the class/workspace
    /// entry instead of adding another copy.
    #[test]
    fn insert_layout_folds_title_keyed_duplicates() {
        let mut file: LayoutFile = BTreeMap::new();
        file.insert(
            "[[\"kitty\",\"~ - fish\"]]".to_string(),
            vec![LayoutEntry { title: "~ - fish".to_string(), ..entry("kitty", "shell", "1", false) }],
        );
        file.insert(
            "[[\"kitty\",\"~/src - fish\"]]".to_string(),
            vec![LayoutEntry { title: "~/src - fish".to_string(), ..entry("kitty", "shell", "1", false) }],
        );
        insert_layout(&mut file, vec![entry("kitty", "anything", "1", false)]);
        assert_eq!(file.len(), 1, "the old title-keyed copies are gone: {file:?}");
        let (name, entries) = file.iter().next().unwrap();
        assert_eq!(*name, auto_name(entries), "stored under the class/workspace key");
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
