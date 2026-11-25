//! Overlay settings persisted to `overlay.json` next to the hook's log file.
//!
//! Every field has a default and unknown or missing fields fall back to it, so older and newer
//! files keep loading. Values are clamped on load because the file is user-editable.

use std::path::PathBuf;

use log::{info, warn};
use protocol::OverlaySort;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "overlay.json";

pub const METER_WIDTH_RANGE: (f32, f32) = (240.0, 900.0);
pub const METER_SCALE_RANGE: (f32, f32) = (0.6, 2.0);
pub const ENEMY_HP_WIDTH_RANGE: (f32, f32) = (160.0, 900.0);
pub const ENEMY_HP_COUNT_RANGE: (u32, u32) = (1, 10);
pub const COMBAT_LOG_HEIGHT_RANGE: (f32, f32) = (60.0, 900.0);

pub fn data_dir() -> Option<PathBuf> {
    protocol::paths::data_dir()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct OverlayConfig {
    pub show_meter: bool,
    pub meter: MeterConfig,
    pub show_enemy_hp: bool,
    pub enemy_hp: EnemyHpConfig,
    pub show_damage_taken: bool,
    pub damage_taken: DamageTakenConfig,
    pub show_combat_log: bool,
    pub combat_log: CombatLogConfig,
    pub font: FontSettings,
    /// Opens and closes the in-game menu.
    pub menu_hotkey: super::hotkey::KeyCombo,
    /// Menu sections the player folded.
    pub collapsed_sections: Vec<String>,
    /// Top-left corner of the settings menu; `None` until it has been moved.
    pub menu_position: Option<[f32; 2]>,
    /// Experimental: marks tracked actors in the 3D scene. Off until the player turns it on;
    /// remembered from then on, like what it shows (`debug`).
    pub debug_view: bool,
    /// What the debug view draws.
    pub debug: DebugViewConfig,
    /// The display the window positions were saved on; they are scaled when the game runs at
    /// another size. `None` in settings saved before it was kept.
    pub layout_display: Option<[f32; 2]>,
}

impl OverlayConfig {
    /// Scales the saved positions and window sizes to `display` when it differs from the display
    /// they were laid out on. Returns whether anything moved.
    ///
    /// Settings from before `layout_display` was kept are taken as laid out on this display when
    /// every window fits on it whole, otherwise on the reference display the defaults use.
    pub fn adapt_to_display(&mut self, display: [f32; 2]) -> bool {
        if display[0] <= 0.0 || display[1] <= 0.0 {
            return false;
        }
        // Each window's saved corner and roughly its size (the smallest it is drawn at).
        let windows = [
            (
                self.meter.position,
                [self.meter.width * self.meter.scale, 60.0],
            ),
            (
                self.enemy_hp.position,
                [self.enemy_hp.width * self.enemy_hp.scale, 40.0],
            ),
            (
                self.damage_taken.position,
                [self.damage_taken.width * self.damage_taken.scale, 40.0],
            ),
            (
                self.combat_log.position,
                [
                    self.combat_log.width * self.combat_log.scale,
                    self.combat_log.height * self.combat_log.scale,
                ],
            ),
        ];
        let layout = self.layout_display.unwrap_or_else(|| {
            let fits = windows.iter().all(|(position, size)| {
                position.map_or(true, |p| {
                    p[0] + size[0] <= display[0] && p[1] + size[1] <= display[1]
                })
            });
            if fits {
                display
            } else {
                super::placement::REFERENCE_DISPLAY
            }
        });
        self.layout_display = Some(display);
        if (layout[0] - display[0]).abs() < 1.0 && (layout[1] - display[1]).abs() < 1.0 {
            return false;
        }
        let [sx, sy] = [display[0] / layout[0], display[1] / layout[1]];
        let scale_position = |position: &mut Option<[f32; 2]>| {
            if let Some(p) = position.as_mut() {
                *p = [p[0] * sx, p[1] * sy];
            }
        };
        scale_position(&mut self.meter.position);
        scale_position(&mut self.enemy_hp.position);
        scale_position(&mut self.damage_taken.position);
        scale_position(&mut self.combat_log.position);
        scale_position(&mut self.menu_position);
        // Sizes follow the height, so the windows keep their share of the screen.
        let scale = |value: f32| clamp_or(value * sy, METER_SCALE_RANGE, value);
        self.meter.scale = scale(self.meter.scale);
        self.enemy_hp.scale = scale(self.enemy_hp.scale);
        self.damage_taken.scale = scale(self.damage_taken.scale);
        self.combat_log.scale = scale(self.combat_log.scale);
        true
    }
}

/// Parts of the debug view, each shown or hidden on its own.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct DebugViewConfig {
    pub status_line: bool,
    pub enemies: bool,
    pub hurtboxes: bool,
    pub facing: bool,
    pub hit_points: bool,
    pub party: bool,
    pub npcs: bool,
    pub items: bool,
    pub objects: bool,
    pub body_collision: bool,
    pub terrain: bool,
    /// The game draws a 16:9 picture centred in the window (black bars around it); off when it
    /// fills the window, e.g. on ultrawide screens.
    pub fit_16_9: bool,
}

