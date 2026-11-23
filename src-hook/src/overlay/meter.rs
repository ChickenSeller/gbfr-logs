//! In-game damage meter: a port of the desktop meter window.
//!
//! Title line (team damage, DPS, timer, copy menu), a table with the columns chosen on the
//! settings page, sortable by clicking a header, and rows that expand into the skill breakdown
//! (with condensed skill groups). Bars use the MHR-Overlay style: the top damage dealer fills
//! the row.
//!
//! Everything is drawn through one window draw list: imgui-rs panics if a second one is taken
//! while the first is alive. Clickable areas are invisible buttons placed over the drawing,
//! which only receive input while the menu has released the mouse.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hudhook::imgui::{Condition, DrawListMut, ImColor32, MouseButton, Ui, WindowFlags};
use protocol::{
    Message, OverlayAttackType, OverlayColumn, OverlayCopyKind, OverlayCopyRequest,
    OverlayNoticeLevel, OverlayPlayer, OverlaySettingsValues, OverlaySkill, OverlayState,
    OverlayStatus,
};

use super::config::MeterConfig;
use super::feed;
use crate::event;

const DEFAULT_POSITION: [f32; 2] = [1075.0, 1449.0];
/// Part of the window that must stay on screen when restoring a saved position.
const MIN_VISIBLE: [f32; 2] = [80.0, 40.0];
/// Background opacity while the menu is open, so the window is easy to find and drag.
const MIN_INTERACTIVE_ALPHA: f32 = 0.6;

const BAR_ALPHA: u8 = 200;
const SKILL_BAR_ALPHA: u8 = 90;
const TRACK_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 90);
const TITLE_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 110);
const HOVER_COLOR: ImColor32 = ImColor32::from_rgba(255, 255, 255, 40);
const TEXT_COLOR: ImColor32 = ImColor32::from_rgba(255, 255, 255, 255);
const SHADOW_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 200);
const MUTED_COLOR: ImColor32 = ImColor32::from_rgba(200, 200, 200, 255);
const SUCCESS_COLOR: [f32; 4] = [0.55, 0.9, 0.55, 1.0];
const ERROR_COLOR: [f32; 4] = [1.0, 0.45, 0.45, 1.0];
const NOTICE_DURATION: Duration = Duration::from_secs(3);
const COPY_MENU_ID: &str = "##overlay_copy_menu";

/// Skill table columns, as labelled on the desktop meter.
const SKILL_HEADERS: [&str; 6] = ["Hits", "Total", "Min", "Max", "Avg", "%"];

/// Pixel sizes derived from the configured width and scale, so slider changes apply on the
/// next frame.
struct Layout {
    width: f32,
    row_height: f32,
    row_gap: f32,
    padding: f32,
    column_gap: f32,
    indent: f32,
    rounding: f32,
}

impl Layout {
    fn new(config: &MeterConfig) -> Self {
        let scale = config.scale;
        Self {
            width: config.width * scale,
            row_height: 22.0 * scale,
            row_gap: 2.0 * scale,
            padding: 6.0 * scale,
            column_gap: 10.0 * scale,
            indent: 14.0 * scale,
            rounding: 3.0 * scale,
        }
    }
}

/// What the meter shows: the latest snapshot, with the settings currently in effect (which
/// may be in-game edits the app has not echoed back yet).
#[derive(Clone, Copy)]
pub struct MeterView<'a> {
    pub state: &'a OverlayState,
    pub values: &'a OverlaySettingsValues,
}

pub struct MeterWindow {
    /// Sends copy requests to the desktop app, which owns the export formats.
    tx: event::Tx,
    /// Moves the window back to its default position on the next frame.
    reset_position: bool,
    expanded_players: HashSet<u32>,
    expanded_groups: HashSet<(u32, String)>,
}

/// One frame's drawing state: the shared draw list and a vertical cursor.
struct Frame<'ui> {
    ui: &'ui Ui,
    draw: DrawListMut<'ui>,
    layout: Layout,
    left: f32,
    top: f32,
    y: f32,
    interactive: bool,
}

impl MeterWindow {
    pub fn new(tx: event::Tx) -> Self {
        Self {
            tx,
            reset_position: false,
            expanded_players: HashSet::new(),
            expanded_groups: HashSet::new(),
        }
    }

