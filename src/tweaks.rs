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

/// One pass from the bar: every enabled tweak is applied, every disabled
/// one puts the user's own Hyprland values back.
pub fn sync(panel: &config::PanelConfig, clients: Option<&[WinRow]>) {
    sync_gaps(panel, clients);
    sync_shadows(panel);
    sync_borders(panel);
}

/// Dynamic gaps: apply when the window count changed, restore Hyprland's
/// own values when the tweak is off. The configured value drives the
/// screen edge (`gaps_out`); the gap between windows (`gaps_in`) is half
/// of it. Only the total of mapped windows counts (gaps are global: a
/// per-workspace count would re-tile every monitor on every workspace
/// switch), special workspaces excluded.
fn sync_gaps(panel: &config::PanelConfig, clients: Option<&[WinRow]>) {
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
    let count = window_count(clients);
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

/// The windows the gap counts: every mapped client except the special
/// workspaces (scratchpads) and the X11 popup menus. A menu (Steam's
/// hover menus, several toolkits') is reported as a floating client
/// with an empty title and its parent's class/pid — counting it shrank
/// the gaps as if a window had opened, and it has no place in the
/// tiling. A real floating window with no title (rare) is the one
/// false negative: it costs a larger gap, nothing breaks.
fn window_count(clients: &[WinRow]) -> usize {
    clients
        .iter()
        .filter(|row| !row.workspace.starts_with("special:"))
        .filter(|row| !(row.floating && row.title.trim().is_empty()))
        .count()
}

/// Put the captured value back and forget the override. Called every
/// tick while the tweak is disabled (a no-op after the first restore)
/// and on exit.
pub fn restore() {
    restore_gaps();
    restore_shadows();
    restore_borders();
}

fn restore_gaps() {
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


// ── shadows ─────────────────────────────────────────────────────────

/// `decoration:shadow:*` as the compositor reports it. Colours are kept
/// in the `getoption` gradient form (`"99000000 0deg"`, AARRGGBB stops)
/// so a captured value round-trips through `gradient_lua` unchanged.
#[derive(Clone, PartialEq)]
struct ShadowValues {
    enabled: bool,
    range: i32,
    render_power: i32,
    sharp: bool,
    color: String,
    color_inactive: String,
    offset: [i32; 2],
    scale: f32,
}

impl ShadowValues {
    /// The configured values as Hyprland would report them; the tweak
    /// being on means the shadows are on.
    fn from_config(cfg: &config::ShadowsConfig) -> Self {
        Self {
            enabled: true,
            range: cfg.range,
            render_power: cfg.render_power,
            sharp: cfg.sharp,
            color: gradient_from_hex(&cfg.color),
            color_inactive: gradient_from_hex(&cfg.color_inactive),
            offset: cfg.offset,
            scale: cfg.scale,
        }
    }
}

#[derive(Default)]
struct ShadowState {
    /// What piuma last wrote (`None` = it never overrode the shadows).
    applied: Option<ShadowValues>,
    /// The user's own values, captured before the first override.
    restore: Option<ShadowValues>,
}

static SHADOW_STATE: Mutex<ShadowState> = Mutex::new(ShadowState {
    applied: None,
    restore: None,
});

/// Window shadows: apply the configured values once, restore the user's
/// own when the tweak is off. One `hl.config` call per change (a single
/// fork); the capture costs one tiny read per option, once.
fn sync_shadows(panel: &config::PanelConfig) {
    let Some(cfg) = panel
        .tweaks
        .as_ref()
        .and_then(|t| t.shadows.as_ref())
        .filter(|s| s.enabled)
    else {
        restore_shadows();
        return;
    };
    let wanted = ShadowValues::from_config(cfg);
    let mut state = SHADOW_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.restore.is_none() {
        // A failed capture (no hyprctl, mid-reload) is retried next tick
        // instead of making the restore impossible.
        state.restore = read_shadows();
        if state.restore.is_none() {
            return;
        }
    }
    if state.applied.as_ref() == Some(&wanted) {
        return;
    }
    if apply_shadows(&wanted) {
        state.applied = Some(wanted);
    }
}

fn restore_shadows() {
    let mut state = SHADOW_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.applied.take().is_none() {
        return;
    }
    if let Some(own) = state.restore.take() {
        apply_shadows(&own);
    }
}

/// The user's current `decoration:shadow:*` values (`None` if any read
/// fails: a partial capture must never be restored over the real ones).
fn read_shadows() -> Option<ShadowValues> {
    let option = |name: &str| -> Option<serde_json::Value> {
        let path = format!("decoration:shadow:{name}");
        let out = std::process::Command::new("hyprctl")
            .args(["-j", "getoption", &path])
            .output()
            .ok()?;
        serde_json::from_slice(&out.stdout).ok()
    };
    let vec2 = option("offset")?;
    let offset = vec2.get("vec2")?.as_array()?;
    Some(ShadowValues {
        enabled: option("enabled")?.get("bool")?.as_bool()?,
        range: option("range")?.get("int")?.as_i64()? as i32,
        render_power: option("render_power")?.get("int")?.as_i64()? as i32,
        sharp: option("sharp")?.get("bool")?.as_bool()?,
        color: option("color")?.get("gradient")?.as_str()?.to_string(),
        color_inactive: option("color_inactive")?
            .get("gradient")?
            .as_str()?
            .to_string(),
        offset: [
            offset.first()?.as_i64()? as i32,
            offset.get(1)?.as_i64()? as i32,
        ],
        scale: option("scale")?.get("float")?.as_f64()? as f32,
    })
}

/// Write the whole shadow block in one Lua config call.
fn apply_shadows(v: &ShadowValues) -> bool {
    let call = format!(
        "hl.config({{ decoration = {{ shadow = {{ enabled = {}, range = {}, \
         render_power = {}, sharp = {}, color = {}, color_inactive = {}, \
         offset = {{ {}, {} }}, scale = {} }} }} }})",
        v.enabled,
        v.range,
        v.render_power,
        v.sharp,
        gradient_lua(&v.color),
        gradient_lua(&v.color_inactive),
        v.offset[0],
        v.offset[1],
        v.scale
    );
    // `output()` over `status()`: the reply ("ok") is not worth a line of
    // the bar's log on every change.
    std::process::Command::new("hyprctl")
        .args(["eval", &call])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// `#rrggbbaa` (the config's order) → the gradient form Hyprland reports
/// (AARRGGBB stops, the order `gradient_lua` reads back).
fn gradient_from_hex(color: &str) -> String {
    match config::parse_color(color) {
        Ok(c) => {
            let byte = |v: f32| (v * 255.0).round() as u8;
            format!(
                "{:02x}{:02x}{:02x}{:02x} 0deg",
                byte(c.a),
                byte(c.r),
                byte(c.g),
                byte(c.b)
            )
        }
        // Validation already refused this: fall back to Hyprland's own
        // default rather than writing a broken colour.
        Err(_) => "99000000 0deg".to_string(),
    }
}

/// `getoption`'s gradient data (`"99000000 0deg"`, AARRGGBB stops) → the
/// value the Lua config takes: one `rgba()` for a plain colour, a gradient
/// table for a multi-stop one.
fn gradient_lua(data: &str) -> String {
    let mut tokens: Vec<&str> = data.split_whitespace().collect();
    let angle = tokens
        .last()
        .and_then(|last| last.strip_suffix("deg"))
        .and_then(|a| a.parse::<i32>().ok());
    if angle.is_some() {
        tokens.pop();
    }
    let stops: Vec<String> = tokens
        .iter()
        .filter_map(|stop| u32::from_str_radix(stop, 16).ok())
        .map(|argb| {
            let (a, r, g, b) = (
                (argb >> 24) & 0xff,
                (argb >> 16) & 0xff,
                (argb >> 8) & 0xff,
                argb & 0xff,
            );
            format!("rgba({r:02x}{g:02x}{b:02x}{a:02x})")
        })
        .collect();
    match (stops.len(), angle) {
        // A Lua expression: the caller pastes it as the option's value.
        (1, _) => format!("\"{}\"", stops[0]),
        (0, _) => "\"rgba(00000099)\"".to_string(),
        _ => format!(
            "{{ colors = {{ {} }}, angle = {} }}",
            stops
                .iter()
                .map(|stop| format!("\"{stop}\""))
                .collect::<Vec<_>>()
                .join(", "),
            angle.unwrap_or(0)
        ),
    }
}


// ── borders ─────────────────────────────────────────────────────────

/// `general:border_size` + `general:col.*_border` as the compositor
/// reports them; colours stay in the `getoption` gradient form so a
/// captured value round-trips unchanged.
#[derive(Clone, PartialEq)]
struct BorderValues {
    size: i32,
    active: String,
    inactive: String,
}

impl BorderValues {
    fn from_config(cfg: &config::BordersConfig) -> Self {
        Self {
            size: cfg.size,
            active: gradient_from_hex(&cfg.active),
            inactive: gradient_from_hex(&cfg.inactive),
        }
    }
}

#[derive(Default)]
struct BorderState {
    applied: Option<BorderValues>,
    restore: Option<BorderValues>,
}

static BORDER_STATE: Mutex<BorderState> = Mutex::new(BorderState {
    applied: None,
    restore: None,
});

/// Window borders: apply once, restore the user's own when the tweak is
/// off (same contract as the shadows).
fn sync_borders(panel: &config::PanelConfig) {
    let Some(cfg) = panel
        .tweaks
        .as_ref()
        .and_then(|t| t.borders.as_ref())
        .filter(|b| b.enabled)
    else {
        restore_borders();
        return;
    };
    let wanted = BorderValues::from_config(cfg);
    let mut state = BORDER_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.restore.is_none() {
        state.restore = read_borders();
        if state.restore.is_none() {
            return;
        }
    }
    if state.applied.as_ref() == Some(&wanted) {
        return;
    }
    if apply_borders(&wanted) {
        state.applied = Some(wanted);
    }
}

fn restore_borders() {
    let mut state = BORDER_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.applied.take().is_none() {
        return;
    }
    if let Some(own) = state.restore.take() {
        apply_borders(&own);
    }
}

/// The user's current border values (`None` if any read fails: a partial
/// capture must never be restored over the real ones).
fn read_borders() -> Option<BorderValues> {
    let gradient = |name: &str| -> Option<String> {
        let path = format!("general:{name}");
        let out = std::process::Command::new("hyprctl")
            .args(["-j", "getoption", &path])
            .output()
            .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        Some(value.get("gradient")?.as_str()?.to_string())
    };
    let size = std::process::Command::new("hyprctl")
        .args(["-j", "getoption", "general:border_size"])
        .output()
        .ok()?;
    let size: serde_json::Value = serde_json::from_slice(&size.stdout).ok()?;
    Some(BorderValues {
        size: size.get("int")?.as_i64()? as i32,
        active: gradient("col.active_border")?,
        inactive: gradient("col.inactive_border")?,
    })
}

/// Write the border block in one Lua config call.
fn apply_borders(v: &BorderValues) -> bool {
    let call = format!(
        "hl.config({{ general = {{ border_size = {}, col = {{ active_border = {}, \
         inactive_border = {} }} }} }})",
        v.size,
        gradient_lua(&v.active),
        gradient_lua(&v.inactive)
    );
    std::process::Command::new("hyprctl")
        .args(["eval", &call])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::{css, gap_for, gaps_for, gradient_from_hex, gradient_lua, window_count};
    use crate::windows::WinRow;

    /// The gap counts windows, not the floating popup menus X11 apps
    /// report as clients with no title (Steam's "Community" menu opened
    /// as a 102×100 floating `steam` client and shrank the gaps).
    #[test]
    fn window_count_skips_special_workspaces_and_popup_menus() {
        let row = |title: &str, ws: &str, floating: bool| WinRow {
            address: "0x1".to_string(),
            class: "steam".to_string(),
            title: title.to_string(),
            workspace: ws.to_string(),
            focused: false,
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            floating,
            fullscreen: false,
            visible: true,
            pinned: false,
        };
        let clients = vec![
            row("Steam", "1", false),
            row("", "1", true),             // Steam's popup menu
            row("notes", "1", true),        // a real floating window
            row("scratch", "special:magic", false),
            row("", "2", false),            // tiled and untitled: still a window
        ];
        assert_eq!(window_count(&clients), 3, "two windows + the untitled tile");
        assert_eq!(window_count(&[]), 0);
    }

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

    /// The config's `#rrggbbaa` becomes the compositor's gradient form,
    /// and that form becomes the `rgba()` the Lua config takes: a round
    /// trip that keeps the alpha.
    #[test]
    fn shadow_colours_round_trip_through_hyprland() {
        assert_eq!(gradient_from_hex("#00000099"), "99000000 0deg");
        assert_eq!(gradient_lua("99000000 0deg"), "\"rgba(00000099)\"");
        assert_eq!(gradient_from_hex("#ff8000ff"), "ffff8000 0deg");
        assert_eq!(gradient_lua("ffff8000 0deg"), "\"rgba(ff8000ff)\"");
        // A multi-stop gradient keeps every stop and its angle.
        assert_eq!(
            gradient_lua("11223344 55667788 45deg"),
            "{ colors = { \"rgba(22334411)\", \"rgba(66778855)\" }, angle = 45 }"
        );
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
