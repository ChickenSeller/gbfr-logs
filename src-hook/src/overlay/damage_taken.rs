//! In-game window listing the damage each party member took, from the app's snapshots.

use hudhook::imgui::{Condition, ImColor32, Ui, WindowFlags};
use protocol::OverlayState;

use super::config::DamageTakenConfig;
use super::enemy_hp::group_digits;

const DEFAULT_POSITION: [f32; 2] = [40.0, 260.0];
const MIN_VISIBLE: [f32; 2] = [80.0, 30.0];
const MIN_INTERACTIVE_ALPHA: f32 = 0.6;
const PADDING: f32 = 6.0;
const TRACK_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 140);
const MUTED_COLOR: [f32; 4] = [0.78, 0.78, 0.78, 1.0];

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
        let rows = state.map_or(&[][..], |state| state.damage_taken.as_slice());
        if rows.is_empty() && !interactive {
            return;
        }

        let mut flags = WindowFlags::NO_TITLE_BAR
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::ALWAYS_AUTO_RESIZE
            | WindowFlags::NO_FOCUS_ON_APPEARING;
        if !interactive {
            flags |= WindowFlags::NO_INPUTS;
        }
        let (position, condition) = if std::mem::take(&mut self.reset_position) {
            config.position = None;
            (DEFAULT_POSITION, Condition::Always)
        } else {
            let saved = config.position.unwrap_or(DEFAULT_POSITION);
            (
                on_screen(saved, ui.io().display_size),
                Condition::FirstUseEver,
            )
        };
        let alpha = if interactive {
            transparency.max(MIN_INTERACTIVE_ALPHA)
        } else {
            transparency
        };

        ui.window("GBFR Logs 承受伤害")
            .position(position, condition)
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
                    ui.text_disabled("战斗中受到攻击后显示  菜单打开时可拖动此窗口");
                    ui.dummy([width, 0.0]);
                }

                let top_damage = rows.iter().map(|row| row.total_damage).max().unwrap_or(0);
                let hits_label = labels.map_or("", |labels| labels.hits.as_str());
                let max_label = labels.map_or("", |labels| labels.max_hit.as_str());
                for row in rows {
                    let start = ui.cursor_screen_pos();
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
                    ui.dummy([width, PADDING * 0.5]);
                }
                if interactive {
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
