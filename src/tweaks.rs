//! Session tweaks that patch Hyprland at runtime (the setup's Tweaks
//! tab). Every tweak is optional and absent costs nothing: the tick
//! skips the whole module when the block is off.

use crate::config;
use crate::windows::WinRow;
use std::sync::Mutex;

/// The gap for `count` open windows: `max` with one (or none), then a
/// 25% of the `max - min` range less per extra window, never below
/// `min`. With 4..16: 16, 13, 10, 7, 4, 4…
pub fn gap_for(count: usize, min: i32, max: i32) -> i32 {
    // A config that slipped past validation must not invert the range.
    let (min, max) = (min.min(max), max.max(min));
    let windows = count.max(1) as f32;
    let step = (max - min) as f32 * 0.25;
    ((max as f32 - step * (windows - 1.0)).floor() as i32).max(min)
}

/// The `(inner, outer)` gaps for `count` windows: `gap_for` drives the
/// screen edge, the gap between windows is half of it (rounded down) so
/// the tiling keeps the classic edge-twice-the-inner proportion.
pub fn gaps_for(count: usize, min: i32, max: i32) -> (i32, i32) {
    let outer = gap_for(count, min, max);
    (outer / 2, outer)
}

/// A Hyprland CSS gap: `[top, right, bottom, left]` (the compositor
/// reports `gaps_in`/`gaps_out` in that form, all equal when plain).
type Gaps = [i32; 4];

/// What the module wrote and what to put back. `restore` is captured
/// before the first override of this process (the user's own Hyprland
/// values: `gaps_in` and `gaps_out`), so disabling the tweak or
/// quitting restores them.
#[derive(Default)]
struct State {
    /// Window count at the last successful apply.
    count: Option<usize>,
    /// Outer gap piuma last wrote (`None` = it never overrode anything).
    applied: Option<i32>,
    /// `(gaps_in, gaps_out)` captured before the first override.
    restore: Option<(Gaps, Gaps)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    count: None,
    applied: None,
    restore: None,
});

/// One pass from the bar: apply the dynamic gaps when the window count
/// changed, restore Hyprland's own values when the tweak is off. The
/// configured value drives the screen edge (`gaps_out`); the gap between
/// windows (`gaps_in`) is half of it. Only the total of mapped windows
/// counts (gaps are global: a per-workspace count would re-tile every
/// monitor on every workspace switch), special workspaces excluded.
pub fn sync(panel: &config::PanelConfig, clients: Option<&[WinRow]>) {
    let gaps = panel
        .tweaks
        .as_ref()
        .and_then(|t| t.dynamic_gaps.as_ref())
        .filter(|g| g.enabled);
    let Some(gaps) = gaps else {
        restore();
        return;
    };
    let Some(clients) = clients else {
        return; // no snapshot this pass: retry on the next one
    };
    let count = clients
        .iter()
        .filter(|row| !row.workspace.starts_with("special:"))
        .count();
    let (inner, outer) = gaps_for(count, gaps.min, gaps.max);

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.restore.is_none() {
        state.restore = read_gaps("in").zip(read_gaps("out"));
    }
    if state.count == Some(count) && state.applied == Some(outer) {
        return;
    }
    // Re-apply even when the value matches a previous one: a Hyprland
    // config reload resets the gap, and the next open/close heals it.
    if set_gaps([inner; 4], [outer; 4]) {
        state.count = Some(count);
        state.applied = Some(outer);
    }
}

/// Put the captured value back and forget the override. Called every
/// tick while the tweak is disabled (a no-op after the first restore)
/// and on exit.
pub fn restore() {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.applied.take().is_none() {
        return;
    }
    state.count = None;
    if let Some((inner, outer)) = state.restore.take() {
        set_gaps(inner, outer);
    }
}

/// The bar's exit paths (SIGTERM/SIGINT and the close action) leave
/// Hyprland's own gaps behind.
pub fn restore_on_exit() {
    restore();
}

/// `general:gaps_<name>` as the compositor reports it (`css` is
/// `"8 8 8 8"`).
fn read_gaps(name: &str) -> Option<Gaps> {
    let option = format!("general:gaps_{name}");
    let out = std::process::Command::new("hyprctl")
        .args(["-j", "getoption", &option])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut values = v["css"].as_str()?.split_whitespace();
    Some([
        values.next()?.parse().ok()?,
        values.next()?.parse().ok()?,
        values.next()?.parse().ok()?,
        values.next()?.parse().ok()?,
    ])
}

/// A gap as the Lua config takes it: one integer when all four sides
/// match, otherwise the per-side table (the plugin rejects the CSS
/// string, `"15 15 15 15"`).
fn css(gaps: Gaps) -> String {
    if gaps.iter().all(|side| *side == gaps[0]) {
        gaps[0].to_string()
    } else {
        format!(
            "{{ top = {}, right = {}, bottom = {}, left = {} }}",
            gaps[0], gaps[1], gaps[2], gaps[3]
        )
    }
}

/// `true` when the compositor accepted the runtime config change.
fn set_gaps(inner: Gaps, outer: Gaps) -> bool {
    let call = format!(
        "hl.config({{ general = {{ gaps_in = {}, gaps_out = {} }} }})",
        css(inner),
        css(outer)
    );
    std::process::Command::new("hyprctl")
        .args(["eval", &call])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::{css, gap_for, gaps_for};

    /// The example from the setup's description: 4..16 shrinks by 3 per
    /// extra window (25% of the 12-pixel range) and stops at 4.
    #[test]
    fn dynamic_gaps_step_down_to_the_floor() {
        let gaps: Vec<i32> = (0..7).map(|n| gap_for(n, 4, 16)).collect();
        assert_eq!(gaps, vec![16, 16, 13, 10, 7, 4, 4]);
    }

    /// A range that does not divide by four never rounds above the
    /// previous step and never leaves the [min, max] band.
    #[test]
    fn dynamic_gaps_never_invert_or_overshoot() {
        let steps: Vec<i32> = (1..9).map(|n| gap_for(n, 5, 16)).collect();
        assert_eq!(steps, vec![16, 13, 10, 7, 5, 5, 5, 5]);
        assert_eq!(gap_for(3, 10, 10), 10, "min = max is a fixed gap");
        assert_eq!(gap_for(3, 20, 10), 15, "an inverted config is read min..max");
    }

    /// The edge gap follows the formula; the inner one is half of it.
    #[test]
    fn inner_gap_is_half_the_outer_one() {
        assert_eq!(gaps_for(1, 4, 16), (8, 16));
        assert_eq!(gaps_for(3, 4, 16), (5, 10));
        assert_eq!(gaps_for(5, 4, 16), (2, 4));
        assert_eq!(gaps_for(2, 8, 50), (19, 39), "rounded down");
    }

    /// The Lua config takes a plain integer only for a uniform gap; the
    /// captured per-side value needs the table form (the CSS string is
    /// rejected at runtime).
    #[test]
    fn captured_gaps_roundtrip_through_the_lua_form() {
        assert_eq!(css([15, 15, 15, 15]), "15");
        assert_eq!(
            css([15, 8, 15, 8]),
            "{ top = 15, right = 8, bottom = 15, left = 8 }"
        );
    }
}