    pub fn reset_position(&mut self) {
        self.reset_position = true;
    }

    /// `interactive` is true while the overlay menu is open: the window can then be dragged,
    /// sorted and expanded, and its position is written back to `config`. Otherwise it ignores
    /// all input so it never steals clicks from the game.
    pub fn render(
        &mut self,
        ui: &Ui,
        interactive: bool,
        config: &mut MeterConfig,
        view: Option<MeterView>,
    ) {
        // Shown whenever it is enabled; until the app connects it holds a placeholder.

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
        let configured_alpha = view.map_or(0.35, |view| view.values.transparency);
        let alpha = if interactive {
            configured_alpha.max(MIN_INTERACTIVE_ALPHA)
        } else {
            configured_alpha
        };

        ui.window("GBFR Logs 伤害统计")
            .position(position, condition)
            .bg_alpha(alpha.clamp(0.0, 1.0))
            .flags(flags)
            .build(|| {
                ui.set_window_font_scale(config.scale);
                let origin = ui.cursor_screen_pos();
                let mut frame = Frame {
                    ui,
                    draw: ui.get_window_draw_list(),
                    layout: Layout::new(config),
                    left: origin[0],
                    top: origin[1],
                    y: origin[1],
                    interactive,
                };

                match view {
                    Some(view) => {
                        self.draw_title(&mut frame, view, config);
                        if !view.state.players.is_empty() {
                            self.draw_table(&mut frame, view, config);
                        }
                    }
                    None => frame.placeholder(if interactive {
                        "等待 GBFR Logs 连接…  菜单打开时可拖动此窗口"
                    } else {
                        "等待 GBFR Logs 连接…"
                    }),
                }

                frame.finish();
                if let Some(notice) = feed::notice(NOTICE_DURATION) {
                    let color = match notice.level {
                        OverlayNoticeLevel::Success => SUCCESS_COLOR,
                        OverlayNoticeLevel::Error => ERROR_COLOR,
                    };
                    ui.text_colored(color, notice.text);
                }
                if interactive {
                    config.position = Some(ui.window_pos());
                }
            });
    }

    /// The desktop title bar: app name, team damage and DPS, then the timer and copy menu.
    fn draw_title(&self, frame: &mut Frame, view: MeterView, config: &MeterConfig) {
        let state = view.state;
        let show_full = view.values.show_full_values;
        let mut left_text = "GBFR Logs".to_string();
        if state.total_damage > 0 {
            left_text.push_str(&format!(
                "  - {} -  {}/s",
                format_amount(state.total_damage as f64, show_full),
                format_amount(state.dps, show_full)
            ));
        }
        let status = match state.status {
            OverlayStatus::Waiting => "Waiting..".to_string(),
            OverlayStatus::InProgress => format_elapsed((now_ms() - state.start_time).max(0)),
            OverlayStatus::Stopped => format_elapsed((state.end_time - state.start_time).max(0)),
        };

        let height = frame.layout.row_height;
        let (left, top, width) = (frame.left, frame.y, frame.layout.width);
        frame
            .draw
            .add_rect([left, top], [left + width, top + height], TITLE_COLOR)
            .filled(true)
            .rounding(frame.layout.rounding)
            .build();

        let icon_size = frame.ui.text_line_height();
        let show_copy = frame.interactive && state.total_damage > 0;
        let mut right = left + width - frame.layout.padding;
        if show_copy {
            let icon_pos = [right - icon_size, top + (height - icon_size) / 2.0];
            self.draw_copy_icon(frame, icon_pos, icon_size, config);
            right -= icon_size + frame.layout.padding;
        }
        let status_width = frame.ui.calc_text_size(&status)[0];
        let text_y = frame.text_y(top, height);
        frame.text([right - status_width, text_y], MUTED_COLOR, &status);

        let left_limit = right - status_width - frame.layout.padding * 2.0 - left;
        let left_text = fit_text(frame.ui, &left_text, left_limit);
        frame.text(
            [left + frame.layout.padding, text_y],
            MUTED_COLOR,
            &left_text,
        );

        frame.y += height + frame.layout.row_gap;
    }