impl Default for DebugViewConfig {
    fn default() -> Self {
        Self {
            status_line: true,
            enemies: true,
            hurtboxes: true,
            facing: true,
            hit_points: true,
            party: true,
            npcs: true,
            items: true,
            objects: true,
            body_collision: true,
            terrain: true,
            fit_16_9: true,
        }
    }
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            show_meter: true,
            meter: MeterConfig::default(),
            show_enemy_hp: true,
            enemy_hp: EnemyHpConfig::default(),
            show_damage_taken: true,
            damage_taken: DamageTakenConfig::default(),
            show_combat_log: protocol::OverlayWindow::CombatLog.default_visible(),
            combat_log: CombatLogConfig::default(),
            font: FontSettings::default(),
            menu_hotkey: super::hotkey::KeyCombo::default(),
            collapsed_sections: Vec::new(),
            menu_position: None,
            debug_view: false,
            debug: DebugViewConfig::default(),
            layout_display: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct FontSettings {
    /// Font file chosen in the menu; `None` uses the system default (Microsoft YaHei).
    pub path: Option<String>,
    /// Glyph size in pixels the atlas is rasterised at.
    pub size: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            path: None,
            size: super::fonts::DEFAULT_FONT_SIZE,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct MeterConfig {
    /// Top-left corner in screen pixels; `None` until the window has been placed.
    pub position: Option<[f32; 2]>,
    /// Width of the bars in unscaled pixels.
    pub width: f32,
    /// Multiplier for text and row sizes.
    pub scale: f32,
    /// Column the players are ordered by, chosen by clicking a header.
    pub sort: OverlaySort,
    /// Split each player's bar by attack type (normal, skill, SBA, ...) instead of one colour.
    pub segment_by_attack_type: bool,
}

impl Default for MeterConfig {
    fn default() -> Self {
        Self {
            position: None,
            width: 505.0,
            scale: 1.0,
            sort: OverlaySort::default(),
            segment_by_attack_type: false,
        }
    }
}

/// The enemy HP window.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct EnemyHpConfig {
    /// Top-left corner in screen pixels; `None` until the window has been placed.
    pub position: Option<[f32; 2]>,
    /// Width of the bars in unscaled pixels.
    pub width: f32,
    pub scale: f32,
    /// How many enemies are listed, biggest max HP first.
    pub max_enemies: u32,
}

impl Default for EnemyHpConfig {
    fn default() -> Self {
        Self {
            position: None,
            width: 360.0,
            scale: 1.0,
            max_enemies: 4,
        }
    }
}

/// The damage taken window.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct DamageTakenConfig {
    pub position: Option<[f32; 2]>,
    pub width: f32,
    pub scale: f32,
}

impl Default for DamageTakenConfig {
    fn default() -> Self {
        Self {
            position: None,
            width: 300.0,
            scale: 1.0,
        }
    }
}

