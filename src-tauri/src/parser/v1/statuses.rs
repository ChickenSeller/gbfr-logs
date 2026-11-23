//! Status effects on party members: how often each one was applied and for how long.
//!
//! Built from `Message::StatusApplied` / `StatusRemoved` in the event log. A status still active
//! when the encounter ends counts until its last event.

use std::collections::HashMap;

use protocol::StatusEvent;
use serde::{Deserialize, Serialize, Serializer};

/// Statistics of one status class on one party member.
#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusStats {
    pub class_name: String,
    pub type_id: u32,
    /// Times it was applied.
    pub count: u32,
    /// Total time it was active, in milliseconds.
    pub total_ms: i64,
    /// Longest single application, in milliseconds.
    pub max_ms: i64,
    /// Applications per source type hash (enemy or character).
    pub sources: HashMap<u32, u32>,
    /// Damage enemies dealt to the member while this status was active. A hit counts for every
    /// status active at the time.
    pub damage_while_active: u64,
}

/// Status statistics per party member (actor index) and class name, plus what is still active.
/// Serializes as `{ actorIndex: { className: StatusStats } }` with active statuses counted until
/// the latest time it was told about.
#[derive(Debug, Default, Clone)]
pub struct StatusTracker {
    stats: HashMap<u32, HashMap<String, StatusStats>>,
    /// Active statuses by (actor index, status id): class name and start time.
    active: HashMap<(u32, u64), (String, i64)>,
    now: i64,
}

impl Serialize for StatusTracker {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.snapshot(self.now).serialize(serializer)
    }
}

impl StatusTracker {
    /// Moves the clock used for statuses that are still active.
    pub fn touch(&mut self, now: i64) {
        self.now = self.now.max(now);
    }

    pub fn applied(&mut self, timestamp: i64, event: &StatusEvent) {
        self.touch(timestamp);
        let key = (event.actor_index, event.status_id);
        if self.active.contains_key(&key) {
            return;
        }
        self.active
            .insert(key, (event.class_name.clone(), timestamp));
        let stats = self
            .stats
            .entry(event.actor_index)
            .or_default()
            .entry(event.class_name.clone())
            .or_insert_with(|| StatusStats {
                class_name: event.class_name.clone(),
                type_id: event.type_id,
                ..Default::default()
            });
        stats.count += 1;
        if let Some(source) = event.source_type {
            *stats.sources.entry(source).or_default() += 1;
        }
    }

    pub fn removed(&mut self, timestamp: i64, event: &StatusEvent) {
        self.touch(timestamp);
        if let Some((class_name, start)) = self.active.remove(&(event.actor_index, event.status_id))
        {
            self.add_duration(event.actor_index, &class_name, timestamp - start);
        }
    }

    fn add_duration(&mut self, actor_index: u32, class_name: &str, duration: i64) {
        if let Some(stats) = self
            .stats
            .get_mut(&actor_index)
            .and_then(|by_class| by_class.get_mut(class_name))
        {
            let duration = duration.max(0);
            stats.total_ms += duration;
            stats.max_ms = stats.max_ms.max(duration);
        }
    }

    /// The statistics with statuses still active counted until `now`.
    pub fn snapshot(&self, now: i64) -> HashMap<u32, HashMap<String, StatusStats>> {
        let mut tracker = self.clone();
        for ((actor_index, _), (class_name, start)) in &self.active {
            tracker.add_duration(*actor_index, class_name, now - start);
        }
        tracker.stats
    }

    /// Adds a hit an actor took to every status class active on them.
    pub fn damage_taken(&mut self, actor_index: u32, damage: u64) {
        for class_name in self.active_classes(actor_index) {
            if let Some(stats) = self
                .stats
                .get_mut(&actor_index)
                .and_then(|by_class| by_class.get_mut(&class_name))
            {
                stats.damage_while_active += damage;
            }
        }
    }

    /// Whether an actor currently has a status of this class.
    pub fn is_active(&self, actor_index: u32, class_name: &str) -> bool {
        self.active
            .iter()
            .any(|((actor, _), (class, _))| *actor == actor_index && class == class_name)
    }

    /// Classes currently active on an actor.
    pub fn active_classes(&self, actor_index: u32) -> Vec<String> {
        let mut classes: Vec<String> = self
            .active
            .iter()
            .filter(|((actor, _), _)| *actor == actor_index)
            .map(|(_, (class, _))| class.clone())
            .collect();
        classes.sort();
        classes.dedup();
        classes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(status_id: u64, class_name: &str) -> StatusEvent {
        StatusEvent {
            actor_index: 1,
            actor_type: 0x26A4848A,
            status_id,
            class_name: class_name.into(),
            type_id: 7,
            source_type: Some(0x1234),
        }
    }

    #[test]
    fn counts_applications_and_durations() {
        let mut tracker = StatusTracker::default();
        tracker.applied(1_000, &event(1, "StatusAilmentPoison"));
        tracker.applied(1_100, &event(1, "StatusAilmentPoison")); // repeated report, ignored
        tracker.removed(4_000, &event(1, "StatusAilmentPoison"));
        tracker.applied(5_000, &event(2, "StatusAilmentPoison"));
        assert!(tracker.is_active(1, "StatusAilmentPoison"));

        let stats = &tracker.snapshot(6_000)[&1]["StatusAilmentPoison"];
        assert_eq!(stats.count, 2);
        assert_eq!(stats.total_ms, 3_000 + 1_000);
        assert_eq!(stats.max_ms, 3_000);
        assert_eq!(stats.sources[&0x1234], 2);
        assert_eq!(stats.damage_while_active, 0);
        // The snapshot does not end the active one.
        assert!(tracker.is_active(1, "StatusAilmentPoison"));
    }

    #[test]
    fn damage_counts_for_every_active_status_once() {
        let mut tracker = StatusTracker::default();
        tracker.applied(0, &event(1, "StatusAilmentPoison"));
        tracker.applied(0, &event(2, "StatusDeffenceDebuff"));
        tracker.applied(0, &event(3, "StatusDeffenceDebuff")); // same class twice
        tracker.damage_taken(1, 500);
        tracker.removed(10, &event(1, "StatusAilmentPoison"));
        tracker.damage_taken(1, 100);
        tracker.damage_taken(2, 999); // another actor

        let stats = tracker.snapshot(20);
        assert_eq!(stats[&1]["StatusAilmentPoison"].damage_while_active, 500);
        assert_eq!(stats[&1]["StatusDeffenceDebuff"].damage_while_active, 600);
    }

    #[test]
    fn removal_without_application_is_ignored() {
        let mut tracker = StatusTracker::default();
        tracker.removed(4_000, &event(9, "StatusAttackDebuff"));
        assert!(tracker.snapshot(5_000).is_empty());
    }
}