    /// Clipboard icon opening the same two exports as the desktop copy menu.
    fn draw_copy_icon(&self, frame: &mut Frame, pos: [f32; 2], size: f32, config: &MeterConfig) {
        let ui = frame.ui;
        let (clicked, hovered) = frame.button("##overlay_copy", pos, [size, size]);
        draw_clipboard_icon(&frame.draw, pos, size, hovered);

        if hovered {
            ui.tooltip_text("复制到剪贴板");
        }
        if clicked {
            ui.open_popup(COPY_MENU_ID);
        }
        ui.popup(COPY_MENU_ID, || {
            if ui.menu_item("复制到剪贴板 (精简)") {
                self.request_copy(OverlayCopyKind::Simple, config);
            }
            if ui.menu_item("复制到剪贴板 (完整)") {
                self.request_copy(OverlayCopyKind::Full, config);
            }
        });
    }

    fn request_copy(&self, kind: OverlayCopyKind, config: &MeterConfig) {
        // Fails only when the desktop app is not connected.
        let _ = self
            .tx
            .send(Message::OverlayCopyRequest(OverlayCopyRequest {
                kind,
                sort: config.sort,
            }));
    }

    fn draw_table(&mut self, frame: &mut Frame, view: MeterView, config: &mut MeterConfig) {
        let state = view.state;
        let show_full = view.values.show_full_values;
        let mut players: Vec<&OverlayPlayer> = state.players.iter().collect();
        players.sort_by(|a, b| {
            let order = a
                .sort_value(config.sort.column)
                .total_cmp(&b.sort_value(config.sort.column));
            if config.sort.descending {
                order.reverse()
            } else {
                order
            }
        });

        // Each value column is as wide as its widest cell, so rows stay aligned.
        let columns: Vec<(OverlayColumn, &str)> = view
            .values
            .columns
            .iter()
            .map(|&column| (column, state.settings.column_label(column)))
            .collect();
        let values: Vec<Vec<String>> = players
            .iter()
            .map(|player| {
                columns
                    .iter()
                    .map(|&(column, _)| player_value(player, column, show_full))
                    .collect()
            })
            .collect();
        let widths: Vec<f32> = columns
            .iter()
            .enumerate()
            .map(|(index, (_, label))| {
                values
                    .iter()
                    .map(|row| frame.ui.calc_text_size(&row[index])[0])
                    .fold(frame.ui.calc_text_size(label)[0], f32::max)
            })
            .collect();
        let right_edges = column_right_edges(frame, &widths);
        let name_right = name_column_right(frame, &right_edges, &widths);

        // Header row: click to sort, like the desktop table headers.
        let height = frame.layout.row_height;
        let top = frame.y;
        let text_y = frame.text_y(top, height);
        let headers = std::iter::once((OverlayColumn::Name, "Name")).chain(columns.iter().copied());
        for (index, (column, label)) in headers.enumerate() {
            let label_width = frame.ui.calc_text_size(label)[0];
            // Cell area that reacts to clicks, and where its label starts.
            let (cell_left, cell_width, label_x) = if index == 0 {
                let label_x = frame.left + frame.layout.padding;
                (frame.left, name_right - frame.left, label_x)
            } else {
                let edge = right_edges[index - 1];
                let width = widths[index - 1];
                (edge - width, width, edge - label_width)
            };
            let (clicked, hovered) = frame.button(
                &format!("##sort{index}"),
                [cell_left, top],
                [cell_width, height],
            );
            if hovered {
                frame.highlight([cell_left, top], [cell_width, height]);
            }
            if clicked {
                // Same column flips the order; a new column starts ascending, as on desktop.
                if config.sort.column == column {
                    config.sort.descending = !config.sort.descending;
                } else {
                    config.sort.column = column;
                    config.sort.descending = false;
                }
            }
            frame.text([label_x, text_y], MUTED_COLOR, label);
            if config.sort.column == column {
                // Arrow after the name label, before right-aligned value labels.
                let (arrow_x, after) = if index == 0 {
                    (label_x + label_width, true)
                } else {
                    (label_x, false)
                };
                frame.sort_arrow(arrow_x, top, height, config.sort.descending, after);
            }
        }
        frame.y += height + frame.layout.row_gap;

        let top_damage = players
            .iter()
            .map(|player| player.total_damage)
            .max()
            .unwrap_or(0)
            .max(1) as f32;
        let segments = config.segment_by_attack_type && !state.attack_types.is_empty();
        if segments {
            draw_attack_legend(frame, &state.attack_types);
        }
        for (player, cells) in players.iter().zip(&values) {
            let types = segments.then_some(state.attack_types.as_slice());
            self.draw_player(
                frame,
                player,
                cells,
                &right_edges,
                name_right,
                top_damage,
                types,
            );
            if self.expanded_players.contains(&player.index) {
                draw_attack_composition(frame, player, &view.state.attack_types);
                self.draw_skills(frame, player, show_full);
            }
        }
    }

