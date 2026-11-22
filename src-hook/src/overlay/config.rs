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

pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("gbfr-logs-awa"))
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
    pub font: FontSettings,
    /// Opens and closes the in-game menu.
    pub menu_hotkey: super::hotkey::KeyCombo,
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
            font: FontSettings::default(),
            menu_hotkey: super::hotkey::KeyCombo::default(),
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
}

impl Default for MeterConfig {
    fn default() -> Self {
        Self {
            position: None,
            width: 505.0,
            scale: 1.0,
            sort: OverlaySort::default(),
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

impl OverlayConfig {
    /// Whether an independent window is shown.
    pub fn window_shown(&self, window: protocol::OverlayWindow) -> &bool {
        match window {
            protocol::OverlayWindow::Meter => &self.show_meter,
            protocol::OverlayWindow::EnemyHp => &self.show_enemy_hp,
            protocol::OverlayWindow::DamageTaken => &self.show_damage_taken,
        }
    }

    pub fn window_shown_mut(&mut self, window: protocol::OverlayWindow) -> &mut bool {
        match window {
            protocol::OverlayWindow::Meter => &mut self.show_meter,
            protocol::OverlayWindow::EnemyHp => &mut self.show_enemy_hp,
            protocol::OverlayWindow::DamageTaken => &mut self.show_damage_taken,
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
    std::thread::spawn(move || {
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
