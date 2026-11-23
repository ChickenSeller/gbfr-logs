//! In-game copy of the settings page's meter options.
//!
//! The settings page's store stays the single source of truth: edits made here are sent to the
//! app as [`Message::OverlaySettingsChange`], which applies them and echoes them back in the
//! next snapshot. Until that echo arrives the edited values are shown and used locally, so
//! sliders and checkboxes respond immediately.

use std::time::{Duration, Instant};

use hudhook::imgui::{Direction, Ui};
use protocol::{Message, OverlayColumn, OverlaySettings, OverlaySettingsValues};

use crate::event;

/// How long an unconfirmed edit is kept, e.g. when the app is not connected.
const PENDING_TIMEOUT: Duration = Duration::from_secs(2);

pub struct SettingsEditor {
    /// Edited values not yet echoed back by the app, and when they were last changed.
    pending: Option<(OverlaySettingsValues, Instant)>,
    tx: event::Tx,
}

impl SettingsEditor {
    pub fn new(tx: event::Tx) -> Self {
        Self { pending: None, tx }
    }

    /// The values in effect: an in-game edit until the app confirms or drops it, otherwise
    /// the app's.
    pub fn effective<'a>(&'a mut self, settings: &'a OverlaySettings) -> &'a OverlaySettingsValues {
        if self.pending.as_ref().is_some_and(|(values, changed)| {
            *values == settings.values || changed.elapsed() > PENDING_TIMEOUT
        }) {
            self.pending = None;
        }
        self.pending
            .as_ref()
            .map_or(&settings.values, |(values, _)| values)
    }

    pub fn render(&mut self, ui: &Ui, settings: Option<&OverlaySettings>) {
        let Some(settings) = settings else {
            ui.text_disabled("连接 GBFR Logs 后可修改统计设置");
            return;
        };
        let current = self.effective(settings).clone();
        let mut values = current.clone();
        let labels = &settings.labels;

        let mut language = settings
            .languages
            .iter()
            .position(|option| option.value == values.language)
            .unwrap_or(0);
        let language_names: Vec<&str> = settings
            .languages
            .iter()
            .map(|option| option.label.as_str())
            .collect();
        if ui.combo_simple_string(&labels.language, &mut language, &language_names) {
            if let Some(option) = settings.languages.get(language) {
                values.language = option.value.clone();
            }
        }

        for (slot, color) in values.party_colors.iter_mut().enumerate() {
            let fallback = format!("Player {} color", slot + 1);
            let label = labels.party_colors.get(slot).unwrap_or(&fallback);
            let mut rgb = rgb_to_floats(*color);
            if ui.color_edit3(label, &mut rgb) {
                *color = floats_to_rgb(rgb);
            }
        }

        ui.slider_config(&labels.transparency, 0.0, 1.0)
            .display_format("%.2f")
            .build(&mut values.transparency);

        ui.checkbox(&labels.show_display_names, &mut values.show_display_names);
        checkbox_with_tooltip(
            ui,
            &labels.streamer_mode,
            &labels.streamer_mode_description,
            &mut values.streamer_mode,
        );
        checkbox_with_tooltip(
            ui,
            &labels.show_full_values,
            &labels.show_full_values_description,
            &mut values.show_full_values,
        );
        checkbox_with_tooltip(
            ui,
            &labels.use_condensed_skills,
            &labels.use_condensed_skills_description,
            &mut values.use_condensed_skills,
        );
        checkbox_with_tooltip(
            ui,
            &labels.open_log_on_save,
            &labels.open_log_on_save_description,
            &mut values.open_log_on_save,
        );

        ui.separator();
        ui.text(&labels.columns);
        render_columns(ui, settings, &mut values.columns);

        if values != current {
            let _ = self.tx.send(Message::OverlaySettingsChange(values.clone()));
            self.pending = Some((values, Instant::now()));
        }
    }
}

/// Shown columns in order, each with a checkbox and up/down buttons, then the hidden ones.
fn render_columns(ui: &Ui, settings: &OverlaySettings, columns: &mut Vec<OverlayColumn>) {
    let mut remove = None;
    let mut swap = None;
    let count = columns.len();
    for (index, &column) in columns.iter().enumerate() {
        let _id = ui.push_id_usize(index);
        let mut shown = true;
        if ui.checkbox(settings.column_label(column), &mut shown) && !shown {
            remove = Some(index);
        }
        column_tooltip(ui, settings, column);
        ui.same_line();
        if ui.arrow_button("##up", Direction::Up) && index > 0 {
            swap = Some((index, index - 1));
        }
        ui.same_line();
        if ui.arrow_button("##down", Direction::Down) && index + 1 < count {
            swap = Some((index, index + 1));
        }
    }
    if let Some((a, b)) = swap {
        columns.swap(a, b);
    }
    if let Some(index) = remove {
        columns.remove(index);
    }

    let hidden: Vec<OverlayColumn> = settings
        .columns
        .iter()
        .map(|info| info.column)
        .filter(|column| !columns.contains(column))
        .collect();
    for column in hidden {
        let mut shown = false;
        if ui.checkbox(settings.column_label(column), &mut shown) && shown {
            columns.push(column);
        }
        column_tooltip(ui, settings, column);
    }
}

fn column_tooltip(ui: &Ui, settings: &OverlaySettings, column: OverlayColumn) {
    if !ui.is_item_hovered() {
        return;
    }
    if let Some(info) = settings.columns.iter().find(|info| info.column == column) {
        ui.tooltip_text(&info.description);
    }
}

fn checkbox_with_tooltip(ui: &Ui, label: &str, description: &str, value: &mut bool) {
    ui.checkbox(label, value);
    if ui.is_item_hovered() && !description.is_empty() {
        ui.tooltip_text(description);
    }
}

fn rgb_to_floats(color: u32) -> [f32; 3] {
    let [_, r, g, b] = color.to_be_bytes();
    [r, g, b].map(|channel| channel as f32 / 255.0)
}

fn floats_to_rgb(rgb: [f32; 3]) -> u32 {
    let [r, g, b] = rgb.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32);
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip_through_the_picker() {
        for color in [0xFF5630, 0x00B8D9, 0x000000, 0xFFFFFF] {
            assert_eq!(floats_to_rgb(rgb_to_floats(color)), color);
        }
    }
}