    fn draw_player(
        &mut self,
        frame: &mut Frame,
        player: &OverlayPlayer,
        cells: &[String],
        right_edges: &[f32],
        name_right: f32,
        top_damage: f32,
        attack_types: Option<&[OverlayAttackType]>,
    ) {
        let (left, top, width) = (frame.left, frame.y, frame.layout.width);
        let height = frame.layout.row_height;
        let rounding = frame.layout.rounding;

        frame
            .draw
            .add_rect([left, top], [left + width, top + height], TRACK_COLOR)
            .filled(true)
            .rounding(rounding)
            .build();
        let fill = (player.total_damage as f32 / top_damage).clamp(0.0, 1.0);
        let attack_total: u64 = player.attack_damage.iter().sum();
        match attack_types {
            // Same length as the plain bar, split by attack type; a stripe of the player's
            // colour on the left keeps rows apart.
            Some(types) if fill > 0.0 && attack_total > 0 => {
                let filled = width * fill;
                let mut x = left;
                for (kind, damage) in types.iter().zip(&player.attack_damage) {
                    if *damage == 0 {
                        continue;
                    }
                    let segment = filled * (*damage as f32 / attack_total as f32);
                    frame
                        .draw
                        .add_rect(
                            [x, top],
                            [(x + segment).min(left + filled), top + height],
                            color_with_alpha(kind.color, BAR_ALPHA),
                        )
                        .filled(true)
                        .build();
                    x += segment;
                }
                let stripe = (height * 0.25).min(filled);
                frame
                    .draw
                    .add_rect(
                        [left, top],
                        [left + stripe, top + height],
                        color_with_alpha(player.color, 255),
                    )
                    .filled(true)
                    .build();
            }
            _ if fill > 0.0 => {
                frame
                    .draw
                    .add_rect(
                        [left, top],
                        [left + width * fill, top + height],
                        color_with_alpha(player.color, BAR_ALPHA),
                    )
                    .filled(true)
                    .rounding(rounding)
                    .build();
            }
            _ => {}
        }

        let (clicked, hovered) = frame.button(
            &format!("##player{}", player.index),
            [left, top],
            [width, height],
        );
        if hovered {
            frame.highlight([left, top], [width, height]);
        }
        if clicked && !self.expanded_players.remove(&player.index) {
            self.expanded_players.insert(player.index);
        }

        let text_y = frame.text_y(top, height);
        let name_left = left + frame.layout.padding;
        let name = fit_text(frame.ui, &player.name, name_right - name_left);
        frame.text([name_left, text_y], TEXT_COLOR, &name);
        for (cell, edge) in cells.iter().zip(right_edges) {
            let cell_width = frame.ui.calc_text_size(cell)[0];
            frame.text([edge - cell_width, text_y], TEXT_COLOR, cell);
        }

        frame.y += height + frame.layout.row_gap;
    }

