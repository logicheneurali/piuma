//! The Wallpaper tab's Hyprland switch: turn the compositor's built-in
//! 8K mascot wallpaper/logo off (or back on) in its own config.
//!
//! The editor is deliberately narrow: it changes only
//! `force_default_wallpaper` and `disable_hyprland_logo` in the existing
//! `misc` table, preserving indentation, commas and comments. When those
//! keys are absent it appends a marked block at the end; that block is
//! removed again when the switch goes back to off.

use std::fs;
use std::path::{Path, PathBuf};

const LUA_BEGIN: &str = "-- piuma: hyprland wallpaper/logo (managed)";
const LUA_END: &str = "-- piuma: end hyprland wallpaper/logo";
const CONF_BEGIN: &str = "# piuma: hyprland wallpaper/logo (managed)";
const CONF_END: &str = "# piuma: end hyprland wallpaper/logo";

fn config_path() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("hypr");
    for name in ["hyprland.lua", "hyprland.conf"] {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn is_lua(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("lua")
}

/// Current switch state, read from the file's last literal values.
pub(crate) fn read_disabled() -> Option<bool> {
    let path = config_path()?;
    let text = fs::read_to_string(path).ok()?;
    Some(disabled_in(&text))
}

/// Write the switch state and reload Hyprland. The config write is the
/// source of truth; a reload failure (e.g. setup outside a session) is
/// only a status warning, not a reason to undo the file.
pub(crate) fn set_disabled(disabled: bool) -> Result<(), String> {
    let path = config_path()
        .ok_or_else(|| "Hyprland config not found (hyprland.lua / hyprland.conf)".to_string())?;
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let updated = apply(&text, disabled, is_lua(&path));
    if updated != text {
        let tmp = path.with_extension(format!(
            "{}.piuma.tmp",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        fs::write(&tmp, updated).map_err(|e| format!("{}: {e}", tmp.display()))?;
        fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let _ = std::process::Command::new("hyprctl").arg("reload").status();
    Ok(())
}

/// Read the last literal values in the file (later assignments win, as
/// in both Lua and hyprlang). Missing keys mean Hyprland's defaults:
/// wallpaper/logo on.
fn disabled_in(text: &str) -> bool {
    let mut force = None;
    let mut logo = None;
    for line in text.lines() {
        if let Some(v) = value_of(line, "force_default_wallpaper") {
            force = v.parse::<i32>().ok();
        }
        if let Some(v) = value_of(line, "disable_hyprland_logo") {
            logo = Some(v == "true");
        }
    }
    let force = force.unwrap_or(-1);
    let logo = logo.unwrap_or(false);
    logo && matches!(force, 0 | 1)
}

/// The value token of `key = value` on this line; comments containing
/// the key are not matched.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let body = line.trim_start();
    if body.starts_with("--") || body.starts_with('#') {
        return None;
    }
    let rest = body.strip_prefix(key)?;
    let after_eq = rest.trim_start().strip_prefix('=')?.trim_start();
    let end = after_eq
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || *c == ',' || *c == ';')
        .map(|(i, _)| i)
        .unwrap_or(after_eq.len());
    Some(&after_eq[..end])
}

/// Replace both values, preserving indentation, the comma and the
/// trailing comment; append a marked block only when a key is absent.
fn apply(text: &str, disabled: bool, lua: bool) -> String {
    let force = if disabled { "0" } else { "-1" };
    let logo = if disabled { "true" } else { "false" };
    let stripped = strip_block(text, lua);
    let mut force_seen = false;
    let mut logo_seen = false;
    let mut out = String::new();
    for line in stripped.lines() {
        let mut new = line.to_string();
        if let Some(replaced) = replace_value(line, "force_default_wallpaper", force) {
            force_seen = true;
            new = replaced;
        }
        if let Some(replaced) = replace_value(&new, "disable_hyprland_logo", logo) {
            logo_seen = true;
            new = replaced;
        }
        out.push_str(&new);
        out.push('\n');
    }
    if disabled && !(force_seen && logo_seen) {
        out.push('\n');
        if lua {
            out.push_str(LUA_BEGIN);
            out.push('\n');
            out.push_str("hl.config({\n    misc = {\n");
            out.push_str(&format!("        force_default_wallpaper = {force},\n"));
            out.push_str(&format!("        disable_hyprland_logo = {logo},\n"));
            out.push_str("    },\n})\n");
            out.push_str(LUA_END);
            out.push('\n');
        } else {
            out.push_str(CONF_BEGIN);
            out.push('\n');
            out.push_str("misc {\n");
            out.push_str(&format!("    force_default_wallpaper = {force}\n"));
            out.push_str(&format!("    disable_hyprland_logo = {logo}\n"));
            out.push_str("}\n");
            out.push_str(CONF_END);
            out.push('\n');
        }
    }
    out
}

fn replace_value(line: &str, key: &str, value: &str) -> Option<String> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, body) = line.split_at(indent_len);
    if body.starts_with("--") || body.starts_with('#') {
        return None;
    }
    let rest = body.strip_prefix(key)?;
    if !rest.trim_start().starts_with('=') {
        return None;
    }
    let (before_eq, after_eq) = rest.split_once('=')?;
    let after_trim = after_eq.trim_start();
    let value_start = after_eq.len() - after_trim.len();
    let end = after_trim
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || *c == ',' || *c == ';')
        .map(|(i, _)| i)
        .unwrap_or(after_trim.len());
    Some(format!(
        "{indent}{key}{before_eq}={}{value}{}",
        &after_eq[..value_start],
        &after_trim[end..]
    ))
}

