//! The message loop: one handler per `Message`, mutating the setup state
//! or spawning iced tasks (focus, icon browsing, reloads).
use iced::widget::operation;
use iced::Task;

use crate::autostart;
use crate::config;
use crate::controls::{content_scroll_id, sensor_name_id, sorted_rules};
use crate::modal::{Modal, RuleDraft};
use crate::view_wallpaper::wallpaper_set;
use crate::model::*;
use crate::state::{file_path, list_config_files, sanitize_file_name, Message, PiumaSetup};

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
        Message::PaletteFromWallpaper(value) => {
            // On = explicit true; off drops back to absent (≈ false):
            // a save never injects the key, and the extracted tones
            // never reach the file.
            state.config.wallpaper_palette = value.then_some(true);
        }
        Message::TabSelected(tab) => state.active_tab = tab,
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
            match field.as_str() {
                "border_width" => d.border_width = Some(value),
                "border_width_hover" => d.border_width_hover = Some(value),
                "border_radius" => d.border_radius = Some(value),
                "padding" => d.padding = Some(value),
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
        Message::SectionNumberChanged(index, field, value) => {
            if index < state.config.sections.len() {
                let s = &mut state.config.sections[index];
                match field.as_str() {
                    "border_width" => s.border_width = Some(value),
                    "border_radius" => s.border_radius = Some(value),
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
                "border_width" => d.border_width = Some(value),
                "border_radius" => d.border_radius = Some(value),
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
                    kind: "perc_cpu".to_string(),
                    name: String::new(),
                });
            }
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
                    section.sensors[sensor_index].border_width = Some(value);
                }
            }
        }
        Message::SensorBorderWidthHoverChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_width_hover = Some(value);
                }
            }
        }
        Message::SensorBorderRadiusChanged(section_index, sensor_index, value) => {
            if section_index < state.config.sections.len() {
                let section = &mut state.config.sections[section_index];
                if sensor_index < section.sensors.len() {
                    section.sensors[sensor_index].border_radius = Some(value);
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
                    section.sensors[sensor_index].padding = Some(value);
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
            if let Some(Modal::AddSensor { kind: slot, .. }) = state.modal.as_mut() {
                *slot = kind;
            }
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
            Some(Modal::AddSensor { section_index, kind, name }) => {
                if section_index < state.config.sections.len() {
                    let mut sensor = Sensor::default();
                    sensor.source_type = kind;
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
            state.config = PiumaConfig::default();
            state.set_status("Configuration reset to defaults".to_string(), false);
        }
        Message::Status(_) => {}
    }
    Task::none()
}