    /// The desktop skill breakdown table under an expanded player.
    fn draw_skills(&mut self, frame: &mut Frame, player: &OverlayPlayer, show_full: bool) {
        // Rows to draw, with their depth; children of expanded groups follow their group.
        let mut rows: Vec<(&OverlaySkill, usize)> = Vec::new();
        for skill in &player.skills {
            rows.push((skill, 0));
            let key = (player.index, skill.name.clone());
            if !skill.children.is_empty() && self.expanded_groups.contains(&key) {
                rows.extend(skill.children.iter().map(|child| (child, 1)));
            }
        }

        let cells: Vec<[String; 6]> = rows
            .iter()
            .map(|(skill, _)| skill_cells(skill, show_full))
            .collect();
        let widths: Vec<f32> = (0..SKILL_HEADERS.len())
            .map(|index| {
                cells
                    .iter()
                    .map(|row| frame.ui.calc_text_size(&row[index])[0])
                    .fold(frame.ui.calc_text_size(SKILL_HEADERS[index])[0], f32::max)
            })
            .collect();
        let right_edges = column_right_edges(frame, &widths);
        let name_right = name_column_right(frame, &right_edges, &widths);
        let indent = frame.layout.indent;
        let height = frame.ui.text_line_height() + 2.0 * frame.layout.row_gap;

        // Sub-header.
        let top = frame.y;
        let text_y = frame.text_y(top, height);
        frame.text([frame.left + indent, text_y], MUTED_COLOR, "Skill");
        for (label, edge) in SKILL_HEADERS.iter().zip(&right_edges) {
            let label_width = frame.ui.calc_text_size(label)[0];
            frame.text([edge - label_width, text_y], MUTED_COLOR, label);
        }
        frame.y += height;

        for ((skill, depth), row) in rows.iter().zip(&cells) {
            let top = frame.y;
            let row_left = frame.left + indent * (1 + depth) as f32;
            let row_width = frame.right() - row_left;
            let fill = (skill.percentage / 100.0).clamp(0.0, 1.0);
            if fill > 0.0 {
                frame
                    .draw
                    .add_rect(
                        [row_left, top],
                        [row_left + row_width * fill, top + height],
                        color_with_alpha(player.color, SKILL_BAR_ALPHA),
                    )
                    .filled(true)
                    .build();
            }

            let text_y = frame.text_y(top, height);
            let mut name_left = row_left;
            if !skill.children.is_empty() {
                let key = (player.index, skill.name.clone());
                let expanded = self.expanded_groups.contains(&key);
                let (clicked, hovered) = frame.button(
                    &format!("##group{}:{}", player.index, skill.name),
                    [row_left, top],
                    [row_width, height],
                );
                if hovered {
                    frame.highlight([row_left, top], [row_width, height]);
                }
                if clicked && !self.expanded_groups.remove(&key) {
                    self.expanded_groups.insert(key);
                }
                frame.disclosure_arrow(name_left, top, height, expanded);
                name_left += frame.ui.text_line_height();
            }
            let name = fit_text(frame.ui, &skill.name, name_right - name_left);
            frame.text([name_left, text_y], TEXT_COLOR, &name);
            for (cell, edge) in row.iter().zip(&right_edges) {
                let cell_width = frame.ui.calc_text_size(cell)[0];
                frame.text([edge - cell_width, text_y], TEXT_COLOR, cell);
            }
            frame.y += height;
        }
        frame.y += frame.layout.row_gap * 2.0;
    }
}