fn strip_block(text: &str, lua: bool) -> String {
    let (begin, end) = if lua { (LUA_BEGIN, LUA_END) } else { (CONF_BEGIN, CONF_END) };
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim() == begin {
            inside = true;
            continue;
        }
        if inside {
            if line.trim() == end {
                inside = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUA: &str = "\
hl.config({
\tmisc = {
\t\tforce_default_wallpaper = -1, -- mascot
\t\tdisable_hyprland_logo = false, -- logo
\t},
})
";

    #[test]
    fn toggles_both_values_and_keeps_comments() {
        assert!(!disabled_in(LUA));
        let on = apply(LUA, true, true);
        assert!(on.contains("force_default_wallpaper = 0, -- mascot"));
        assert!(on.contains("disable_hyprland_logo = true, -- logo"));
        assert!(disabled_in(&on));
        let off = apply(&on, false, true);
        assert!(off.contains("force_default_wallpaper = -1, -- mascot"));
        assert!(off.contains("disable_hyprland_logo = false, -- logo"));
        assert!(!disabled_in(&off));
    }

    #[test]
    fn missing_keys_get_a_marked_block_that_off_removes() {
        let empty = "hl.config({ general = { gaps_in = 8 } })\n";
        let on = apply(empty, true, true);
        assert!(on.contains(LUA_BEGIN));
        assert!(disabled_in(&on));
        let off = apply(&on, false, true);
        assert!(!off.contains(LUA_BEGIN));
        assert!(!disabled_in(&off));
    }

    #[test]
    fn legacy_conf_is_supported() {
        let conf = "misc {\n    force_default_wallpaper = -1\n    disable_hyprland_logo = false\n}\n";
        let on = apply(conf, true, false);
        assert!(on.contains("force_default_wallpaper = 0\n"));
        assert!(on.contains("disable_hyprland_logo = true\n"));
        assert!(disabled_in(&on));
    }

    #[test]
    fn missing_key_is_filled_by_the_marked_block() {
        let text = "hl.config({ misc = { disable_hyprland_logo = false } })\n";
        let on = apply(text, true, true);
        assert!(on.contains("disable_hyprland_logo = true"));
        assert!(on.contains("force_default_wallpaper = 0"));
        assert!(disabled_in(&on));
    }
}
