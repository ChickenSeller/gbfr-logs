//! The in-game hall of shame (战犯榜), from the app's snapshots. Per party member, in three
//! columns: the damage they took; their debuffs; the link attacks and SBA chains they missed,
//! how often they were downed and the grabs they were caught in (yellow).

use hudhook::imgui::{Condition, ImColor32, MouseButton, Ui, WindowFlags};
use protocol::OverlayState;

use super::config::DamageTakenConfig;
use super::enemy_hp::group_digits;

/// Until the player moves it, the window hugs the top-right corner: its right edge stays put and
/// it grows to the left, so the debuff column never covers the windows beside it.
const DEFAULT_MARGIN: [f32; 2] = [25.0, 25.0];
const MIN_INTERACTIVE_ALPHA: f32 = 0.6;
const PADDING: f32 = 6.0;
const TRACK_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 140);
const MUTED_COLOR: [f32; 4] = [0.78, 0.78, 0.78, 1.0];
const DEBUFF_COLOR: [f32; 4] = [1.0, 0.6, 0.55, 1.0];
/// Debuff application counts, bright green so they read at a glance.
const COUNT_COLOR: [f32; 4] = [0.35, 0.95, 0.45, 1.0];
/// Debuffs marked `highlight` (being grabbed), and the right column's counts: bright yellow.
const HIGHLIGHT_COLOR: [f32; 4] = [1.0, 0.86, 0.1, 1.0];
/// Space between the damage bars and the debuff column, in unscaled pixels.
const DEBUFF_COLUMN_GAP: f32 = 14.0;

#[derive(Default)]
pub struct DamageTakenWindow {
    reset_position: bool,
    /// Its size and the display it was placed on, to keep it on screen at any resolution.
    placement: super::placement::Placement,
}