impl Frame<'_> {
    fn right(&self) -> f32 {
        self.left + self.layout.width
    }

    fn text_y(&self, top: f32, height: f32) -> f32 {
        top + (height - self.ui.text_line_height()) / 2.0
    }

    fn text(&self, pos: [f32; 2], color: ImColor32, text: &str) {
        self.draw
            .add_text([pos[0] + 1.0, pos[1] + 1.0], SHADOW_COLOR, text);
        self.draw.add_text(pos, color, text);
    }

    fn highlight(&self, pos: [f32; 2], size: [f32; 2]) {
        self.draw
            .add_rect(pos, [pos[0] + size[0], pos[1] + size[1]], HOVER_COLOR)
            .filled(true)
            .rounding(self.layout.rounding)
            .build();
    }

    /// Invisible button over an area; returns (clicked, hovered). Only reacts while the
    /// menu has released the mouse; a drag that moves the window is not a click.
    fn button(&self, id: &str, pos: [f32; 2], size: [f32; 2]) -> (bool, bool) {
        if !self.interactive || size[0] <= 0.0 || size[1] <= 0.0 {
            return (false, false);
        }
        self.ui.set_cursor_screen_pos(pos);
        let clicked = self.ui.invisible_button(id, size);
        let hovered = self.ui.is_item_hovered();
        (
            clicked && !self.ui.is_mouse_dragging(MouseButton::Left),
            hovered,
        )
    }

    /// Small triangle beside a sorted header: down for descending, up for ascending. `after`
    /// places it right of `x`, otherwise left of it.
    fn sort_arrow(&self, x: f32, top: f32, height: f32, descending: bool, after: bool) {
        let size = self.ui.text_line_height() * 0.3;
        let center_x = if after {
            x + size * 1.5
        } else {
            x - size * 1.5
        };
        let center_y = top + height / 2.0;
        let (tip, base) = if descending {
            (center_y + size * 0.5, center_y - size * 0.5)
        } else {
            (center_y - size * 0.5, center_y + size * 0.5)
        };
        self.draw
            .add_triangle(
                [center_x - size, base],
                [center_x + size, base],
                [center_x, tip],
                MUTED_COLOR,
            )
            .filled(true)
            .build();
    }

    /// Right-pointing triangle for a collapsed group, down-pointing when expanded.
    fn disclosure_arrow(&self, x: f32, top: f32, height: f32, expanded: bool) {
        let size = self.ui.text_line_height() * 0.28;
        let center = [x + size * 1.6, top + height / 2.0];
        let points = if expanded {
            [
                [center[0] - size, center[1] - size * 0.6],
                [center[0] + size, center[1] - size * 0.6],
                [center[0], center[1] + size * 0.8],
            ]
        } else {
            [
                [center[0] - size * 0.6, center[1] - size],
                [center[0] - size * 0.6, center[1] + size],
                [center[0] + size * 0.8, center[1]],
            ]
        };
        self.draw
            .add_triangle(points[0], points[1], points[2], MUTED_COLOR)
            .filled(true)
            .build();
    }

    fn placeholder(&mut self, message: &str) {
        let text_y = self.y;
        self.text([self.left, text_y], MUTED_COLOR, message);
        self.y += self.ui.text_line_height();
    }

    /// Reserves everything drawn so far so the auto-resizing window wraps it. Clickable areas
    /// moved the layout cursor, so this starts from the top again.
    fn finish(self) {
        self.ui.set_cursor_screen_pos([self.left, self.top]);
        self.ui
            .dummy([self.layout.width, (self.y - self.top).max(1.0)]);
    }
}

/// Right edges of the value columns, laid out from the right side of the meter.
fn column_right_edges(frame: &Frame, widths: &[f32]) -> Vec<f32> {
    let mut edge = frame.right() - frame.layout.padding;
    let mut edges = vec![0.0; widths.len()];
    for index in (0..widths.len()).rev() {
        edges[index] = edge;
        edge -= widths[index] + frame.layout.column_gap;
    }
    edges
}

/// Right edge of the name column: just left of the first value column.
fn name_column_right(frame: &Frame, right_edges: &[f32], widths: &[f32]) -> f32 {
    match (right_edges.first(), widths.first()) {
        (Some(edge), Some(width)) => edge - width - frame.layout.column_gap,
        _ => frame.right() - frame.layout.padding,
    }
}

fn player_value(player: &OverlayPlayer, column: OverlayColumn, show_full: bool) -> String {
    match column {
        OverlayColumn::Name => player.name.clone(),
        OverlayColumn::TotalDamage => format_amount(player.total_damage as f64, show_full),
        OverlayColumn::Dps => format_amount(player.dps, show_full),
        OverlayColumn::DamagePercentage => format!("{:.0}%", player.percentage),
        OverlayColumn::Sba if show_full => format!("{:.2}", player.sba / 10.0),
        OverlayColumn::Sba => format!("{:.2}%", player.sba / 10.0),
        OverlayColumn::TotalStunValue => format_amount(player.total_stun_value, show_full),
        OverlayColumn::StunPerSecond => locale_number(player.stun_per_second),
    }
}

/// Hits, Total, Min, Max, Avg and % as the desktop skill rows show them.
fn skill_cells(skill: &OverlaySkill, show_full: bool) -> [String; 6] {
    let optional = |value: Option<u64>| match value {
        Some(value) if value > 0 => format_amount(value as f64, show_full),
        _ => String::new(),
    };
    let average = if skill.hits == 0 {
        0.0
    } else {
        skill.total_damage as f64 / skill.hits as f64
    };
    [
        skill.hits.to_string(),
        format_amount(skill.total_damage as f64, show_full),
        optional(skill.min_damage),
        optional(skill.max_damage),
        format_amount(average, show_full),
        format!("{:.0}%", skill.percentage),
    ]
}

