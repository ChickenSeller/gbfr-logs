//! Messages between the desktop app and the in-game overlay.
//!
//! [`OverlayMessage`] flows from the app to the hook: the app owns the parsed encounter,
//! translations and meter settings, so it sends a display-ready snapshot and the overlay draws
//! it. Sorting and expanded rows are overlay UI state and stay in the hook. Meter settings
//! travel both ways: they arrive in [`OverlaySettings`] and in-game edits go back as
//! [`crate::Message::OverlaySettingsChange`], so the settings page stays the single source of
//! truth. The hook and the
//! app are always shipped together, so the format carries no compatibility layer; never
//! persist these messages.
//!
//! Wire format: bincode-encoded [`OverlayMessage`], each prefixed by its length as a 4-byte
//! big-endian integer (tokio-util's `LengthDelimitedCodec` default).

use serde::{Deserialize, Serialize};

/// Named pipe served by the hook and written by the app.
pub const OVERLAY_PIPE_NAME: &str = r"\\.\pipe\gbfr-logs-overlay";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum OverlayMessage {
    /// Latest meter snapshot. Each message replaces the previous one.
    State(OverlayState),
    /// Short status line shown briefly in the overlay (saves, copy results, alerts).
    Notice(OverlayNotice),
    /// Open or close the in-game menu, e.g. from the tray when no hotkey is usable.
    ToggleMenu,
    /// Show or hide one of the in-game windows.
    ToggleWindow(OverlayWindow),
}

/// The in-game windows that can be shown or hidden on their own.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayWindow {
    Meter,
    EnemyHp,
    DamageTaken,
}

impl OverlayWindow {
    /// Every window, in the order the tray and the menu list them.
    pub const ALL: [OverlayWindow; 3] = [
        OverlayWindow::Meter,
        OverlayWindow::EnemyHp,
        OverlayWindow::DamageTaken,
    ];

    /// Id of the window's check item in the tray menu.
    pub fn tray_id(self) -> &'static str {
        match self {
            OverlayWindow::Meter => "toggle_overlay_meter",
            OverlayWindow::EnemyHp => "toggle_overlay_enemy_hp",
            OverlayWindow::DamageTaken => "toggle_overlay_damage_taken",
        }
    }

    /// Tray label until the translations are loaded.
    pub fn default_label(self) -> &'static str {
        match self {
            OverlayWindow::Meter => "Show In-game Meter",
            OverlayWindow::EnemyHp => "Show Enemy HP",
            OverlayWindow::DamageTaken => "Show Damage Taken",
        }
    }

    pub fn from_tray_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|window| window.tray_id() == id)
    }
}

/// What the in-game overlay currently shows, reported to the app so its tray menu can mirror
/// it with check marks.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct OverlayUiState {
    /// The windows currently shown.
    pub visible_windows: Vec<OverlayWindow>,
    pub menu_open: bool,
    /// The menu hotkey as shown to the user, e.g. `Ctrl+F8`.
    pub menu_hotkey: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OverlayNotice {
    pub text: String,
    pub level: OverlayNoticeLevel,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayNoticeLevel {
    Success,
    Error,
}

/// Which clipboard export the overlay asked for; mirrors the desktop copy menu.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayCopyKind {
    /// Encounter totals and one line per player.
    Simple,
    /// Totals, players and every player's skill breakdown.
    Full,
}

/// Copy request sent from the overlay; exports follow the overlay's current sort order.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCopyRequest {
    pub kind: OverlayCopyKind,
    pub sort: OverlaySort,
}

/// Meter columns. The JSON names match the front-end's `MeterColumns` values.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayColumn {
    #[serde(rename = "name")]
    Name,
    #[serde(rename = "damage")]
    TotalDamage,
    #[serde(rename = "dps")]
    Dps,
    #[serde(rename = "damage-percentage")]
    DamagePercentage,
    #[serde(rename = "sba")]
    Sba,
    #[serde(rename = "total-stun-value")]
    TotalStunValue,
    #[serde(rename = "stun-per-second")]
    StunPerSecond,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySort {
    pub column: OverlayColumn,
    pub descending: bool,
}