impl DamageTakenWindow {
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
            Some(saved) => {
                let (position, condition) = self.placement.place(saved, display);
                (position, [0.0, 0.0], condition)
            }
        };
        let alpha = if interactive {
            transparency.max(MIN_INTERACTIVE_ALPHA)
        } else {
            transparency
        };

        let mut window_size = [0.0, 0.0];
        ui.window("GBFR Logs 战犯榜")
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
                    .unwrap_or("战犯榜");
                ui.text_colored(MUTED_COLOR, title);
                if rows.is_empty() {
                    ui.text_disabled(if interactive {
                        "战斗中漏接 Link/奥义、受到攻击或负面状态后显示  菜单打开时可拖动此窗口"
                    } else {
                        "暂无数据"
                    });
                    ui.dummy([width, 0.0]);
                }

                let top_damage = rows.iter().map(|row| row.total_damage).max().unwrap_or(0);
                let hits_label = labels.map_or("", |labels| labels.hits.as_str());
                let max_label = labels.map_or("", |labels| labels.max_hit.as_str());
                let links_label = labels
                    .map(|labels| labels.missed_links.as_str())
                    .filter(|label| !label.is_empty())
                    .unwrap_or("漏Link");
                let chains_label = labels
                    .map(|labels| labels.missed_chains.as_str())
                    .filter(|label| !label.is_empty())
                    .unwrap_or("漏奥义");
                let deaths_label = labels
                    .map(|labels| labels.deaths.as_str())
                    .filter(|label| !label.is_empty())
                    .unwrap_or("濒死");
                let auto_revive_label = labels
                    .map(|labels| labels.auto_revives.as_str())
                    .filter(|label| !label.is_empty())
                    .unwrap_or("自动复活");
                let downed_time_label = labels
                    .map(|labels| labels.downed_time.as_str())
                    .filter(|label| !label.is_empty())
                    .unwrap_or("濒死时间");
                // The right column's first two lines are pairs (missed links | missed chains,
                // downed | auto-revives); the second of each starts at the same place in every
                // row.
                let pair_offset = rows
                    .iter()
                    .flat_map(|row| {
                        [
                            format!("{links_label} {}", row.missed_links),
                            format!("{deaths_label} {}", row.deaths),
                        ]
                    })
                    .map(|text| ui.calc_text_size(text)[0])
                    .fold(0.0_f32, f32::max)
                    + DEBUFF_COLUMN_GAP * config.scale;
                // The middle column (debuffs) is as wide as its widest line in any row, so the
                // right column (misses, deaths and grabs) lines up across rows.
                let spacing = ui.clone_style().item_spacing[0];
                let gap = DEBUFF_COLUMN_GAP * config.scale;
                let debuffs_width = rows
                    .iter()
                    .flat_map(|row| row.debuffs.iter().filter(|debuff| !debuff.highlight))
                    .map(|debuff| {
                        let [name, count, detail] = debuff_parts(debuff);
                        ui.calc_text_size(name)[0]
                            + ui.calc_text_size(count)[0]
                            + ui.calc_text_size(detail)[0]
                            + 2.0 * spacing
                    })
                    .fold(0.0_f32, f32::max);
                let right_column = if debuffs_width > 0.0 {
                    width + gap + debuffs_width + gap
                } else {
                    width + gap
                };
                for row in rows {
                    let start = ui.cursor_screen_pos();
                    let column = |offset: f32| start[0] - ui.window_pos()[0] + offset;
                    // Left: name, total, bar and hit summary.
                    ui.group(|| {
                        ui.text(&row.name);
                        let value = group_digits(row.total_damage as i64);
                        let value_width = ui.calc_text_size(&value)[0];
                        // In the group: from its left edge.
                        ui.same_line_with_pos((width - value_width).max(0.0));
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

                    // Middle: the member's other debuffs, top-aligned with the row.
                    if row.debuffs.iter().any(|debuff| !debuff.highlight) {
                        ui.same_line_with_pos(column(width + gap));
                        ui.group(|| {
                            for debuff in row.debuffs.iter().filter(|debuff| !debuff.highlight) {
                                debuff_line(ui, debuff, DEBUFF_COLOR);
                            }
                        });
                    }

                    // Right:
                    //   漏Link 3     漏奥义 4
                    //   濒死 1       自动复活 2
                    //   濒死时间 00:20
                    //   grabs (yellow) and their time
                    ui.same_line_with_pos(column(right_column));
                    ui.group(|| {
                        // Inside a group, positions on a line count from the group's left edge.
                        // All in yellow, 0 until something happens.
                        let counted = |label: &str, count: u32| {
                            ui.text_colored(HIGHLIGHT_COLOR, format!("{label} {count}"));
                        };
                        counted(links_label, row.missed_links);
                        ui.same_line_with_pos(pair_offset);
                        counted(chains_label, row.missed_chains);
                        counted(deaths_label, row.deaths);
                        ui.same_line_with_pos(pair_offset);
                        counted(auto_revive_label, row.auto_revives);
                        ui.text_colored(
                            HIGHLIGHT_COLOR,
                            format!("{downed_time_label} {}", minutes_seconds(row.downed_ms)),
                        );
                        for debuff in row.debuffs.iter().filter(|debuff| debuff.highlight) {
                            ui.text_colored(HIGHLIGHT_COLOR, &debuff.name);
                            ui.same_line();
                            ui.text_colored(HIGHLIGHT_COLOR, minutes_seconds(debuff.total_ms));
                        }
                    });
                    ui.dummy([width, PADDING * 0.5]);
                }
                window_size = ui.window_size();
                // Only a drag places it; otherwise it keeps following the right edge.
                if interactive && ui.is_window_focused() && ui.is_mouse_dragging(MouseButton::Left)
                {
                    config.position = Some(ui.window_pos());
                }
            });
        if window_size[0] > 0.0 {
            self.placement.set_size(window_size);
        }
    }
}

/// `mm:ss`, as the hall of shame shows durations.
fn minutes_seconds(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// A debuff line's three parts: name, count and the muted detail (time, damage while active).
fn debuff_parts(debuff: &protocol::OverlayStatusLine) -> [String; 3] {
    let mut detail = format!("{:.1}s", debuff.total_ms as f64 / 1000.0);
    if debuff.damage > 0 {
        detail.push_str(&format!("  {}", group_digits(debuff.damage as i64)));
    }
    [debuff.name.clone(), format!("×{}", debuff.count), detail]
}

/// One debuff: its name in `color`, the count standing out, duration and damage muted.
fn debuff_line(ui: &Ui, debuff: &protocol::OverlayStatusLine, color: [f32; 4]) {
    let [name, count, detail] = debuff_parts(debuff);
    ui.text_colored(color, name);
    ui.same_line();
    ui.text_colored(COUNT_COLOR, count);
    ui.same_line();
    ui.text_colored(MUTED_COLOR, detail);
}