/// Above segmented bars: what each colour stands for, wrapped to the meter width.
fn draw_attack_legend(frame: &mut Frame, types: &[OverlayAttackType]) {
    let left = frame.left + frame.layout.padding;
    let right = frame.right();
    let gap = frame.layout.row_gap.max(2.0);
    let line_height = frame.ui.text_line_height();
    let swatch = line_height * 0.6;
    let spacing = line_height * 0.8;
    let mut x = left;
    for kind in types {
        let item_width = swatch + 4.0 + frame.ui.calc_text_size(&kind.label)[0];
        if x > left && x + item_width > right {
            x = left;
            frame.y += line_height + gap;
        }
        let swatch_top = frame.y + (line_height - swatch) / 2.0;
        frame
            .draw
            .add_rect(
                [x, swatch_top],
                [x + swatch, swatch_top + swatch],
                color_with_alpha(kind.color, 255),
            )
            .filled(true)
            .build();
        frame.text([x + swatch + 4.0, frame.y], MUTED_COLOR, &kind.label);
        x += item_width + spacing;
    }
    frame.y += line_height + gap;
}

/// Under an expanded player: a stacked bar of the damage per attack type, then a legend with
/// each type's share, wrapped to the meter width.
fn draw_attack_composition(frame: &mut Frame, player: &OverlayPlayer, types: &[OverlayAttackType]) {
    let total: u64 = player.attack_damage.iter().sum();
    if total == 0 || types.is_empty() {
        return;
    }
    let shares: Vec<(&OverlayAttackType, f32)> = types
        .iter()
        .zip(&player.attack_damage)
        .filter(|(_, damage)| **damage > 0)
        .map(|(kind, damage)| (kind, *damage as f32 / total as f32))
        .collect();

    let left = frame.left + frame.layout.indent;
    let width = frame.right() - left;
    let gap = frame.layout.row_gap.max(2.0);
    let bar_height = (frame.ui.text_line_height() * 0.45).max(4.0);

    frame.y += gap;
    let mut x = left;
    for (kind, share) in &shares {
        let segment = width * share;
        frame
            .draw
            .add_rect(
                [x, frame.y],
                [x + segment, frame.y + bar_height],
                color_with_alpha(kind.color, 230),
            )
            .filled(true)
            .build();
        x += segment;
    }
    frame.y += bar_height + gap;

    // Legend: colour swatch, label and share, flowing onto new lines when full.
    let line_height = frame.ui.text_line_height();
    let swatch = line_height * 0.6;
    let spacing = line_height * 0.8;
    let mut x = left;
    for (kind, share) in &shares {
        let text = format!("{} {:.1}%", kind.label, share * 100.0);
        let item_width = swatch + 4.0 + frame.ui.calc_text_size(&text)[0];
        if x > left && x + item_width > left + width {
            x = left;
            frame.y += line_height + gap;
        }
        let swatch_top = frame.y + (line_height - swatch) / 2.0;
        frame
            .draw
            .add_rect(
                [x, swatch_top],
                [x + swatch, swatch_top + swatch],
                color_with_alpha(kind.color, 255),
            )
            .filled(true)
            .build();
        frame.text([x + swatch + 4.0, frame.y], TEXT_COLOR, &text);
        x += item_width + spacing;
    }
    frame.y += line_height + gap;
}

fn color_with_alpha(color: u32, alpha: u8) -> ImColor32 {
    let [_, r, g, b] = color.to_be_bytes();
    ImColor32::from_rgba(r, g, b, alpha)
}

/// Outline icon used by the desktop clipboard button: a board, its clip and two text lines.
fn draw_clipboard_icon(draw_list: &DrawListMut<'_>, [x, y]: [f32; 2], size: f32, hovered: bool) {
    if hovered {
        draw_list
            .add_rect([x, y], [x + size, y + size], HOVER_COLOR)
            .filled(true)
            .rounding(size * 0.2)
            .build();
    }
    let at = |fx: f32, fy: f32| [x + size * fx, y + size * fy];
    let stroke = (size * 0.08).max(1.0);
    draw_list
        .add_rect(at(0.2, 0.18), at(0.8, 0.9), TEXT_COLOR)
        .thickness(stroke)
        .rounding(size * 0.08)
        .build();
    draw_list
        .add_rect(at(0.36, 0.08), at(0.64, 0.26), TEXT_COLOR)
        .filled(true)
        .rounding(size * 0.05)
        .build();
    for fy in [0.48, 0.66] {
        draw_list
            .add_line(at(0.33, fy), at(0.67, fy), TEXT_COLOR)
            .thickness(stroke)
            .build();
    }
}

