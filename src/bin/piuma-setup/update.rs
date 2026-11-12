//! The message loop: one handler per `Message`, mutating the setup state
//! or spawning iced tasks (focus, icon browsing, reloads).
use iced::advanced::widget::Operation;
use iced::widget::operation;
use iced::widget::scrollable::AbsoluteOffset;
use iced::Task;

use crate::autostart;
use crate::config;
use crate::controls::{border_live, content_scroll_id, scroll_target, sensor_card_id, sensor_name_id, sorted_rules};
use crate::modal::{Modal, RuleDraft};
use crate::view_wallpaper::wallpaper_set;
use crate::model::*;
use crate::state::{file_path, list_config_files, sanitize_file_name, Message, PiumaSetup, Tab};

/// Applies one message, then recomputes the invalid-value list so the
/// red fields, the section frames and the save guard follow every edit
/// without needing a save attempt to appear.
pub(crate) fn update(state: &mut PiumaSetup, message: Message) -> Task<Message> {
    let task = update_inner(state, message);
    state.refresh_validation();
    task
}

fn update_inner(state: &mut PiumaSetup, message: Message) -> Task<Message> {
    match message {
        Message::FontChanged(value) => state.config.font = if value.is_empty() { None } else { Some(value) },
        Message::FontSizeChanged(value) => state.config.font_size = value,
        Message::BoldChanged(value) => {
            // On = explicit true; off drops back to absent (≈ false).
            state.config.bold = value.then_some(true);
        }
        Message::IconThemeChanged(value) => {
            state.config.icon_theme = if value.is_empty() { None } else { Some(value) };
        }
        Message::AutostartChanged(value) => {
            // On = explicit true; off drops back to absent (≈ false).
            state.config.autostart = value.then_some(true);
        }
        Message::AutostartAppToggled(file, enabled) => {
            // The selection lives in the config (`panel.autostart_disabled`):
            // the switch rewrites that list and the tab re-reads the
            // effective state; the Save persists it. No autostart file is
            // ever written, so other desktop environments keep their own,
            // unaffected view.
            match autostart::editor::toggle_disabled(
                &mut state.config.autostart_disabled,
                &file,
                enabled,
            ) {
                Ok(()) => {
                    state.autostart_apps =
                        autostart::editor::apps(&state.config.autostart_disabled);
                    let verb = if enabled { "enabled" } else { "disabled" };
                    state.set_status(format!("{} {}", verb, file), false);
                }
                Err(e) => state.set_status(format!("{}: {}", file, e), true),
            }
        }
        Message::AppThemeChanged(value) => {
            // On = explicit true; off drops back to absent (≈ false).
            state.config.app_theme = value.then_some(true);
        }
        Message::SectionTitleChanged(value) => {
            // Unlike app_theme, both states are explicit: the default
            // is titles drawn, so off must write a false.
            state.config.section_title = Some(value);
        }
        Message::ValueEffectChanged(value) => {
            // Absent means off: keep the file minimal when disabled.
            state.config.value_effect = (value != "off").then_some(value);
        }
        Message::ThemeModeChanged(value) => {
            // Absent means dark (the historic default); light is explicit.
            state.config.theme_mode = (value == "light").then_some(value);
        }
        Message::WallpaperEnabled(value) => {
            // The toggle only flips `enabled`: the chosen path survives
            // both directions (the surface opens/closes on save + reload).
            wallpaper_set(&mut state.config, |w| w.enabled = value);
        }
        Message::WallpaperPathChanged(path) => {
            // List pick and typed path share the destination; the bar
            // checks existence itself, so a not-yet-there path is fine.
            wallpaper_set(&mut state.config, |w| w.path = path);
        }
        Message::WallpaperModeChanged(mode) => {
            wallpaper_set(&mut state.config, |w| {
                w.mode = mode;
                // Give the colour mode a usable starting colour; the
                // user can edit it right away.
                if w.mode == "color" && w.color.trim().is_empty() {
                    w.color = "#152040".to_string();
                }
            });
        }
        Message::WallpaperColorChanged(color) => {
            wallpaper_set(&mut state.config, |w| w.color = color);
        }
        Message::HyprWallpaperToggled(disabled) => {
            // This one edits Hyprland's config, not piuma.json, and
            // applies now (hyprctl reload); the wallpaper card's Save
            // does not own it.
            match crate::hyprland_wallpaper::set_disabled(disabled) {
                Ok(()) => {
                    state.hypr_wallpaper_disabled = Some(disabled);
                    state.set_status(
                        if disabled {
                            "Hyprland wallpaper/logo disabled".to_string()
                        } else {
                            "Hyprland wallpaper/logo restored".to_string()
                        },
                        false,
                    );
                }
                Err(err) => state.set_status(err, true),
            }
        }
        Message::PaletteFromWallpaper(value) => {
            // On = explicit true; off drops back to absent (≈ false):
            // a save never injects the key, and the extracted tones
            // never reach the file.
            state.config.wallpaper_palette = value.then_some(true);
        }
        Message::CornersEnabled(value) => {
            // The toggle flips `enable`: colour and radius survive both
            // directions (the masks open/close on save + reload).
            corners_set(&mut state.config, |c| c.enable = value);
        }
        Message::CornersColorChanged(value) => {
            corners_set(&mut state.config, |c| c.color = value);
        }
        Message::CornersRadiusChanged(value) => {
            corners_set(&mut state.config, |c| c.radius = value);
        }
        Message::DynamicGapsEnabled(value) => {
            // The toggle flips `enabled`: min/max survive both directions,
            // and the bar only starts querying Hyprland while it is on.
            dynamic_gaps_mut(&mut state.config).enabled = value;
        }
        Message::WindowMenuEnabled(value) => {
            window_menu_mut(&mut state.config).enabled = value;
        }
        Message::WindowMenuPositionChanged(label) => {
            // The picker offers the bar's eight values; an unknown label
            // falls back to the default instead of writing garbage.
            let key = WINDOW_MENU_POSITIONS
                .iter()
                .find(|(_, option)| *option == label)
                .map(|(key, _)| *key)
                .unwrap_or("top-center");
            window_menu_mut(&mut state.config).position = key.to_string();
        }
        Message::WindowMenuHideIdle(value) => {
            window_menu_mut(&mut state.config).hide_idle = value;
        }
        Message::WindowMenuHideAfter(value) => {
            window_menu_mut(&mut state.config).hide_after = value.clamp(10, 120);
        }
        Message::ShadowsEnabled(value) => {
            // The switch only arms the block: the values survive both ways,
            // and the bar writes nothing while it is off.
            shadows_mut(&mut state.config).enabled = value;
        }
        Message::ShadowRangeChanged(value) => {
            shadows_mut(&mut state.config).range = value.clamp(0, 100);
        }
        Message::ShadowPowerChanged(value) => {
            shadows_mut(&mut state.config).render_power = value.clamp(1, 4);
        }
        Message::ShadowSharpChanged(value) => shadows_mut(&mut state.config).sharp = value,
        Message::ShadowColorChanged(value) => shadows_mut(&mut state.config).color = value,
        Message::ShadowInactiveColorChanged(value) => {
            shadows_mut(&mut state.config).color_inactive = value;
        }
        Message::ShadowOffsetChanged(axis, value) => {
            let shadows = shadows_mut(&mut state.config);
            shadows.offset[axis.min(1)] = value.clamp(-500, 500);
        }
        Message::ShadowScaleChanged(tenths) => {
            shadows_mut(&mut state.config).scale = tenths.clamp(1, 100) as f32 / 10.0;
        }
        Message::BordersEnabled(value) => {
            // First use seeds the block from the compositor's own values:
            // enabling must not repaint the borders until a field is edited.
            let known = state
                .config
                .tweaks
                .as_ref()
                .and_then(|t| t.borders.as_ref())
                .is_some();
            let borders = borders_mut(&mut state.config);
            borders.enabled = value;
            if value && !known {
                if let Some((size, active, inactive)) = border_live() {
                    borders.size = size;
                    borders.active = active;
                    borders.inactive = inactive;
                }
            }
        }
        Message::BorderSizeChanged(value) => {
            borders_mut(&mut state.config).size = value.clamp(0, 20);
        }
        Message::BorderActiveChanged(value) => borders_mut(&mut state.config).active = value,
        Message::BorderInactiveChanged(value) => {
            borders_mut(&mut state.config).inactive = value;
        }
        Message::ScreensaverEnabled(value) => {
            screensaver_mut(&mut state.config).enabled = value;
        }
        Message::ScreensaverMinutesChanged(value) => {
            screensaver_mut(&mut state.config).minutes = value.clamp(1, 120);
        }
        Message::GlassEnabled(value) => {
            glass_mut(&mut state.config).enabled = value;
        }
        Message::GlassOpacityChanged(value) => {
            glass_mut(&mut state.config).opacity =
                value.clamp(5, 100) as f32 / 100.0;
        }
        Message::AutoHideEnabled(value) => {
            auto_hide_mut(&mut state.config).enabled = value;
        }
        Message::ScreensaverShow => {
            // The lock works even with the idle trigger off (the sensor
            // does too): the signal is handled unconditionally.
            match crate::reload::trigger_screensaver() {
                Ok(()) => state.set_status(
                    "Locked — the session password unlocks it".to_string(),
                    false,
                ),
                Err(e) => state.set_status(format!("Lock: {e}"), true),
            }
        }
        Message::GapMinChanged(value) => {
            // The sliders are mutually bounded; clamp here too so a
            // programmatic edit can never store min > max.
            let gaps = dynamic_gaps_mut(&mut state.config);
            let max = gaps.max.clamp(1, 200);
            gaps.min = value.clamp(1, max);
        }
        Message::GapMaxChanged(value) => {
            let gaps = dynamic_gaps_mut(&mut state.config);
            let min = gaps.min.clamp(1, 200);
            gaps.max = value.clamp(min, 200);
        }
        Message::ThemeApplied(slug) => {
            // An unknown slug is custom, exactly like a hand-edited file:
            // never leave a name the bar would silently ignore.
            let chosen = slug.as_deref().and_then(|s| crate::themes::find(s)).map(|t| t.slug);
            match crate::themes::write_active(chosen) {
                Ok(()) => {
                    state.active_theme = chosen;
                    let label = chosen
                        .and_then(crate::themes::find)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| "Custom".to_string());
                    state.set_status(
                        format!("Theme: {label} (saved in theme.json; the bar applies it live)"),
                        false,
                    );
                }
                Err(e) => state.set_status(e, true),
            }
        }
        Message::TabSelected(tab) => {
            // The Layouts tab reads the bar's file on every selection,
            // so layouts saved meanwhile show up without a restart; an
            // unreadable file is named in the status instead of being
            // shown as an empty list.
            if tab == Tab::Layouts {
                match load_layouts() {
                    Ok(list) => state.layouts = list,
                    Err(e) => {
                        state.layouts = Vec::new();
                        state.set_status(e, true);
                    }
                }
            }
            // The theme file may have changed from the bar's sensor.
            if tab == Tab::Themes {
                state.active_theme = crate::themes::active();
            }
            state.active_tab = tab;
            // Wallpaper thumbnails decode once, off the UI thread; the
            // grid then only clones handles.
            if tab == Tab::Wallpaper
                && !state.wallpaper_thumbs_requested
                && !state.wallpaper_files.is_empty()
            {
                state.wallpaper_thumbs_requested = true;
                return wallpaper_thumb_task(state.wallpaper_files.clone());
            }
        }
        Message::WallpaperThumbsLoaded(batch) => {
            state.wallpaper_thumbs.extend(batch);
        }
        Message::ThicknessChanged(value) => state.config.thickness = value,
        Message::DefaultsStyleChanged(field, value) => {
            let d = &mut state.config.sensor_defaults;
            let v = if value.is_empty() { None } else { Some(value) };
            match field.as_str() {
                "text_color" => d.text_color = v,
                "bg_color" => d.bg_color = v,
                "text_color_hover" => d.text_color_hover = v,
                "bg_color_hover" => d.bg_color_hover = v,
                "border_color" => d.border_color = v,
                "border_color_hover" => d.border_color_hover = v,
                "border_type" => d.border_type = v,
                _ => {}
            }
        }
        Message::DefaultsNumberChanged(field, value) => {
            let d = &mut state.config.sensor_defaults;
            // -1 is the "inherit" marker: dropping the key says the same
            // to the bar and keeps the saved file minimal.
            let v = (value >= 0).then_some(value);
            match field.as_str() {
                "border_width" => d.border_width = v,
                "border_width_hover" => d.border_width_hover = v,
                "border_radius" => d.border_radius = v,
                "padding" => d.padding = v,
                _ => {}
            }
        }
        Message::SectionStyleChanged(index, field, value) => {
            if index < state.config.sections.len() {
                // Border style keys; section bg/text keep their own messages.
                let v = if value.is_empty() { None } else { Some(value) };
                let s = &mut state.config.sections[index];
                match field.as_str() {
                    "border_color" => s.border_color = v,
                    "border_type" => s.border_type = v,
                    _ => {}
                }
            }
        }
        Message::SectionCollapsibleChanged(index, value) => {
            if let Some(section) = state.config.sections.get_mut(index) {
                section.collapsible = value;
            }
        }
        Message::SectionNumberChanged(index, field, value) => {
            if index < state.config.sections.len() {
                let s = &mut state.config.sections[index];
                match field.as_str() {
                    "border_width" => s.border_width = (value >= 0).then_some(value),
                    "border_radius" => s.border_radius = (value >= 0).then_some(value),
                    _ => {}
                }
            }
        }
        Message::SecDefaultsStyleChanged(field, value) => {
            let d = &mut state.config.section_defaults;
            let v = if value.is_empty() { None } else { Some(value) };
            match field.as_str() {
                "bg_color" => d.bg_color = v,
                "text_color" => d.text_color = v,
                "border_color" => d.border_color = v,
                "border_type" => d.border_type = v,
                _ => {}
            }
        }
        Message::SecDefaultsNumberChanged(field, value) => {
            let d = &mut state.config.section_defaults;
            match field.as_str() {
                "border_width" => d.border_width = (value >= 0).then_some(value),
                "border_radius" => d.border_radius = (value >= 0).then_some(value),
                _ => {}
            }
        }
        Message::SecDefaultsRespectEdges(value) => {
            // Off = absent key (the bar reads false), on = explicit true.
            state.config.section_defaults.respect_screen_edges =
                value.then_some(true);
        }
        Message::SectionLayoutChanged(value) => state.config.section_layout = value,
        Message::DefaultSectionWeightChanged(value) => state.config.default_section_weight = value,
        Message::DefaultBgColorChanged(value) => state.config.default_bg_color = if value.is_empty() { None } else { Some(value) },
        Message::DefaultTextColorChanged(value) => state.config.default_text_color = if value.is_empty() { None } else { Some(value) },
        Message::AnchorChanged(value) => state.config.anchor = if value.is_empty() { None } else { Some(value) },
        Message::AlignChanged(value) => state.config.align = if value.is_empty() { None } else { Some(value) },
        Message::OutputChanged(value) => {
            state.config.output = (value != "auto" && !value.is_empty()).then_some(value);
        }
        Message::RootSectionsChanged(value) => {
            state.config.visible_sections = parse_sections(&value);
            state.config.root_sections_text = value;
        }
        Message::PanelAdded => {
            state.config.panels.push(PanelDraft::default());
        }
        Message::PanelRemoved(index) => {
            if index < state.config.panels.len() {
                state.config.panels.remove(index);
            }
        }
        Message::PanelOutputChanged(index, value) => {
            if let Some(panel) = state.config.panels.get_mut(index) {
                panel.output = (value != "auto" && !value.is_empty()).then_some(value);
            }
        }
        Message::PanelAnchorChanged(index, value) => {
            if let Some(panel) = state.config.panels.get_mut(index) {
                panel.anchor = (value != "default" && !value.is_empty()).then_some(value);
            }
        }
        Message::PanelAlignChanged(index, value) => {
            if let Some(panel) = state.config.panels.get_mut(index) {
                panel.align = (value != "default" && !value.is_empty()).then_some(value);
            }
        }
        Message::PanelSectionsChanged(index, value) => {
            if let Some(panel) = state.config.panels.get_mut(index) {
                panel.sections = parse_sections(&value);
                panel.sections_text = value;
            }
        }
        Message::NotificationsEnabled(on) => {
            // Enabling from scratch seeds the defaults; disabling drops the
            // block, which is what turns piuma's D-Bus server off.
            state.config.notifications = match (on, state.config.notifications.take()) {
                (true, Some(prev)) => Some(prev),
                (true, None) => Some(NotificationsCfg::default()),
                (false, _) => None,
            };
        }
        Message::NotificationsTimeoutChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.default_timeout_ms = value;
            }
        }
        Message::NotificationsHistoryChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.history_size = value;
            }
        }
        Message::NotificationsReplaceChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.replace_existing = value;
            }
        }
        Message::NotifStyleChanged(key, value) => {
            if let Some(n) = &mut state.config.notifications {
                set_notif_style(n, &key, &value);
            }
        }
        Message::NotifNumberChanged(key, value) => {
            if let Some(n) = &mut state.config.notifications {
                set_notif_number(n, &key, value);
            }
        }
        Message::NotifPositionChanged(value) => {
            if let Some(n) = &mut state.config.notifications {
                n.popup_position = value;
            }
        }
        Message::AddSectionPressed => {
            state.modal = Some(Modal::AddSection { name: String::new() });
        }
        Message::RemoveSectionPressed(index) => {
            // The last section is never removable (the view also disables
            // its button); the confirmation names the collateral sensors.
            if state.config.sections.len() > 1 && index < state.config.sections.len() {
                state.modal = Some(Modal::ConfirmRemoveSection(index));
            }
        }
        Message::SectionMoveUp(index) => {
            if index > 0 {
                state.swap_sections(index - 1, index);
            }
        }
        Message::SectionMoveDown(index) => {
            if index + 1 < state.config.sections.len() {
                state.swap_sections(index, index + 1);
            }
        }
        Message::SectionNameChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].name = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::SectionLayoutChangedIndex(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].layout = Some(value);
            }
        }
        Message::SectionWeightChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].weight = Some(value);
            }
        }
        Message::SectionBgColorChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].bg_color = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::SectionTextColorChanged(index, value) => {
            if index < state.config.sections.len() {
                state.config.sections[index].text_color = if value.is_empty() { None } else { Some(value) };
            }
        }
        Message::AddSensorPressed(section_index) => {
            if section_index < state.config.sections.len() {
                state.modal = Some(Modal::AddSensor {
                    section_index,
                    sensor_index: None,
                    kind: "perc_cpu".to_string(),
                    name: String::new(),
                });
            }
        }
        Message::SensorTypePickerOpened(section_index, sensor_index) => {
            if let Some(sensor) = state
                .config
                .sections
                .get(section_index)
                .and_then(|section| section.sensors.get(sensor_index))
            {
                state.modal = Some(Modal::AddSensor {
                    section_index,
                    sensor_index: Some(sensor_index),
                    kind: sensor.source_type.clone(),
                    name: String::new(),
                });
            }
        }
        Message::SensorSelected(section_index, sensor_index) => {
            // The mini-bar jumps to the card: its tab, its section and its
            // body open, so the sensor being previewed is the one on screen.
            if state
                .config
                .sections
                .get(section_index)
                .is_some_and(|section| sensor_index < section.sensors.len())
            {
                state.active_tab = Tab::Sections;
                state.sections_collapsed.remove(&section_index);
                state.sensors_open.insert((section_index, sensor_index));
                // The card is now in the next layout: ask it (and the
                // scrollable's translation) for the offset to jump to.
                return iced_runtime::task::widget(FindSensorScroll {
                    target: sensor_card_id(section_index, sensor_index),
                    scroll: content_scroll_id(),
                    scroll_position: None,
                    sensor_position: None,
                });
            }
        }
        Message::SensorScrollTarget(offset) => {
            return operation::scroll_to(
                content_scroll_id(),
                AbsoluteOffset {
                    x: Some(0.0),
                    y: Some(offset),
                },
            );
        }
        Message::RemoveSensorPressed(section_index, sensor_index) => {
            if section_index < state.config.sections.len()
                && sensor_index < state.config.sections[section_index].sensors.len()
            {
                state.modal = Some(Modal::ConfirmRemoveSensor(section_index, sensor_index));
            }
        }
        Message::SensorDuplicated(section_index, sensor_index) => {
            if let Some(mut copy) = state
                .config
                .sections
                .get(section_index)
                .and_then(|sec| sec.sensors.get(sensor_index))
                .cloned()
            {
                // The copy is named after the original so it is recognizable.
                copy.name = copy.name.map(|n| format!("{n} (copy)"));
                let new_index = sensor_index + 1;
                state.remap_sensor_keys(|s, i| {
                    Some((s, if s == section_index && i >= new_index { i + 1 } else { i }))
                });
                state.config.sections[section_index].sensors.insert(new_index, copy);
                state.sensors_open.insert((section_index, new_index));
                state.sections_collapsed.remove(&section_index);
                // Focus the copy's name so renaming it is the next keystroke.
                return operation::focus(sensor_name_id(section_index, new_index));
            }
            return Task::none();
        }
        Message::SensorMoveTo(section_index, sensor_index, target) => {
            let in_bounds = section_index < state.config.sections.len()
                && sensor_index < state.config.sections[section_index].sensors.len()
                && target < state.config.sections.len();
            if in_bounds {
                let was_open = state.sensors_open.contains(&(section_index, sensor_index));
                let was_styled = state.sensor_style_open.contains(&(section_index, sensor_index));
                let sensor = state.config.sections[section_index].sensors.remove(sensor_index);
                // Removal first, then insertion at the target's end: for the
                // same section this reads as "move to its own end".
                state.remap_sensor_keys(|s, i| {
                    if s == section_index && i == sensor_index {
                        None
                    } else if s == section_index && i > sensor_index {
                        Some((s, i - 1))
                    } else {
                        Some((s, i))
                    }
                });
                let target_len = state.config.sections[target].sensors.len();
                state.config.sections[target].sensors.insert(target_len, sensor);
                state.remap_sensor_keys(|s, i| {
                    Some((s, if s == target && i >= target_len { i + 1 } else { i }))
                });
                // Open blocks follow the sensor; the target section unfolds
                // so the move is visible where it landed.
                if was_open {
                    state.sensors_open.insert((target, target_len));
                }
                if was_styled {
                    state.sensor_style_open.insert((target, target_len));
                }
                state.sections_collapsed.remove(&target);
            }
        }
        Message::SensorMoveUp(section_index, sensor_index) => {
            if section_index < state.config.sections.len() && sensor_index > 0 {
                state.swap_sensors(section_index, sensor_index - 1);
            }
        }
        Message::SensorMoveDown(section_index, sensor_index) => {
            if section_index < state.config.sections.len()
                && sensor_index + 1 < state.config.sections[section_index].sensors.len()
            {
                state.swap_sensors(section_index, sensor_index);
            }
        }
        Message::SensorNameChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].name = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorDescriptionChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].description = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorIconChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].icon = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::BrowseIconsPressed(section_index, sensor_index) => {
            let in_bounds = section_index < state.config.sections.len()
                && sensor_index < state.config.sections[section_index].sensors.len();
            if in_bounds {
                // The search starts from the current name, so refining an
                // existing icon needs no retype.
                let current = state.config.sections[section_index].sensors[sensor_index]
                    .icon
                    .clone()
                    .unwrap_or_default();
                state.modal = Some(Modal::BrowseIcons {
                    section_index,
                    sensor_index,
                    query: current,
                });
            }
        }
        Message::IconPicked(section_index, sensor_index, name) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].icon =
                        if name.is_empty() { None } else { Some(name) };
                }
            }
            state.modal = None;
        }
        Message::SensorSourceTypeChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].source_type = value;
                }
            }
        }
        Message::SensorSourceTypeConfigChanged(section_index, sensor_index, key, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    let sensor = &mut section.sensors[sensor_index];
                    match key.as_str() {
                        "interface" => sensor.source_config.interface = if value.is_empty() { None } else { Some(value) },
                        "device" => sensor.source_config.device = if value.is_empty() { None } else { Some(value) },
                        "format" => sensor.source_config.format = if value.is_empty() { None } else { Some(value) },
                        "command" => sensor.source_config.command = if value.is_empty() { None } else { Some(value) },
                        "env_var" => sensor.source_config.env_var = if value.is_empty() { None } else { Some(value) },
                        "text" => sensor.source_config.static_text = if value.is_empty() { None } else { Some(value) },
                        "count" => sensor.source_config.count = value.parse::<u32>().ok(),
                        "interval" => {
                            sensor.source_config.interval = value
                                .trim()
                                .parse::<u32>()
                                .ok()
                                .filter(|interval| *interval > 0);
                            sensor.source_config.interval_text = value;
                        }
                        _ => {}
                    }
                }
            }
        }
        Message::SensorTextColorChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].text_color = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBgColorChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].bg_color = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorTextColorHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].text_color_hover = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBgColorHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].bg_color_hover = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBorderColorChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_color = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBorderColorHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_color_hover = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorBorderWidthChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_width = (value >= 0).then_some(value);
                }
            }
        }
        Message::SensorBorderWidthHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_width_hover = (value >= 0).then_some(value);
                }
            }
        }
        Message::SensorBorderRadiusChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_radius = (value >= 0).then_some(value);
                }
            }
        }
        Message::SensorBorderTypeChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_type = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorPaddingChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].padding = (value >= 0).then_some(value);
                }
            }
        }
        Message::SensorActionChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].action = if value.is_empty() { None } else { Some(value) };
                }
            }
        }
        Message::SensorDrawGraphChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].draw_graph = value;
                }
            }
        }
        Message::SensorEffectChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].effect = value;
                }
            }
        }
        Message::ColorRulesPressed(section_index, sensor_index) => {
            // The editor opens only for a sensor that exists (like the icon
            // browser); its rows start sorted by `min`, the order the bar
            // evaluates them, whatever order the loaded file had.
            let existing = state
                .config
                .sections
                .get(section_index)
                .and_then(|sec| sec.sensors.get(sensor_index))
                .map(|s| s.color_rules.clone().unwrap_or_default());
            if let Some(rules) = existing {
                let drafts = sorted_rules(rules)
                    .into_iter()
                    .map(|r| RuleDraft { min: r.min.to_string(), color: r.color })
                    .collect();
                state.modal = Some(Modal::ColorRules { section_index, sensor_index, rules: drafts });
            }
        }
        Message::ColorRuleAdd => {
            if let Some(Modal::ColorRules { rules, .. }) = state.modal.as_mut() {
                rules.push(RuleDraft { min: String::new(), color: String::new() });
            }
        }
        Message::ColorRuleRemove(index) => {
            if let Some(Modal::ColorRules { rules, .. }) = state.modal.as_mut() {
                if index < rules.len() {
                    rules.remove(index);
                }
            }
        }
        Message::ColorRuleMinChanged(index, value) => {
            if let Some(Modal::ColorRules { rules, .. }) = state.modal.as_mut() {
                if let Some(rule) = rules.get_mut(index) {
                    rule.min = value;
                }
            }
        }
        Message::ColorRuleColorChanged(index, value) => {
            if let Some(Modal::ColorRules { rules, .. }) = state.modal.as_mut() {
                if let Some(rule) = rules.get_mut(index) {
                    rule.color = value;
                }
            }
        }
        Message::SaveConfig => match state.serialize() {
            // Serialization errors surface in the status bar; otherwise the
            // dirty-check decides between straight save and confirmation.
            Err(e) => state.set_status(e, true),
            Ok(current) => {
                if current == state.snapshot {
                    let name = state.current_file.clone();
                    let _ = state.save_reporting(&name);
                } else {
                    // Unsaved changes: ask before overwriting the file.
                    state.modal = Some(Modal::ConfirmSave);
                }
            }
        },
        Message::ReloadPiuma => {
            // Restarting reads the file: unsaved edits would be ignored,
            // so ask for a save first instead of silently dropping them.
            let dirty = state
                .serialize()
                .map(|current| current != state.snapshot)
                .unwrap_or(true);
            if dirty {
                state.set_status("Save the configuration before reloading Piuma".to_string(), true);
                return Task::none();
            }
            let path = std::fs::canonicalize(file_path(&state.current_file))
                .unwrap_or_else(|_| file_path(&state.current_file));
            state.set_status("Reloading Piuma…".to_string(), false);
            return Task::perform(
                smol::unblock(move || crate::reload::reload(path)),
                Message::PiumaReloaded,
            );
        }
        Message::PiumaReloaded(result) => match result {
            Ok(()) => state.set_status(
                "Piuma restarted with the saved configuration".to_string(),
                false,
            ),
            Err(e) => state.set_status(format!("Reload failed: {e}"), true),
        },
        Message::SaveAs => {
            state.modal = Some(Modal::SaveAs {
                name: state.current_file.clone(),
                exists: file_path(&state.current_file).exists(),
            });
        }
        Message::LoadConfig => {
            // Picker lists the *.json of the config dir, preselecting the file in use.
            let files = list_config_files();
            let selected = if files.contains(&state.current_file) {
                state.current_file.clone()
            } else {
                files.first().cloned().unwrap_or_default()
            };
            state.modal = Some(Modal::LoadFile { selected, files });
        }
        Message::ModalTextChanged(value) => match state.modal.as_mut() {
            Some(Modal::SaveAs { name, exists }) => {
                *exists = sanitize_file_name(&value).map(|n| file_path(&n).exists()).unwrap_or(false);
                *name = value;
            }
            Some(Modal::LoadFile { selected, .. }) => *selected = value,
            Some(Modal::AddSection { name }) | Some(Modal::AddSensor { name, .. }) => *name = value,
            Some(Modal::BrowseIcons { query, .. }) => *query = value,
            _ => {}
        },
        Message::ModalTypeChanged(kind) => {
            let mut retype = None;
            if let Some(Modal::AddSensor { section_index, sensor_index, kind: slot, .. }) =
                state.modal.as_mut()
            {
                *slot = kind.clone();
                retype = Some((*section_index, *sensor_index));
            }
            // Retyping an existing sensor applies on the click; the
            // add flow keeps the grid open for the optional name.
            if let Some((section_index, Some(sensor_index))) = retype {
                if let Some(sensor) = state
                    .config
                    .sections
                    .get_mut(section_index)
                    .and_then(|section| section.sensors.get_mut(sensor_index))
                {
                    sensor.source_type = kind;
                }
                state.modal = None;
            }
        }
        Message::LayoutDeletePressed(key) => {
            state.modal = Some(Modal::ConfirmRemoveLayout(key));
        }
        Message::LayoutsClearPressed => {
            state.modal = Some(Modal::ConfirmClearLayouts);
        }
        Message::ModalConfirm => match state.modal.take() {
            // The error list has no confirm action: only "Close" dismisses it.
            Some(modal @ Modal::InvalidFields { .. }) => state.modal = Some(modal),
            Some(Modal::ConfirmSave) => {
                let name = state.current_file.clone();
                // Invalid values replace the confirmation with the error
                // list (set by save_reporting); only other failures —
                // e.g. filesystem errors — keep the dialog open.
                if state.save_reporting(&name).is_err() && state.validation.is_empty() {
                    state.modal = Some(Modal::ConfirmSave);
                }
            }
            Some(Modal::SaveAs { name, exists }) => match sanitize_file_name(&name) {
                Ok(target) => {
                    if file_path(&target).exists() {
                        // Existing target: extra confirmation before overwrite.
                        state.modal = Some(Modal::ConfirmSaveAs(target));
                    } else {
                        let _ = state.save_reporting(&target);
                    }
                }
                Err(e) => {
                    // Invalid name: keep the form open and show the hint in it.
                    state.set_status(e, true);
                    state.modal = Some(Modal::SaveAs { name, exists });
                }
            },
            Some(Modal::ConfirmSaveAs(name)) => {
                if state.save_reporting(&name).is_err() {
                    state.modal = Some(Modal::ConfirmSaveAs(name));
                }
            }
            Some(Modal::LoadFile { selected, files }) => {
                if selected.is_empty() {
                    state.set_status("No config file found to load".to_string(), true);
                    state.modal = Some(Modal::LoadFile { selected, files });
                } else if state.load_reporting(&selected).is_err() {
                    state.modal = Some(Modal::LoadFile { selected, files });
                }
            }
            Some(Modal::AddSection { name }) => {
                let mut section = Section::default();
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    section.name = Some(trimmed.to_string());
                }
                // Appending a section shifts no existing sensor key; the
                // fresh section starts open with its default sensor open.
                state.config.sections.push(section);
                let new_index = state.config.sections.len() - 1;
                state.sections_collapsed.remove(&new_index);
                state.sensors_open.insert((new_index, 0));
                return operation::snap_to_end(content_scroll_id());
            }
            Some(Modal::AddSensor { section_index, sensor_index, kind, name }) => {
                if let Some(sensor_index) = sensor_index {
                    if let Some(sensor) = state
                        .config
                        .sections
                        .get_mut(section_index)
                        .and_then(|section| section.sensors.get_mut(sensor_index))
                    {
                        sensor.source_type = kind;
                    }
                } else if section_index < state.config.sections.len() {
                    let mut sensor = Sensor {
                        source_type: kind,
                        ..Sensor::default()
                    };
                    let trimmed = name.trim();
                    // No name = absent key: the bar renders the value alone.
                    sensor.name = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
                    let new_index = state.config.sections[section_index].sensors.len();
                    state.remap_sensor_keys(|s, i| {
                        Some((s, if s == section_index && i >= new_index { i + 1 } else { i }))
                    });
                    state.config.sections[section_index].sensors.push(sensor);
                    state.sensors_open.insert((section_index, new_index));
                    state.sections_collapsed.remove(&section_index);
                    let focus = operation::focus(sensor_name_id(section_index, new_index));
                    return if section_index + 1 == state.config.sections.len() {
                        // Appended to the last section: the row sits at the
                        // scrollable's bottom, so a plain snap lands on it.
                        Task::batch([focus, operation::snap_to_end(content_scroll_id())])
                    } else {
                        focus
                    };
                }
            }
            Some(Modal::ConfirmRemoveSection(index)) => {
                if state.config.sections.len() > 1 && index < state.config.sections.len() {
                    state.config.sections.remove(index);
                    state.sections_collapsed = state
                        .sections_collapsed
                        .drain()
                        .filter(|&s| s != index)
                        .map(|s| if s > index { s - 1 } else { s })
                        .collect();
                    state.remap_sensor_keys(|s, i| {
                        if s == index {
                            None
                        } else {
                            Some((if s > index { s - 1 } else { s }, i))
                        }
                    });
                }
            }
            Some(Modal::ConfirmRemoveSensor(section_index, sensor_index)) => {
                let in_bounds = section_index < state.config.sections.len()
                    && sensor_index < state.config.sections[section_index].sensors.len();
                if in_bounds {
                    state.config.sections[section_index].sensors.remove(sensor_index);
                    state.remap_sensor_keys(|s, i| {
                        if s == section_index && i == sensor_index {
                            None
                        } else if s == section_index && i > sensor_index {
                            Some((s, i - 1))
                        } else {
                            Some((s, i))
                        }
                    });
                }
            }
            Some(Modal::ConfirmRemoveLayout(key)) => match delete_layout(&key) {
                Ok(()) => match load_layouts() {
                    Ok(list) => {
                        state.layouts = list;
                        state.set_status("Layout deleted".to_string(), false);
                    }
                    Err(e) => state.set_status(e, true),
                },
                // The dialog stays open so the error shows inside it.
                Err(e) => {
                    state.set_status(e, true);
                    state.modal = Some(Modal::ConfirmRemoveLayout(key));
                }
            },
            Some(Modal::ConfirmClearLayouts) => match clear_layouts() {
                Ok(()) => {
                    state.layouts = Vec::new();
                    state.set_status("All layouts deleted".to_string(), false);
                }
                // The dialog stays open so the error shows inside it.
                Err(e) => {
                    state.set_status(e, true);
                    state.modal = Some(Modal::ConfirmClearLayouts);
                }
            },
            Some(Modal::ColorRules { section_index, sensor_index, rules }) => {
                // Apply: a row whose `min` is blank is one still being typed
                // (or just added) and is dropped; a typed-but-wrong `min`,
                // or a rule without a colour, is an error shown INSIDE the
                // popup, which reopens with its rows so nothing is lost.
                let mut parsed = Vec::new();
                let mut invalid = None;
                for (i, rule) in rules.iter().enumerate() {
                    let raw = rule.min.trim();
                    if raw.is_empty() {
                        continue;
                    }
                    match raw.parse::<f64>() {
                        Ok(min) if min.is_finite() => match rule.color.trim() {
                            "" => invalid = Some(format!("Rule {}: missing colour", i + 1)),
                            color => parsed.push(config::ColorRule { min, color: color.to_string() }),
                        },
                        _ => invalid = Some(format!("Rule {}: \"{}\" is not a number", i + 1, raw)),
                    }
                    if invalid.is_some() {
                        break;
                    }
                }
                if let Some(msg) = invalid {
                    state.set_status(msg, true);
                    state.modal = Some(Modal::ColorRules { section_index, sensor_index, rules });
                } else if let Some(sensor) = state
                    .config
                    .sections
                    .get_mut(section_index)
                    .and_then(|sec| sec.sensors.get_mut(sensor_index))
                {
                    // No rule left = key absent, like every other style field.
                    let sorted = sorted_rules(parsed);
                    sensor.color_rules = (!sorted.is_empty()).then_some(sorted);
                }
            }
            Some(m @ Modal::BrowseIcons { .. }) => {
                // No confirm action in the browser: a picked cell already
                // applies, so Enter keeps the modal open as it is.
                state.modal = Some(m);
            }
            None => {}
        },
        Message::ModalCancel => state.modal = None,
        Message::ToggleDefaults => state.defaults_expanded = !state.defaults_expanded,
        Message::ToggleSecDefaults => state.sec_defaults_expanded = !state.sec_defaults_expanded,
        Message::ToggleSensorStyle(section, sensor) => {
            // Insert-or-remove keeps the set as the single source of truth.
            if !state.sensor_style_open.remove(&(section, sensor)) {
                state.sensor_style_open.insert((section, sensor));
            }
        }
        Message::ToggleSection(index) => {
            if !state.sections_collapsed.remove(&index) {
                state.sections_collapsed.insert(index);
            }
        }
        Message::ToggleSensor(section, sensor) => {
            if !state.sensors_open.remove(&(section, sensor)) {
                state.sensors_open.insert((section, sensor));
            }
        }
        Message::ResetConfig => {
            // Reset goes back to the shipped default (the bar's own
            // first-start config), not the bare built-in model.
            state.config = PiumaConfig::default();
            let _ = state.load_from_json(crate::state::SHIPPED_DEFAULT);
            state.set_status("Configuration reset to defaults".to_string(), false);
        }
        Message::Status(_) => {}
    }
    Task::none()
}