impl Default for OverlaySort {
    /// The desktop meter's initial order: most damage first.
    fn default() -> Self {
        Self {
            column: OverlayColumn::TotalDamage,
            descending: true,
        }
    }
}

/// Mirrors the meter's `EncounterStatus` strings in JSON.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayStatus {
    #[default]
    Waiting,
    InProgress,
    Stopped,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlayState {
    pub status: OverlayStatus,
    /// First damage of the encounter, UTC milliseconds since the epoch.
    pub start_time: i64,
    /// Last known damage of the encounter, UTC milliseconds since the epoch.
    pub end_time: i64,
    pub total_damage: u64,
    pub dps: f64,
    pub settings: OverlaySettings,
    /// Players in party order, already filtered by streamer mode. The overlay sorts them.
    pub players: Vec<OverlayPlayer>,
    /// Translated names of the enemies hit in this encounter. HP itself is read in game.
    pub enemies: Vec<OverlayEnemy>,
    /// Attack types of `OverlayPlayer::attack_damage`, with their labels and colours.
    pub attack_types: Vec<OverlayAttackType>,
    /// Damage taken per party member, most first, already filtered by streamer mode.
    pub damage_taken: Vec<OverlayDamageTaken>,
    pub damage_taken_labels: OverlayDamageTakenLabels,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlayDamageTaken {
    /// Name exactly as the meter shows it.
    pub name: String,
    /// Bar colour as `0xRRGGBB`.
    pub color: u32,
    pub total_damage: u64,
    pub hits: u32,
    pub max_hit: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlayDamageTakenLabels {
    pub title: String,
    pub hits: String,
    pub max_hit: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverlayEnemy {
    /// Hash ID of the enemy type, as in `Actor::actor_type`.
    pub type_hash: u32,
    pub name: String,
}

/// The settings page's meter options, with what the in-game settings menu needs to edit them.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
    pub values: OverlaySettingsValues,
    /// Selectable languages, as on the settings page.
    pub languages: Vec<OverlayOption>,
    /// Every value column with its translated header and description.
    pub columns: Vec<OverlayColumnInfo>,
    /// Translated labels of the settings page.
    pub labels: OverlaySettingsLabels,
}

/// The editable values. Sent back by the overlay when they are changed in game.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettingsValues {
    pub language: String,
    /// Bar colours of party slots 1-4 as `0xRRGGBB`.
    pub party_colors: Vec<u32>,
    /// Background opacity of the meter, 0-1.
    pub transparency: f32,
    pub show_display_names: bool,
    pub streamer_mode: bool,
    /// Full numbers with separators instead of `12.3k` style units.
    pub show_full_values: bool,
    pub use_condensed_skills: bool,
    pub open_log_on_save: bool,
    /// Value columns after the name, in display order.
    pub columns: Vec<OverlayColumn>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlayOption {
    pub value: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OverlayColumnInfo {
    pub column: OverlayColumn,
    pub label: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettingsLabels {
    pub title: String,
    pub language: String,
    /// "Player N color" for party slots 1-4.
    pub party_colors: Vec<String>,
    pub transparency: String,
    pub show_display_names: String,
    pub streamer_mode: String,
    pub streamer_mode_description: String,
    pub show_full_values: String,
    pub show_full_values_description: String,
    pub use_condensed_skills: String,
    pub use_condensed_skills_description: String,
    pub open_log_on_save: String,
    pub open_log_on_save_description: String,
    pub columns: String,
}

impl OverlaySettings {
    /// Translated header of a column, falling back to its identifier.
    pub fn column_label(&self, column: OverlayColumn) -> &str {
        self.columns
            .iter()
            .find(|info| info.column == column)
            .map_or("?", |info| info.label.as_str())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlayer {
    /// Stable actor index, used to remember expanded rows.
    pub index: u32,
    /// Position in party order; sorting by name uses it, like the desktop meter.
    pub party_order: u32,
    /// Display name exactly as the meter shows it, including the party slot prefix.
    pub name: String,
    /// Bar colour as `0xRRGGBB`.
    pub color: u32,
    pub total_damage: u64,
    pub dps: f64,
    /// Share of the encounter's total damage, 0-100.
    pub percentage: f32,
    /// SBA gauge, 0-1000.
    pub sba: f32,
    pub total_stun_value: f64,
    pub stun_per_second: f64,
    /// Skill breakdown, most damage first, grouped when condensed skills are enabled.
    pub skills: Vec<OverlaySkill>,
    /// Damage per attack type, in the order of `OverlayState::attack_types`.
    pub attack_damage: Vec<u64>,
}

/// An attack type of the damage composition (normal attacks, skills, SBA, ...).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlayAttackType {
    pub label: String,
    /// Segment colour as `0xRRGGBB`.
    pub color: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySkill {
    pub name: String,
    pub hits: u32,
    pub total_damage: u64,
    pub min_damage: Option<u64>,
    pub max_damage: Option<u64>,
    /// Share of the player's damage, 0-100.
    pub percentage: f32,
    /// Skills inside a condensed group; empty for plain skills.
    pub children: Vec<OverlaySkill>,
}

impl OverlayPlayer {
    /// Numeric value of a column, for sorting.
    pub fn sort_value(&self, column: OverlayColumn) -> f64 {
        match column {
            OverlayColumn::Name => self.party_order as f64,
            OverlayColumn::TotalDamage => self.total_damage as f64,
            OverlayColumn::Dps => self.dps,
            OverlayColumn::DamagePercentage => self.percentage as f64,
            OverlayColumn::Sba => self.sba as f64,
            OverlayColumn::TotalStunValue => self.total_stun_value,
            OverlayColumn::StunPerSecond => self.stun_per_second,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_bincode() {
        let state = OverlayState {
            status: OverlayStatus::InProgress,
            start_time: 1_700_000_000_000,
            end_time: 1_700_000_030_000,
            total_damage: 1_234_567,
            dps: 41_152.2,
            settings: OverlaySettings {
                values: OverlaySettingsValues {
                    language: "zh-CN".into(),
                    transparency: 0.2,
                    columns: vec![OverlayColumn::Dps],
                    ..Default::default()
                },
                columns: vec![OverlayColumnInfo {
                    column: OverlayColumn::Dps,
                    label: "DPS".into(),
                    description: "Damage per second".into(),
                }],
                ..Default::default()
            },
            players: vec![OverlayPlayer {
                name: "[1] 古兰 (Gran)".into(),
                color: 0xFF5630,
                total_damage: 1_234_567,
                skills: vec![OverlaySkill {
                    name: "Group".into(),
                    min_damage: None,
                    children: vec![OverlaySkill::default()],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            damage_taken: vec![OverlayDamageTaken {
                name: "[1] Gran".into(),
                hits: 3,
                ..Default::default()
            }],
            damage_taken_labels: OverlayDamageTakenLabels::default(),
            attack_types: vec![OverlayAttackType {
                label: "Skills".into(),
                color: 0x4C9AFF,
            }],
            enemies: vec![OverlayEnemy {
                type_hash: 0x1234_5678,
                name: "Enemy".into(),
            }],
        };

        let bytes = bincode::serialize(&OverlayMessage::State(state.clone())).unwrap();
        let OverlayMessage::State(decoded) = bincode::deserialize(&bytes).unwrap() else {
            panic!("decoded a different message");
        };

        assert_eq!(decoded.status, state.status);
        assert_eq!(decoded.settings.values, state.settings.values);
        assert_eq!(decoded.settings.column_label(OverlayColumn::Dps), "DPS");
        assert_eq!(decoded.players[0].name, state.players[0].name);
        assert_eq!(decoded.players[0].skills[0].children.len(), 1);
        assert_eq!(decoded.enemies, state.enemies);
        assert_eq!(decoded.damage_taken, state.damage_taken);
    }
}
