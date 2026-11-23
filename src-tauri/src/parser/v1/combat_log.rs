//! Combat log: the encounter's events as a list of structured entries.
//!
//! Entries carry ids and numbers only; the front-end turns them into text in the selected
//! language. They are sent live as events happen and rebuilt from the event log for saved logs.

use protocol::{ActionType, DamageEvent, Message};
use serde::Serialize;

use crate::parser::constants::CharacterType;

/// Name of the event that carries live entries to the logs window.
pub const LIVE_EVENT: &str = "combat-log-entry";

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CombatLogEntry {
    /// A party member (or one of their summons) hit an enemy.
    #[serde(rename_all = "camelCase")]
    Damage {
        /// Milliseconds since the encounter started.
        time: i64,
        /// Actor index of the party member, as in the meter.
        source_index: u32,
        source_type: CharacterType,
        /// The actor that actually hit (a pet or form), same as `source_type` if none.
        child_type: CharacterType,
        /// Enemy type hash.
        target_type: u32,
        action: ActionType,
        damage: i32,
    },
    /// An enemy hit a party member.
    #[serde(rename_all = "camelCase")]
    DamageTaken {
        time: i64,
        /// Enemy type hash.
        source_type: u32,
        target_index: u32,
        target_type: CharacterType,
        damage: i32,
    },
    /// A party member's Skybound Art gauge is full and they are ready to start a chain.
    #[serde(rename_all = "camelCase")]
    SbaReady { time: i64, actor_index: u32 },
    #[serde(rename_all = "camelCase")]
    SbaPerform { time: i64, actor_index: u32 },
    #[serde(rename_all = "camelCase")]
    SbaChain { time: i64, actor_index: u32 },
    #[serde(rename_all = "camelCase")]
    Death { time: i64, actor_index: u32 },
    /// A status effect appeared on a party member.
    #[serde(rename_all = "camelCase")]
    StatusApplied {
        time: i64,
        actor_index: u32,
        class_name: String,
        /// Enemy or character hash of whoever applied it, if known.
        source_type: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    StatusRemoved {
        time: i64,
        actor_index: u32,
        class_name: String,
    },
}

fn damage_entry(time: i64, event: &DamageEvent) -> CombatLogEntry {
    CombatLogEntry::Damage {
        time,
        source_index: event.source.parent_index,
        source_type: CharacterType::from_hash(event.source.parent_actor_type),
        child_type: CharacterType::from_hash(event.source.actor_type),
        target_type: event.target.parent_actor_type,
        action: event.action_id,
        damage: event.damage,
    }
}

/// The entry for a logged message, if it belongs in the combat log. `time` is relative to the
/// start of the encounter.
pub fn entry(time: i64, message: &Message) -> Option<CombatLogEntry> {
    Some(match message {
        Message::DamageEvent(event) => damage_entry(time, event),
        Message::DamageTaken(event) => CombatLogEntry::DamageTaken {
            time,
            source_type: event.source.parent_actor_type,
            target_index: event.target.parent_index,
            target_type: CharacterType::from_hash(event.target.parent_actor_type),
            damage: event.damage,
        },
        Message::OnAttemptSBA(event) => CombatLogEntry::SbaReady {
            time,
            actor_index: event.actor_index,
        },
        Message::OnPerformSBA(event) => CombatLogEntry::SbaPerform {
            time,
            actor_index: event.actor_index,
        },
        Message::OnContinueSBAChain(event) => CombatLogEntry::SbaChain {
            time,
            actor_index: event.actor_index,
        },
        Message::OnDeathEvent(event) => CombatLogEntry::Death {
            time,
            actor_index: event.actor_index,
        },
        Message::StatusApplied(event) => CombatLogEntry::StatusApplied {
            time,
            actor_index: event.actor_index,
            class_name: event.class_name.clone(),
            source_type: event.source_type,
        },
        Message::StatusRemoved(event) => CombatLogEntry::StatusRemoved {
            time,
            actor_index: event.actor_index,
            class_name: event.class_name.clone(),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use protocol::{Actor, OnDeathEvent};

    use super::*;

    #[test]
    fn serializes_with_a_kind_tag() {
        let entry = entry(
            1_500,
            &Message::OnDeathEvent(OnDeathEvent {
                actor_index: 3,
                death_counter: 1,
            }),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(entry).unwrap(),
            serde_json::json!({ "kind": "death", "time": 1500, "actorIndex": 3 })
        );
    }

    #[test]
    fn damage_keeps_the_acting_child() {
        let event = DamageEvent {
            source: Actor {
                index: 7,
                actor_type: 0x2AF678E8,
                parent_index: 1,
                parent_actor_type: 0xFBA6615D,
            },
            target: Actor {
                index: 9,
                actor_type: 0x1234,
                parent_index: 9,
                parent_actor_type: 0x1234,
            },
            damage: 42,
            flags: 0,
            action_id: ActionType::Normal(65),
            attack_rate: None,
            stun_value: None,
            damage_cap: None,
            details: None,
        };
        let json = serde_json::to_value(entry(10, &Message::DamageEvent(event)).unwrap()).unwrap();
        assert_eq!(json["kind"], "damage");
        assert_eq!(json["sourceIndex"], 1);
        assert_eq!(json["sourceType"], "Pl0700");
        assert_eq!(json["childType"], "Pl0700Ghost");
        assert_eq!(json["action"], serde_json::json!({ "Normal": 65 }));
    }
}