/// Widget operation behind the mini-bar jump: reads the content
/// scrollable's position/translation and the target sensor card's window
/// position, then reports the content offset to scroll to. Two frames at
/// most: the card must be laid out (the click that expands it changes the
/// tree right before the operation runs).
struct FindSensorScroll {
    target: iced::widget::Id,
    scroll: iced::widget::Id,
    /// (top y, translation y) of the content scrollable.
    scroll_position: Option<(f32, f32)>,
    /// Window y of the target card.
    sensor_position: Option<f32>,
}

impl Operation<Message> for FindSensorScroll {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&iced::widget::Id>,
        bounds: iced::Rectangle,
        _content_bounds: iced::Rectangle,
        translation: iced::Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&self.scroll) {
            self.scroll_position = Some((bounds.y, translation.y));
        }
    }

    fn container(&mut self, id: Option<&iced::widget::Id>, bounds: iced::Rectangle) {
        if id == Some(&self.target) {
            self.sensor_position = Some(bounds.y);
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<Message> {
        match (self.scroll_position, self.sensor_position) {
            (Some((top, translation)), Some(sensor_y)) => {
                iced::advanced::widget::operation::Outcome::Some(Message::SensorScrollTarget(
                    scroll_target(top, translation, sensor_y),
                ))
            }
            _ => iced::advanced::widget::operation::Outcome::None,
        }
    }
}

/// Decode wallpaper previews in the background: the grid is complete from
/// the first frame and every decoded preview replaces its placeholder as
/// it lands. Two workers halve the wait of a cold cache while capping the
/// peak memory at two full decodes; the disk cache makes later runs
/// instant.
fn wallpaper_thumb_task(files: Vec<std::path::PathBuf>) -> Task<Message> {
    const WORKERS: usize = 2;
    let (sender, receiver) = iced::futures::channel::mpsc::channel(64);
    let per_worker = files.len().div_ceil(WORKERS).max(1);
    for part in files.chunks(per_worker) {
        let sender = sender.clone();
        let part = part.to_vec();
        std::thread::spawn(move || {
            for path in part {
                let Some(thumb) = crate::view_wallpaper::thumbnail(&path) else {
                    continue;
                };
                let sent = sender
                    .clone()
                    .try_send(Message::WallpaperThumbsLoaded(vec![thumb]));
                if sent.is_err() {
                    break;
                }
            }
        });
    }
    Task::stream(receiver)
}