/// The combat log window.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct CombatLogConfig {
    pub position: Option<[f32; 2]>,
    pub width: f32,
    /// Height of the scrolling area in unscaled pixels.
    pub height: f32,
    pub scale: f32,
}

impl Default for CombatLogConfig {
    fn default() -> Self {
        Self {
            position: None,
            width: 460.0,
            height: 180.0,
            scale: 1.0,
        }
    }
}

impl OverlayConfig {
    /// Whether an independent window is shown.
    pub fn window_shown(&self, window: protocol::OverlayWindow) -> &bool {
        match window {
            protocol::OverlayWindow::Meter => &self.show_meter,
            protocol::OverlayWindow::EnemyHp => &self.show_enemy_hp,
            protocol::OverlayWindow::DamageTaken => &self.show_damage_taken,
            protocol::OverlayWindow::CombatLog => &self.show_combat_log,
        }
    }

    pub fn window_shown_mut(&mut self, window: protocol::OverlayWindow) -> &mut bool {
        match window {
            protocol::OverlayWindow::Meter => &mut self.show_meter,
            protocol::OverlayWindow::EnemyHp => &mut self.show_enemy_hp,
            protocol::OverlayWindow::DamageTaken => &mut self.show_damage_taken,
            protocol::OverlayWindow::CombatLog => &mut self.show_combat_log,
        }
    }

    fn sanitized(mut self) -> Self {
        let enemy_defaults = EnemyHpConfig::default();
        let enemy = &mut self.enemy_hp;
        enemy.width = clamp_or(enemy.width, ENEMY_HP_WIDTH_RANGE, enemy_defaults.width);
        enemy.scale = clamp_or(enemy.scale, METER_SCALE_RANGE, enemy_defaults.scale);
        enemy.max_enemies = enemy
            .max_enemies
            .clamp(ENEMY_HP_COUNT_RANGE.0, ENEMY_HP_COUNT_RANGE.1);
        if enemy
            .position
            .is_some_and(|[x, y]| !x.is_finite() || !y.is_finite())
        {
            enemy.position = None;
        }
        let log_defaults = CombatLogConfig::default();
        let log = &mut self.combat_log;
        log.width = clamp_or(log.width, ENEMY_HP_WIDTH_RANGE, log_defaults.width);
        log.height = clamp_or(log.height, COMBAT_LOG_HEIGHT_RANGE, log_defaults.height);
        log.scale = clamp_or(log.scale, METER_SCALE_RANGE, log_defaults.scale);
        if log
            .position
            .is_some_and(|[x, y]| !x.is_finite() || !y.is_finite())
        {
            log.position = None;
        }
        if self
            .menu_position
            .is_some_and(|[x, y]| !x.is_finite() || !y.is_finite())
        {
            self.menu_position = None;
        }
        let taken_defaults = DamageTakenConfig::default();
        let taken = &mut self.damage_taken;
        taken.width = clamp_or(taken.width, ENEMY_HP_WIDTH_RANGE, taken_defaults.width);
        taken.scale = clamp_or(taken.scale, METER_SCALE_RANGE, taken_defaults.scale);
        if taken
            .position
            .is_some_and(|[x, y]| !x.is_finite() || !y.is_finite())
        {
            taken.position = None;
        }

        let defaults = MeterConfig::default();
        let meter = &mut self.meter;
        meter.width = clamp_or(meter.width, METER_WIDTH_RANGE, defaults.width);
        meter.scale = clamp_or(meter.scale, METER_SCALE_RANGE, defaults.scale);
        self.font.size = clamp_or(
            self.font.size,
            super::fonts::FONT_SIZE_RANGE,
            super::fonts::DEFAULT_FONT_SIZE,
        );
        if !self.menu_hotkey.is_valid() {
            self.menu_hotkey = super::hotkey::KeyCombo::default();
        }
        if self.font.path.as_deref().is_some_and(str::is_empty) {
            self.font.path = None;
        }
        let meter = &mut self.meter;
        if meter
            .position
            .is_some_and(|[x, y]| !x.is_finite() || !y.is_finite())
        {
            meter.position = None;
        }
        self
    }
}

