//! In-game window listing the damage each party member took, from the app's snapshots.

use hudhook::imgui::{Condition, ImColor32, MouseButton, Ui, WindowFlags};
use protocol::OverlayState;

use super::config::DamageTakenConfig;
use super::enemy_hp::group_digits;

/// Until the player moves it, the window hugs the top-right corner: its right edge stays put and
/// it grows to the left, so the debuff column never covers the windows beside it.
const DEFAULT_MARGIN: [f32; 2] = [25.0, 25.0];
const MIN_VISIBLE: [f32; 2] = [80.0, 30.0];
const MIN_INTERACTIVE_ALPHA: f32 = 0.6;
const PADDING: f32 = 6.0;
const TRACK_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 140);
const MUTED_COLOR: [f32; 4] = [0.78, 0.78, 0.78, 1.0];
const DEBUFF_COLOR: [f32; 4] = [1.0, 0.6, 0.55, 1.0];
/// Debuff application counts, bright green so they read at a glance.
const COUNT_COLOR: [f32; 4] = [0.35, 0.95, 0.45, 1.0];
/// Debuffs marked `highlight` (being grabbed): bright yellow.
const HIGHLIGHT_COLOR: [f32; 4] = [1.0, 0.86, 0.1, 1.0];
/// Space between the damage bars and the debuff column, in unscaled pixels.
const DEBUFF_COLUMN_GAP: f32 = 14.0;

#[derive(Default)]
pub struct DamageTakenWindow {
    reset_position: bool,
}

impl DamageTakenWindow {
    pub fn reset_position(&mut self) {
        self.reset_position = true;
    }

    pub fn render(
        &mut self,
        ui: &Ui,
        interactive: bool,
        config: &mut DamageTakenConfig,
        state: Option<&OverlayState>,
        transparency: f32,
    ) {
        // Shown whenever it is enabled, even without data, so it matches its checkbox.
        let rows = state.map_or(&[][..], |state| state.damage_taken.as_slice());

        let mut flags = WindowFlags::NO_TITLE_BAR
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::ALWAYS_AUTO_RESIZE
            | WindowFlags::NO_FOCUS_ON_APPEARING;
        if !interactive {
            flags |= WindowFlags::NO_INPUTS;
        }
        if std::mem::take(&mut self.reset_position) {
            config.position = None;
        }
        let display = ui.io().display_size;
        // Anchored by its right edge every frame until placed; then by its top-left corner.
        let (position, pivot, condition) = match config.position {
            None => (
                [display[0] - DEFAULT_MARGIN[0], DEFAULT_MARGIN[1]],
                [1.0, 0.0],
                Condition::Always,
            ),
            Some(saved) => (
                on_screen(saved, display),
                [0.0, 0.0],
                Condition::FirstUseEver,
            ),
        };
        let alpha = if interactive {
            transparency.max(MIN_INTERACTIVE_ALPHA)
        } else {
            transparency
        };

        ui.window("GBFR Logs 承受伤害")
            .position(position, condition)
            .position_pivot(pivot)
            .bg_alpha(alpha.clamp(0.0, 1.0))
            .flags(flags)
            .build(|| {
                ui.set_window_font_scale(config.scale);
                let width = config.width * config.scale;
                let labels = state.map(|state| &state.damage_taken_labels);
                let title = labels
                    .map(|labels| labels.title.as_str())
                    .filter(|title| !title.is_empty())
                    .unwrap_or("承受伤害");
                ui.text_colored(MUTED_COLOR, title);
                if rows.is_empty() {
                    ui.text_disabled(if interactive {
                        "战斗中受到攻击或负面状态后显示  菜单打开时可拖动此窗口"
                    } else {
                        "暂无数据"
                    });
                    ui.dummy([width, 0.0]);
                }

                let top_damage = rows.iter().map(|row| row.total_damage).max().unwrap_or(0);
                let hits_label = labels.map_or("", |labels| labels.hits.as_str());
                let max_label = labels.map_or("", |labels| labels.max_hit.as_str());
                for row in rows {
                    let start = ui.cursor_screen_pos();
                    // Left: name, total, bar and hit summary.
                    ui.group(|| {
                        ui.text(&row.name);
                        let value = group_digits(row.total_damage as i64);
                        let value_width = ui.calc_text_size(&value)[0];
                        ui.same_line_with_pos(
                            start[0] - ui.window_pos()[0] + (width - value_width).max(0.0),
                        );
                        ui.text(&value);

                        let bar_height = 6.0 * config.scale;
                        let top = ui.cursor_screen_pos()[1];
                        let ratio = if top_damage > 0 {
                            row.total_damage as f32 / top_damage as f32
                        } else {
                            0.0
                        };
                        let [_, r, g, b] = row.color.to_be_bytes();
                        let draw = ui.get_window_draw_list();
                        draw.add_rect(
                            [start[0], top],
                            [start[0] + width, top + bar_height],
                            TRACK_COLOR,
                        )
                        .filled(true)
                        .rounding(2.0)
                        .build();
                        if ratio > 0.0 {
                            draw.add_rect(
                                [start[0], top],
                                [start[0] + width * ratio, top + bar_height],
                                ImColor32::from_rgba(r, g, b, 220),
                            )
                            .filled(true)
                            .rounding(2.0)
                            .build();
                        }
                        drop(draw);
                        ui.dummy([width, bar_height]);
                        ui.text_colored(
                            MUTED_COLOR,
                            format!(
                                "{hits_label} {}   {max_label} {}",
                                row.hits,
                                group_digits(row.max_hit as i64)
                            ),
                        );
                    });

                    // Right: the member's debuffs, top-aligned with the row, in one column
                    // shared by all rows.
                    if !row.debuffs.is_empty() {
                        ui.same_line_with_pos(
                            start[0] - ui.window_pos()[0]
                                + width
                                + DEBUFF_COLUMN_GAP * config.scale,
                        );
                        ui.group(|| {
                            for debuff in &row.debuffs {
                                let color = if debuff.highlight {
                                    HIGHLIGHT_COLOR
                                } else {
                                    DEBUFF_COLOR
                                };
                                ui.text_colored(color, &debuff.name);
                                // The count stands out; duration and damage stay muted.
                                ui.same_line();
                                ui.text_colored(COUNT_COLOR, format!("×{}", debuff.count));
                                ui.same_line();
                                let mut detail = format!("{:.1}s", debuff.total_ms as f64 / 1000.0);
                                if debuff.damage > 0 {
                                    detail.push_str(&format!(
                                        "  {}",
                                        group_digits(debuff.damage as i64)
                                    ));
                                }
                                ui.text_colored(MUTED_COLOR, detail);
                            }
                        });
                    }
                    ui.dummy([width, PADDING * 0.5]);
                }
                // Only a drag places it; otherwise it keeps following the right edge.
                if interactive && ui.is_window_focused() && ui.is_mouse_dragging(MouseButton::Left)
                {
                    config.position = Some(ui.window_pos());
                }
            });
    }
}

fn on_screen(position: [f32; 2], display: [f32; 2]) -> [f32; 2] {
    if display[0] <= 0.0 || display[1] <= 0.0 {
        return position;
    }
    [
        position[0].clamp(0.0, (display[0] - MIN_VISIBLE[0]).max(0.0)),
        position[1].clamp(0.0, (display[1] - MIN_VISIBLE[1]).max(0.0)),
    ]
}
