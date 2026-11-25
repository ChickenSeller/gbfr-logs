//! In-game combat log: the latest lines from the app's snapshots in a scrolling area.

use hudhook::imgui::{Condition, Ui, WindowFlags};
use protocol::OverlayLogLine;

use super::config::CombatLogConfig;

/// Default layout: a row along the top edge, after the enemy HP window.
const DEFAULT_POSITION: [f32; 2] = [875.0, 25.0];
const MIN_INTERACTIVE_ALPHA: f32 = 0.6;
const TIME_COLOR: [f32; 4] = [0.65, 0.65, 0.65, 1.0];

#[derive(Default)]
pub struct CombatLogWindow {
    reset_position: bool,
    /// Its size and the display it was placed on, to keep it on screen at any resolution.
    placement: super::placement::Placement,
    /// Text of the newest line drawn, to scroll down when a new one arrives.
    last_line: Option<(String, String)>,
}

impl CombatLogWindow {
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
        config: &mut CombatLogConfig,
        lines: &[OverlayLogLine],
        transparency: f32,
    ) {
        // Shown whenever it is enabled, even without data, so it matches its checkbox.

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

        let newest = lines
            .last()
            .map(|line| (line.time.clone(), line.text.clone()));
        let has_new_line = newest != self.last_line;
        self.last_line = newest;

        let mut window_size = [0.0, 0.0];
        ui.window("GBFR Logs 战斗日志")
            .position(position, condition)
            .bg_alpha(alpha.clamp(0.0, 1.0))
            .flags(flags)
            .build(|| {
                ui.set_window_font_scale(config.scale);
                let size = [config.width * config.scale, config.height * config.scale];
                let mut child_flags = WindowFlags::empty();
                if !interactive {
                    child_flags |= WindowFlags::NO_SCROLLBAR | WindowFlags::NO_INPUTS;
                }
                ui.child_window("##combat_log_lines")
                    .size(size)
                    .flags(child_flags)
                    .build(|| {
                        ui.set_window_font_scale(config.scale);
                        if lines.is_empty() {
                            ui.text_disabled(if interactive {
                                "战斗日志：战斗开始后显示  菜单打开时可拖动此窗口"
                            } else {
                                "战斗日志：暂无数据"
                            });
                        }
                        // Follow new lines unless the player scrolled up to read.
                        let at_bottom = ui.scroll_y() >= ui.scroll_max_y() - 2.0;
                        for line in lines {
                            ui.text_colored(TIME_COLOR, &line.time);
                            ui.same_line();
                            ui.text_colored(rgb(line.color), &line.text);
                        }
                        if has_new_line && (at_bottom || !interactive) {
                            ui.set_scroll_here_y_with_ratio(1.0);
                        }
                    });
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

fn rgb(color: u32) -> [f32; 4] {
    let [_, r, g, b] = color.to_be_bytes();
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}