/// Pulls a saved position back on screen, e.g. after switching to a smaller resolution.
fn on_screen(position: [f32; 2], display: [f32; 2]) -> [f32; 2] {
    if display[0] <= 0.0 || display[1] <= 0.0 {
        return position;
    }
    [
        position[0].clamp(0.0, (display[0] - MIN_VISIBLE[0]).max(0.0)),
        position[1].clamp(0.0, (display[1] - MIN_VISIBLE[1]).max(0.0)),
    ]
}

/// Trims `text` with an ellipsis so it fits `max_width`.
fn fit_text(ui: &Ui, text: &str, max_width: f32) -> String {
    if ui.calc_text_size(text)[0] <= max_width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>());
        if ui.calc_text_size(&candidate)[0] <= max_width {
            return candidate;
        }
    }
    String::new()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// `MM:SS`, matching the desktop meter's title bar.
fn format_elapsed(ms: i64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60 % 60, seconds % 60)
}

/// Full value with separators, or the short form with a unit, per the settings page.
fn format_amount(value: f64, show_full: bool) -> String {
    if show_full {
        locale_number(value)
    } else {
        humanize(value)
    }
}

/// Same thresholds and units as the desktop meter's `humanizeNumbers`.
fn humanize(value: f64) -> String {
    const UNITS: [(f64, &str); 4] = [(1e12, "t"), (1e9, "b"), (1e6, "m"), (1e3, "k")];
    for (scale, unit) in UNITS {
        if value >= scale {
            return format!("{}{unit}", trim_decimal(value / scale));
        }
    }
    format!("{value:.0}")
}

/// One decimal place without a trailing `.0`, like `+(n).toFixed(1)` in JavaScript.
fn trim_decimal(value: f64) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0").map(str::to_string).unwrap_or(text)
}

/// Like JavaScript's `toLocaleString()` in English: thousands separators and up to three
/// decimals without trailing zeros.
fn locale_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let rounded = format!("{:.3}", value.abs());
    let (integer, fraction) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let mut grouped = String::new();
    for (index, digit) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let fraction = fraction.trim_end_matches('0');
    let sign = if value < 0.0 && (grouped != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_matches_desktop_meter() {
        assert_eq!(humanize(999.0), "999");
        assert_eq!(humanize(1_000.0), "1k");
        assert_eq!(humanize(12_345.0), "12.3k");
        assert_eq!(humanize(2_500_000.0), "2.5m");
        assert_eq!(humanize(3_000_000_000.0), "3b");
    }

    #[test]
    fn locale_number_matches_to_locale_string() {
        assert_eq!(locale_number(0.0), "0");
        assert_eq!(locale_number(999.0), "999");
        assert_eq!(locale_number(1_234_567.0), "1,234,567");
        assert_eq!(locale_number(1234.5678), "1,234.568");
        assert_eq!(locale_number(12.5), "12.5");
    }

    #[test]
    fn skill_cells_follow_desktop_formats() {
        let skill = OverlaySkill {
            name: "Skill".into(),
            hits: 4,
            total_damage: 10_000,
            min_damage: Some(1_000),
            max_damage: None,
            percentage: 25.4,
            children: Vec::new(),
        };
        assert_eq!(
            skill_cells(&skill, false),
            ["4", "10k", "1k", "", "2.5k", "25%"].map(String::from)
        );
        assert_eq!(
            skill_cells(&skill, true),
            ["4", "10,000", "1,000", "", "2,500", "25%"].map(String::from)
        );
    }

    #[test]
    fn saved_positions_are_kept_on_screen() {
        let display = [1920.0, 1080.0];
        assert_eq!(on_screen([100.0, 200.0], display), [100.0, 200.0]);
        assert_eq!(on_screen([3000.0, -50.0], display), [1840.0, 0.0]);
        // Without a known display size the position is left alone.
        assert_eq!(on_screen([3000.0, 10.0], [0.0, 0.0]), [3000.0, 10.0]);
    }

    #[test]
    fn elapsed_formats_as_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(83_000), "01:23");
    }
}
