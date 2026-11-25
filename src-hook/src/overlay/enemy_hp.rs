//! In-game enemy HP bars, read live from the game (see `hooks::enemy_hp`).

use std::collections::HashMap;

use hudhook::imgui::{Condition, ImColor32, Ui, WindowFlags};
use protocol::OverlayEnemy;

use super::config::EnemyHpConfig;
use crate::hooks::enemy_hp::{self, EnemyHp, PartState};

/// Default layout: a row along the top edge, right of the settings menu.
const DEFAULT_POSITION: [f32; 2] = [470.0, 25.0];
const MIN_INTERACTIVE_ALPHA: f32 = 0.6;
const PADDING: f32 = 6.0;

const TRACK_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 140);
const HP_COLOR: ImColor32 = ImColor32::from_rgba(214, 64, 52, 230);
const LOW_HP_COLOR: ImColor32 = ImColor32::from_rgba(255, 160, 40, 230);
const DEFEATED_COLOR: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
const TEXT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const PART_BAR_COLOR: ImColor32 = ImColor32::from_rgba(255, 196, 64, 220);
const PART_TEXT_COLOR: [f32; 4] = [1.0, 0.85, 0.45, 1.0];
const PART_BROKEN_COLOR: [f32; 4] = [0.6, 0.6, 0.6, 1.0];

/// Below this share of max HP the bar turns orange.
const LOW_HP_RATIO: f32 = 0.3;

#[derive(Default)]
pub struct EnemyHpWindow {
    reset_position: bool,
    /// Its size and the display it was placed on, to keep it on screen at any resolution.
    placement: super::placement::Placement,
}

impl EnemyHpWindow {
    /// Places the window again on the next frame, after its saved position was scaled.
    pub fn replace(&mut self) {
        self.placement.replace();
    }

    pub fn reset_position(&mut self) {
        self.reset_position = true;
    }

    pub fn render(
        &mut self,
        ui: &Ui,
        interactive: bool,
        config: &mut EnemyHpConfig,
        names: &[OverlayEnemy],
        transparency: f32,
    ) {
        // Shown whenever it is enabled, even without data, so it matches its checkbox.
        let enemies = enemy_hp::snapshot();
        let names: HashMap<u32, &str> = names
            .iter()
            .map(|enemy| (enemy.type_hash, enemy.name.as_str()))
            .collect();

        let mut flags = WindowFlags::NO_TITLE_BAR
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::ALWAYS_AUTO_RESIZE
            | WindowFlags::NO_FOCUS_ON_APPEARING;
        if !interactive {
            flags |= WindowFlags::NO_INPUTS;
        }
        let display = ui.io().display_size;
        let default = super::placement::scaled_default(DEFAULT_POSITION, display);
        let (position, condition) = if std::mem::take(&mut self.reset_position) {
            config.position = None;
            self.placement.replace();
            (default, Condition::Always)
        } else {
            self.placement
                .place(config.position.unwrap_or(default), display)
        };
        let alpha = if interactive {
            transparency.max(MIN_INTERACTIVE_ALPHA)
        } else {
            transparency
        };

        let mut window_size = [0.0, 0.0];
        ui.window("GBFR Logs 敌人血量")
            .position(position, condition)
            .bg_alpha(alpha.clamp(0.0, 1.0))
            .flags(flags)
            .build(|| {
                ui.set_window_font_scale(config.scale);
                if enemies.is_empty() {
                    ui.text_disabled(if interactive {
                        "敌人血量：攻击敌人后显示  菜单打开时可拖动此窗口"
                    } else {
                        "敌人血量：暂无数据"
                    });
                    // Keep the placeholder as wide as a bar so the window can be placed.
                    ui.dummy([config.width * config.scale, 0.0]);
                }
                let shown = enemies.iter().take(config.max_enemies as usize);
                for enemy in shown {
                    draw_enemy(ui, config, enemy, names.get(&enemy.type_hash).copied());
                }
                window_size = ui.window_size();
                if interactive {
                    config.position = Some(ui.window_pos());
                }
            });
        if window_size[0] > 0.0 {
            self.placement.set_size(window_size);
        }
    }
}

