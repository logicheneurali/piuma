//! Sensor data application: the hot-reloaded config, the async read pass
//! and the merge that fills the bar's sections.

use std::time::{Instant, SystemTime};

use crate::styles::restyle;
use crate::popup_calendar::reap_abandoned_calendar;
use crate::popup_graph::reap_abandoned_popup;
use crate::popup_notifications::sync_popup;
use crate::text_util::value_font;
use crate::{
    config, dashboard, effects, icons, icons_theme, layouts, osd, popup_anim, sensor_runner,
    notifications, panel_thickness, sensors, text_width, themes, tray, tweaks, windows, workspace,
    Command,
    Message, Panel, SectionData, SensorValue, CONFIG_PATH, DEFAULT_FONT_SIZE,
};

/// State of the watched config file: `(modified time, length)`.
/// `None` when no path is set or the file is missing/unreadable.
pub(crate) fn config_file_state() -> Option<(SystemTime, u64)> {
    let path = CONFIG_PATH.get()?.clone();
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Hot reload of the config file: returns `(parsed config, config path)`
/// only when the file changed on disk since the last attempt (mtime +
/// length check, so an unchanged file is not re-read every tick). The
/// attempt is recorded regardless of the outcome; a parse failure (e.g.
/// a half-written file) logs once per change and keeps the previous
/// config, retrying when the file changes again. The path rides along so
/// callers do not touch `CONFIG_PATH` again (one static ref per reload).
/// Rebuild the effective config from the user's saved copy and re-apply
/// the active theme over it, then re-resolve every colour chain. `None`
/// as the active theme restores the user's own colours.
pub(crate) fn apply_active_theme(panel: &mut Panel) {
    panel.config = panel.config_base.clone();
    if let Some(slug) = panel.active_theme {
        themes::apply(&mut panel.config, slug);
    }
    restyle(panel);
}

pub(crate) fn maybe_reload(panel: &mut Panel) -> Option<Reload> {
    let state = config_file_state()?;
    if panel.last_reload == Some(state) {
        return None;
    }
    let path = CONFIG_PATH.get()?.clone();
    panel.last_reload = Some(state);

    let contents = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<config::Config>(&contents) {
        Ok(cfg) => {
            eprintln!("[piuma] config reloaded: {}", path);
            // Same value report as at startup: a reload can bring in an
            // invalid value, and the log must name it. The toast says it
            // too — a hand edit must not be silently ignored.
            let errors = config::report_invalid(&cfg);
            Some(Reload::Applied(Box::new(cfg), path, errors))
        }
        Err(e) => {
            eprintln!("[piuma] config changed but failed to parse ({}), keeping previous", e);
            Some(Reload::Failed(e.to_string()))
        }
    }
}

/// Outcome of a config-file change between two ticks: the bar applies a
/// parsed file, keeps the previous one on a parse error.
pub(crate) enum Reload {
    /// The new base config, the file path and the values piuma ignores.
    Applied(Box<config::Config>, String, Vec<config::ConfigError>),
    /// The file did not parse: the previous config stays.
    Failed(String),
}

// ── update ──────────────────────────────────────────────────────────

/// Applies a message to the panel. `timed`'s `Instant` parameter was dropped
/// in the migration to the `daemon` build pattern (multi-surface), and the
/// `Tick` payload with it: the tick only marks the second edge.
/// Spawn the tick's data pass: the shared compositor snapshot plus
/// every sensor's reading runs on a worker thread, and its reply
/// pairs them with the resolved styles (`SensorsReady` →
/// `apply_read`). One pass at a time (`read_pending`): replies stay
/// ordered and a slow native sensor delays only the next rebuild, never
/// the UI thread. User commands do not run here: `sensor_runner`
/// refreshes them on their own threads and the pass only reads the
/// cache, so a hung command cannot stretch the tick. The config
/// generation rides along so a reply built against a reloaded config is
/// dropped instead of misaligned.
pub(crate) fn spawn_read(panel: &mut Panel) -> Command<Message> {
    panel.read_pending = true;
    panel.read_started = Some(Instant::now());
    panel.read_pass += 1;
    let pass = panel.read_pass;
    let cfg_gen = panel.cfg_gen;
    let sources: Vec<config::SensorSource> = panel
        .config
        .panel
        .sections
        .iter()
        .flat_map(|sec| &sec.sensors)
        .map(|s| s.source.clone())
        .collect();
    // Dynamic gaps count the open windows every pass; off = no snapshot
    // unless a configured sensor already needs it.
    let dynamic_gaps = panel
        .config
        .panel
        .tweaks
        .as_ref()
        .and_then(|t| t.dynamic_gaps.as_ref())
        .is_some_and(|g| g.enabled);
    let want_dashboard = panel.dashboard.is_some();
    Command::perform(
        smol::unblock(move || sensor_runner::read_all(&sources, dynamic_gaps, want_dashboard)),
        move |reads| Message::SensorsReady(cfg_gen, pass, reads),
    )
}

/// Pair one data pass with the resolved styles and run the downstream
/// work the tick used to do inline: graph rings, icon fits, popup
/// reaping and the popup/link sync against fresh sections. Returns
/// the window commands the caller batches.
/// The OSD label for the readings it follows.
fn osd_label(source: &config::SensorSource) -> Option<&'static str> {
    match source {
        config::SensorSource::PercVolume { .. } | config::SensorSource::VolumeMeter { .. } => {
            Some("Volume")
        }
        config::SensorSource::PercBacklight {} => Some("Brightness"),
        _ => None,
    }
}