fn clamp_or(value: f32, (min, max): (f32, f32), fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

pub fn load() -> OverlayConfig {
    let Some(path) = data_dir().map(|dir| dir.join(FILE_NAME)) else {
        return OverlayConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return OverlayConfig::default();
    };
    match serde_json::from_str::<OverlayConfig>(&text) {
        Ok(config) => config.sanitized(),
        Err(error) => {
            warn!("Ignoring unreadable {}: {error}", path.display());
            OverlayConfig::default()
        }
    }
}

/// Writes the config on a background thread so the render thread never waits on disk.
/// The file is replaced atomically so a crash cannot leave it half-written.
pub fn save(config: &OverlayConfig) {
    let Some(dir) = data_dir() else {
        return;
    };
    let Ok(text) = serde_json::to_string_pretty(config) else {
        return;
    };
    crate::shutdown::spawn(move || {
        let path = dir.join(FILE_NAME);
        let temp = dir.join(format!("{FILE_NAME}.tmp"));
        let result = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&temp, text))
            .and_then(|()| std::fs::rename(&temp, &path));
        match result {
            Ok(()) => info!("Overlay settings saved"),
            Err(error) => warn!("Could not save {}: {error}", path.display()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_view_is_off_by_default_and_remembered() {
        assert!(!OverlayConfig::default().debug_view);
        // Settings saved before it was remembered have no such field.
        let old: OverlayConfig = serde_json::from_str(r#"{"meter":{"width":500}}"#).unwrap();
        assert!(!old.debug_view);
        let config = OverlayConfig {
            debug_view: true,
            ..OverlayConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: OverlayConfig = serde_json::from_str(&json).unwrap();
        assert!(loaded.debug_view);
    }

    #[test]
    fn layout_follows_the_display() {
        // Laid out on 2560x1600 (an older file without `layout_display`: the meter lies off a
        // 1080p screen), then shown at 1920x1080.
        let mut config = OverlayConfig::default();
        config.meter.position = Some([1065.0, 1449.0]);
        config.meter.scale = 1.0;
        assert!(config.adapt_to_display([1920.0, 1080.0]));
        let [x, y] = config.meter.position.unwrap();
        assert!((x - 798.75).abs() < 0.01 && (y - 978.075).abs() < 0.01);
        assert!((config.meter.scale - 0.675).abs() < 0.001);
        assert_eq!(config.layout_display, Some([1920.0, 1080.0]));
        // Same display again: nothing moves.
        assert!(!config.adapt_to_display([1920.0, 1080.0]));
    }

    #[test]
    fn older_layouts_that_fit_are_kept() {
        let mut config = OverlayConfig::default();
        config.meter.position = Some([400.0, 700.0]);
        assert!(!config.adapt_to_display([1920.0, 1080.0]));
        assert_eq!(config.meter.position, Some([400.0, 700.0]));
    }

    #[test]
    fn older_layouts_are_judged_by_whole_windows() {
        // Every corner lies on 1920x1080, but the meter's bottom and the damage taken window's
        // right side do not: laid out on the larger reference display.
        let mut config = OverlayConfig::default();
        config.meter.position = Some([1065.0, 1040.0]);
        config.damage_taken.position = Some([1864.0, 9.0]);
        assert!(config.adapt_to_display([1920.0, 1080.0]));
        assert!(config.meter.position.unwrap()[1] < 1040.0 * 0.7);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let config: OverlayConfig = serde_json::from_str(r#"{"meter":{"width":500}}"#).unwrap();
        assert!(config.show_meter);
        assert_eq!(config.meter.width, 500.0);
        assert_eq!(config.meter.scale, 1.0);
        assert_eq!(config.meter.position, None);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let config: OverlayConfig =
            serde_json::from_str(r#"{"meter":{"width":5,"scale":10,"position":[10,20]}}"#).unwrap();
        let config = config.sanitized();
        assert_eq!(config.meter.width, METER_WIDTH_RANGE.0);
        assert_eq!(config.meter.scale, METER_SCALE_RANGE.1);
        assert_eq!(config.meter.position, Some([10.0, 20.0]));
    }
}