fn draw_enemy(ui: &Ui, config: &EnemyHpConfig, enemy: &EnemyHp, name: Option<&str>) {
    let width = config.width * config.scale;
    let bar_height = 10.0 * config.scale;
    let ratio = (enemy.hp as f64 / enemy.max_hp as f64).clamp(0.0, 1.0) as f32;

    let fallback;
    let name = match name {
        Some(name) => name,
        None => {
            fallback = format!("{:08x}", enemy.type_hash);
            &fallback
        }
    };
    let value = format!(
        "{} / {}  {:.1}%",
        group_digits(enemy.hp),
        group_digits(enemy.max_hp),
        ratio * 100.0
    );

    let start = ui.cursor_screen_pos();
    let color = if enemy.hp == 0 {
        DEFEATED_COLOR
    } else {
        TEXT_COLOR
    };
    ui.text_colored(color, name);
    let value_width = ui.calc_text_size(&value)[0];
    ui.same_line_with_pos(start[0] - ui.window_pos()[0] + (width - value_width).max(0.0));
    ui.text_colored(color, &value);

    let top = ui.cursor_screen_pos()[1];
    let left = start[0];
    let draw = ui.get_window_draw_list();
    draw.add_rect([left, top], [left + width, top + bar_height], TRACK_COLOR)
        .filled(true)
        .rounding(2.0)
        .build();
    if ratio > 0.0 {
        let fill = if ratio < LOW_HP_RATIO {
            LOW_HP_COLOR
        } else {
            HP_COLOR
        };
        draw.add_rect([left, top], [left + width * ratio, top + bar_height], fill)
            .filled(true)
            .rounding(2.0)
            .build();
    }
    drop(draw);
    ui.dummy([width, bar_height + PADDING * 0.5]);

    let numbered = enemy.parts.len() > 1;
    for (index, part) in enemy.parts.iter().enumerate() {
        draw_part(ui, config, width, part, numbered.then_some(index + 1));
    }
    ui.dummy([width, PADDING * 0.5]);
}

/// The breakable part under the HP bar: a thinner bar with its durability, or "broken".
fn draw_part(ui: &Ui, config: &EnemyHpConfig, width: f32, part: &PartState, number: Option<usize>) {
    // The game's own break value: drained by damage to the part, the part breaks at zero.
    // The game has no names for parts in its text: the internal name where the game keeps one,
    // otherwise the part's id.
    let label = part
        .name
        .clone()
        .unwrap_or_else(|| format!("#{:x}", part.id));
    let name = match number {
        Some(number) => format!("部位{number}（{label}） 破坏值"),
        None => format!("部位（{label}） 破坏值"),
    };
    let ratio = if part.max_durability > 0.0 {
        (part.durability / part.max_durability).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label = if part.broken() {
        format!("{name}  已破坏")
    } else {
        format!(
            "{name}  {:.0} / {:.0}  {:.0}%",
            part.durability,
            part.max_durability,
            ratio * 100.0
        )
    };
    let color = if part.broken() {
        PART_BROKEN_COLOR
    } else {
        PART_TEXT_COLOR
    };
    ui.text_colored(color, label);

    let bar_height = 5.0 * config.scale;
    let [left, top] = ui.cursor_screen_pos();
    let draw = ui.get_window_draw_list();
    draw.add_rect([left, top], [left + width, top + bar_height], TRACK_COLOR)
        .filled(true)
        .rounding(2.0)
        .build();
    if ratio > 0.0 {
        draw.add_rect(
            [left, top],
            [left + width * ratio, top + bar_height],
            PART_BAR_COLOR,
        )
        .filled(true)
        .rounding(2.0)
        .build();
    }
    drop(draw);
    ui.dummy([width, bar_height]);
}

/// `113810780` -> `113,810,780`.
pub fn group_digits(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::group_digits;

    #[test]
    fn groups_thousands() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(113_810_780), "113,810,780");
    }
}