pub(crate) fn apply_read(panel: &mut Panel, data: sensor_runner::Reads) -> Vec<Command<Message>> {
    let sensor_runner::Reads {
        hypr,
        values,
        dashboard,
    } = data;
    // Dynamic gaps first: the layout pass below measures the tiling the
    // new gap produces, never one it invalidates right after.
    tweaks::sync(
        &panel.config.panel,
        hypr.as_ref().and_then(|h| h.clients.as_deref()),
    );
    // ── Layout engine (roadmap layout-automatici) ──
    // One shared text for every `layout` sensor, computed here from the
    // engine's own state (the runner knows nothing about it). The
    // activation check — matching the window set against the saved
    // layouts and applying the hit once — runs only when a window was
    // created or closed since the last pass; the other passes just
    // report the cached state. The file itself is read once per
    // session (see `LayoutState`).
    let layout_text = match (
        panel
            .config
            .panel
            .sections
            .iter()
            .flat_map(|sec| &sec.sensors)
            .any(|s| matches!(s.source, config::SensorSource::Layout {})),
        hypr.as_ref().and_then(|h| h.clients.as_deref()),
    ) {
        (true, Some(rows)) => Some(layouts::tick(&mut panel.layout_state, rows)),
        _ => None,
    };
    // One reading per configured sensor, consumed in config order.
    let mut reads = values.into_iter();
    // Samples of this pass for the graph history (one read per
    // sensor, collected here so the sections and the history never
    // borrow the panel at once).
    let mut graph_samples: Vec<((usize, usize), f64)> = Vec::new();
    // Volume/brightness changes for the on-screen display (the last one
    // of the pass wins).
    let mut osd_updates: Vec<(&'static str, String)> = Vec::new();
    let effect_mode = panel.config.panel.value_effect.as_deref().unwrap_or("off");
    let effect_enabled = matches!(effect_mode, effects::UNDERLINE | effects::SHEEN);
    let mut effect_starts: Vec<(usize, usize)> = Vec::new();
    let mut sections: Vec<SectionData> = Vec::new();
    // The styles are config-driven and already resolved (`restyle`,
    // on load/reload/palette change): this pass only pairs its
    // readings with them — no colour parsing, no shape strings per
    // sensor per second. The zip stops at the shorter side, so a
    // stale style list can never panic.
    for (section_idx, (section_cfg, section_style)) in panel
        .config
        .panel
        .sections
        .iter()
        .zip(&panel.styles)
        .enumerate()
    {
        let sensors: Vec<SensorValue> = section_cfg
            .sensors
            .iter()
            .zip(&section_style.sensors)
            .enumerate()
            .map(|(sensor_idx, (s, st))| {
                // In config order — the same flatten `spawn_read`
                // cloned the sources from; `missing` covers a reply
                // shorter than the sensor list.
                let read = reads.next().unwrap_or_else(sensor_runner::Read::missing);
                // Graph engine: record this pass's sample for the
                // sensors that opted in (`draw_graph` on a
                // graphable source); the columns ignore the rest.
                if s.draw_graph || config::net_auto_peak(s) {
                    if let Some((sample, _)) = read.graph {
                        graph_samples.push(((section_idx, sensor_idx), sample));
                    }
                }
                let mut value = read.text;
                // The Wi-Fi icon follows this pass's signal; a static
                // icon configured on the sensor still wins.
                let wifi_icon = matches!(s.source, config::SensorSource::Wifi {})
                    .then(|| {
                        let name =
                            icons::wifi_signal_icon(read.graph.map(|(value, _)| value as f32));
                        icons_theme(&panel.config)
                            .and_then(|theme| icons::resolve(theme, name, st.icon_size))
                    })
                    .flatten();
                // The OSD follows the real changes of the volume and
                // brightness sliders, whatever moved them.
                if let Some(label) = osd_label(&s.source) {
                    let old = panel
                        .sections
                        .get(section_idx)
                        .and_then(|section| section.sensors.get(sensor_idx))
                        .map(|sensor| sensor.value.clone());
                    if old
                        .as_deref()
                        .is_some_and(|old| !old.is_empty() && old != value)
                    {
                        osd_updates.push((label, value.clone()));
                    }
                }
                // A caffeine sensor mirrors the panel toggle: the runner
                // cannot see it.
                if matches!(s.source, config::SensorSource::Caffeine {}) {
                    value = if panel.caffeine { "on" } else { "off" }.to_string();
                }
                // The theme sensor names the active theme (or Custom);
                // only the panel knows it.
                if matches!(s.source, config::SensorSource::Theme {}) {
                    value = panel
                        .active_theme
                        .and_then(themes::find)
                        .map(|theme| theme.name.clone())
                        .unwrap_or_else(|| "Custom".to_string());
                }
                // A layout sensor shows the engine's state (✓ saved /
                // ✓ applied / ⋯ armed / nothing while the file is
                // absent) instead of any reading.
                if matches!(s.source, config::SensorSource::Layout {}) {
                    if let Some(t) = &layout_text {
                        value = t.clone();
                    }
                }
                // A dragged volume meter has not written the sink yet:
                // keep the local target across this tick, or the bar
                // would snap back to the old reading every second.
                if let Some((drag_section, drag_sensor, percent)) = panel.volume_drag {
                    if (drag_section, drag_sensor) == (section_idx, sensor_idx) {
                        value = format!("{percent:.0}%");
                    }
                }
                // Tray sensors render icons from the shared tray state
                // (tray.rs) instead of text.
                let tray_icons =
                    matches!(s.source, config::SensorSource::Tray {}).then(tray::items);
                // Workspace sensors build their button list from
                // the shared snapshot (roadmap2 #4): one entry per
                // available workspace, the active one flagged.
                let workspace_buttons = matches!(s.source, config::SensorSource::Workspace {})
                    .then(|| workspace::buttons(hypr.as_ref()));
                // Taskbar sensors rebuild their icon slots from the
                // shared snapshot per pass; the icons resolve
                // through the memoized theme lookups.
                let taskbar_icons = matches!(s.source, config::SensorSource::Taskbar {})
                    .then(|| windows::task_icons(hypr.as_ref()));
                // The grid's fraction of its reference: the value itself
                // for percentages, the reading's own used/total for the
                // `used_*` families, the rate over `scale_max` (or the
                // recent peak) for the network rates. The peak is the
                // previous pass's history — one tick behind, and clamped
                // below anyway.
                let level: Option<f32> = match &s.source {
                    config::SensorSource::PercCpu {}
                    | config::SensorSource::PercGpu {}
                    | config::SensorSource::PercMem {}
                    | config::SensorSource::PercSwap {}
                    | config::SensorSource::PercDisk {}
                    | config::SensorSource::PercBattery {}
                    | config::SensorSource::PercBacklight {}
                    | config::SensorSource::PercVolume { .. } => {
                        sensors::leading_number(&value).map(|pct| (pct / 100.0).clamp(0.0, 1.0))
                    }
                    config::SensorSource::UsedMem {}
                    | config::SensorSource::UsedSwap {}
                    | config::SensorSource::UsedDisk {}
                    | config::SensorSource::UsedBattery {} => read
                        .graph
                        .map(|(percent, _)| (percent as f32 / 100.0).clamp(0.0, 1.0)),
                    config::SensorSource::TempCpu {}
                    | config::SensorSource::TempGpu {} => {
                        read.graph.map(|(temp, _)| temp_level(temp))
                    }
                    config::SensorSource::UsedNetdown { .. }
                    | config::SensorSource::UsedNetup { .. } => {
                        // Auto peak ignores idle noise: a peak below
                        // the floor is no reference at all (the grid
                        // stays empty), or a quiet interface would show
                        // a full grid just because that is the busiest
                        // it has been.
                        let reference = config::parse_rate_max(
                            s.scale_max.as_deref().unwrap_or_default(),
                        )
                        .or_else(|| {
                            panel
                                .graphs
                                .peak(section_idx, sensor_idx)
                                .map(f64::from)
                                .filter(|peak| *peak >= AUTO_PEAK_MIN)
                        });
                        rate_level(read.graph.map(|(rate, _)| rate), reference)
                    }
                    _ => None,
                };
                // Only the net rate sensors pin their value box
                // (roadmap M5), and the pin is the displayed value's
                // OWN tier: exact width (no headroom for units this
                // state never shows), constant while the value moves
                // inside the tier (figure spaces), moving with it on
                // a ×1024 crossing — same pass, same string, so the
                // box can never undershoot its content. Computed
                // before the literal because `value` is moved into
                // it; the measurement is cached per
                // font/size/patterns, so the per-pass rebuild only
                // re-measures when the tier label changes.
                let fixed_width = matches!(
                    &s.source,
                    config::SensorSource::UsedNetup { .. }
                        | config::SensorSource::UsedNetdown { .. }
                )
                .then(|| {
                    let font = value_font(&panel.config);
                    let size = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
                    text_width::widest(&font, size, &sensors::net_rate_samples(&value))
                });
                if effect_enabled && s.effect {
                    let old_value = panel
                        .sections
                        .get(section_idx)
                        .and_then(|section| section.sensors.get(sensor_idx))
                        .map(|sensor| sensor.value.as_str());
                    if effects::value_changed(old_value, &value) {
                        effect_starts.push((section_idx, sensor_idx));
                    }
                }
                SensorValue {
                    name: st.name.clone(),
                    description: st.description.clone(),
                    value,
                    effect: s.effect,
                    text_color: Some(st.text_color),
                    color_rules: st.color_rules.clone(),
                    bg_color: st.bg_color,
                    text_color_hover: st.text_color_hover,
                    bg_color_hover: st.bg_color_hover,
                    border_color: st.border_color,
                    border_width: st.border_width,
                    border_radius: st.border_radius,
                    border_type: st.border_type.clone(),
                    padding: st.padding,
                    action: st.action.clone(),
                    tray: tray_icons,
                    workspaces: workspace_buttons,
                    taskbar: taskbar_icons,
                    media: read.media,
                    value_display: crate::panel::ValueDisplay::from_config(
                        s.value_display.as_deref(),
                    ),
                    level,
                    is_notifications: st.is_notifications,
                    icon: st.icon.clone().or(wifi_icon),
                    icon_size: st.icon_size,
                    fixed_width,
                }
            })
            .collect();

        sections.push(SectionData {
            name: section_style.name.clone(),
            layout: section_style.layout.clone(),
            bg_color: section_style.bg_color,
            text_color: section_style.text_color,
            title_color: section_style.title_color,
            border_color: section_style.border_color,
            border_width: section_style.border_width,
            border_radius: section_style.border_radius,
            border_type: section_style.border_type.clone(),
            sensors,
        });
    }
    // The next click on a layout sensor saves from this snapshot — no
    // fork, and exactly what the engine just matched against.
    panel.hypr = hypr;
    panel.sections = sections;
    // Restart the effect of every value that really changed this pass;
    // expired entries go away so the frame subscription can stop.
    let now = Instant::now();
    panel
        .value_effects
        .retain(|_, start| now.saturating_duration_since(*start) < effects::DURATION);
    if effect_enabled {
        for key in effect_starts {
            panel.value_effects.insert(key, now);
        }
    } else {
        panel.value_effects.clear();
    }
    // One ring advance per pass: columns repeat their last value
    // when a reading was missing (time stays uniform).
    panel.graphs.record(&graph_samples);
    // Icons size to the space their sensor box really has, capped by the
    // font; both the view and the icon resolution read the targets.
    update_icon_targets(panel);

    let mut cmds: Vec<Command<Message>> = Vec::new();
    // The dashboard's media card: refresh the track text every pass, and
    // decode the cover once per art URL (never per tick, never from the
    // network).
    if panel.dashboard.is_some() {
        let art = dashboard
            .as_ref()
            .and_then(|probe| probe.media.as_ref())
            .and_then(|media| media.art.clone());
        panel.dash_probe = dashboard;
        // The probe is the truth: drop the optimistic hints unless a
        // slider drag is still in flight.
        panel.dash_profile = None;
        if !panel.dash_volume_drag {
            panel.dash_volume = None;
        }
        if !panel.dash_brightness_drag {
            panel.dash_brightness = None;
        }
        if art != panel.cover_url {
            panel.cover_url = art.clone();
            panel.cover = None;
            if let Some(url) = art {
                let task_url = url.clone();
                cmds.push(Command::perform(
                    smol::unblock(move || dashboard::cover(&task_url)),
                    move |handle| Message::DashboardCover(url, handle),
                ));
            }
        }
    }
    if let Some((label, value)) = osd_updates.pop() {
        let fraction = sensors::leading_number(&value)
            .map(|v| (v / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        cmds.push(osd::show(panel, label, &value, fraction));
    }
    // A popup that lost both its sensor and its pointer gets
    // reaped once its grace runs out: the sensor's leave can act
    // on stale coordinates and, after the cursor rests on another
    // application's window, no event ever arrives again.
    if let Some(gp) = reap_abandoned_popup(panel) {
        cmds.push(popup_anim::close(panel, gp.id, popup_anim::Ghost::Graph(gp)));
    }
    // The calendar popup's three-second grace runs on the same
    // pass: neither sensor nor surface under the cursor spends it.
    if let Some(cp) = reap_abandoned_calendar(panel) {
        cmds.push(popup_anim::close(
            panel,
            cp.id,
            popup_anim::Ghost::Calendar(cp),
        ));
    }
    // The popup follows the active set (and re-anchors with the bar
    // on a geometry change) — one place drives its window.
    let resync = std::mem::take(&mut panel.popup_resync);
    if let Some(cmd) = sync_popup(panel, resync) {
        cmds.push(cmd);
    }
    cmds
}

/// Recompute the icon edge SHARED by both icon rows from the built
/// sections (every tick and on every reload): the panel font's line
/// height plus the sensor's own padding on both sides (`font_icon_edge` —
/// a tray/taskbar box drops that inset on the thickness axis so its row
/// fills the height, and the icon spends it: font 12 + padding 4 → 22),
/// capped by what a sensor row physically fits on the cross axis
/// (`row_icon_target` with zero padding — taskbar and tray icons fill the
/// whole row, the view draws these boxes without the cross-axis inset).
/// One number — the smallest fit across every tray and taskbar sensor —
/// is written to both rows, so tray and taskbar icons are always the same
/// size in both orientations. `panel.icon_size` replaces that auto fit
/// when set (still capped by the row). With no such sensor at all the
/// target is left alone.
pub(crate) fn update_icon_targets(panel: &Panel) {
    let thickness = panel_thickness(&panel.config);
    let font = panel.config.panel.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let rooms = crate::geometry::section_icon_room(&panel.config, font);
    let mut shared: Option<u16> = None;
    for (section_idx, section) in panel.sections.iter().enumerate() {
        // Title height as the view draws it, shared with the sensor icon
        // fit: text in a box padded by the section's content inset.
        let room = rooms
            .get(section_idx)
            .copied()
            .unwrap_or(crate::geometry::IconRoom {
                title_extra: 0,
                pad: panel.config.panel.section_content_pad(false),
            });
        for sensor in &section.sensors {
            if sensor.tray.is_none() && sensor.taskbar.is_none() {
                continue;
            }
            let auto = icons::font_icon_edge(font, u32::from(sensor.padding)).min(
                icons::row_icon_target(thickness, u32::from(room.pad), room.title_extra, 0),
            );
            let edge = icons::icon_edge(panel.config.panel.icon_size, auto);
            shared = Some(shared.map_or(edge, |m| m.min(edge)));
        }
    }
    if let Some(edge) = shared {
        tray::set_icon_target(edge);
        windows::set_icon_target(edge);
    }
}

/// Tray and notification events refresh only their own sensors: the
/// slots and the feed text come from the shared in-memory state and
/// every other reading stays what the last tick read — a chatty tray
/// icon must not re-fork (and re-read) the whole sensor set. The
/// tint follows in the view: colour rules are evaluated per render.
pub(crate) fn refresh_tray_and_notif(panel: &mut Panel) {
    for section in &mut panel.sections {
        for sensor in &mut section.sensors {
            if sensor.tray.is_some() {
                sensor.tray = Some(tray::items());
            }
            if sensor.is_notifications {
                sensor.value = notifications::badge_value();
            }
        }
    }
}

/// A temperature as a grid level: one square per 10 °C (a conventional
/// 0–100 scale, clamped — the colour rules still say how hot it is).
fn temp_level(temp: f64) -> f32 {
    (temp / 100.0).clamp(0.0, 1.0) as f32
}

/// The auto peak ignores an idle interface: below this rate (bytes/s)
/// the history holds noise, not a reference the grid should scale to.
const AUTO_PEAK_MIN: f64 = 10_000.0;

/// The grid level for a network rate. `None` only when the reading is
/// missing (`N/A`); a zero rate — or a reference not known yet while the
/// auto peak fills its window — is a real level of 0, so a grid-only
/// sensor keeps the grid instead of flashing its "0.0 B" text.
fn rate_level(rate: Option<f64>, reference: Option<f64>) -> Option<f32> {
    let rate = rate?;
    match reference.filter(|reference| *reference > 0.0) {
        Some(reference) if rate.is_finite() && rate > 0.0 => {
            Some((rate / reference).clamp(0.0, 1.0) as f32)
        }
        _ => Some(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::{rate_level, temp_level};

    #[test]
    fn a_temperature_maps_ten_degrees_per_square() {
        assert_eq!(temp_level(0.0), 0.0);
        assert_eq!(temp_level(61.0), 0.61);
        assert_eq!(temp_level(75.0), 0.75);
        assert_eq!(temp_level(100.0), 1.0);
        assert_eq!(temp_level(120.0), 1.0, "clamped above boiling");
    }

    #[test]
    fn a_zero_rate_is_an_empty_grid_not_missing_data() {
        assert_eq!(rate_level(None, Some(1024.0)), None, "N/A keeps the text");
        assert_eq!(rate_level(Some(0.0), Some(1024.0)), Some(0.0));
        assert_eq!(rate_level(Some(0.0), None), Some(0.0), "peak not ready yet");
        assert_eq!(rate_level(Some(2048.0), Some(4096.0)), Some(0.5));
        assert_eq!(rate_level(Some(9000.0), Some(4096.0)), Some(1.0));
        assert_eq!(rate_level(Some(2048.0), None), Some(0.0));
    }
}
