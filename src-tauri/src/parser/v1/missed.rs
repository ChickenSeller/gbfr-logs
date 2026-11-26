//! Link attacks and SBA chains a party member could have joined but did not, times downed
//! (real downs apart from auto-revives) and time spent downed.
//!
//! - Link: when a member enters a link attack or deals link attack damage, a window of
//!   `LINK_WINDOW` opens. It is a link only if link attack damage follows in it (an AI entering
//!   the action alone once counted as a link nobody saw); then every party member who neither
//!   entered a link attack nor dealt link damage in it missed that link. Damage counts as
//!   joining because an online player's action may not read right. (In game 2.0.2 logs the
//!   members who joined entered within about 1.2 s of the first, their damage 0.6-0.8 s after.)
//! - SBA chain: when a member starts an SBA, every other member whose gauge is full can chain
//!   it; one who does not start their own within `CHAIN_WINDOW` missed that chain. Each SBA
//!   started in the window opens it again for the members still waiting.
//!
//! Party members are those with a known SBA gauge (the hook reports it for every member, AI
//! supporters included) or a link attack. A window still open counts nothing until it closes.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Serialize, Serializer};

pub const LINK_WINDOW_MS: i64 = 3_000;
pub const CHAIN_WINDOW_MS: i64 = 5_000;
/// The status a member gets when an auto-revive saves them, and how close to the down it must
/// appear to count.
const AUTO_REVIVE_BUFF: &str = "StatusAutoReviveBuff";
const AUTO_REVIVE_PAIRING_MS: i64 = 1_500;
/// The SBA gauge's maximum, as the hook reports it.
const FULL_GAUGE: f32 = 999.5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissStats {
    /// Link attacks joined.
    pub links: u32,
    pub missed_links: u32,
    /// SBAs started.
    pub sbas: u32,
    /// SBA chains that could have been joined (gauge full) but were not.
    pub missed_chains: u32,
    /// Times really downed (`ExPlayerDie`): auto-revives not included.
    pub deaths: u32,
    /// Time spent really downed, the current spell included.
    pub downed_ms: u64,
    /// Downs an auto-revive got them out of (the auto-revive buff appeared with the down).
    pub auto_revives: u32,
}

#[derive(Debug, Clone)]
struct LinkWindow {
    start: i64,
    joined: HashSet<u32>,
    confirmed: bool,
}

#[derive(Debug, Default, Clone)]
pub struct MissTracker {
    stats: HashMap<u32, MissStats>,
    members: BTreeSet<u32>,
    gauge: HashMap<u32, f32>,
    /// The open link window: when it opened, who joined, and whether link damage confirmed it.
    link: Option<LinkWindow>,
    /// Members who can chain the current SBA, until when.
    chain: HashMap<u32, i64>,
    /// Members whose SBA started (attempt seen) and has not landed yet.
    casting: HashSet<u32>,
    /// Members downed right now, since when.
    downed: HashMap<u32, i64>,
    /// Each member's last down (when, and whether it was counted as an auto-revive) and last
    /// auto-revive buff, to pair them.
    last_down: HashMap<u32, (i64, bool)>,
    last_auto_revive_buff: HashMap<u32, i64>,
    now: i64,
}

impl MissTracker {
    /// Moves the clock, closing the windows that ended by `now`.
    pub fn touch(&mut self, now: i64) {
        self.now = self.now.max(now);
        if let Some(window) = &self.link {
            if self.now > window.start + LINK_WINDOW_MS {
                if window.confirmed {
                    for member in &self.members {
                        let stats = self.stats.entry(*member).or_default();
                        if window.joined.contains(member) {
                            stats.links += 1;
                        } else {
                            stats.missed_links += 1;
                        }
                    }
                }
                self.link = None;
            }
        }
        let now = self.now;
        let stats = &mut self.stats;
        self.chain.retain(|member, until| {
            let open = now <= *until;
            if !open {
                stats.entry(*member).or_default().missed_chains += 1;
            }
            open
        });
    }

    pub fn gauge(&mut self, timestamp: i64, actor: u32, value: f32) {
        self.touch(timestamp);
        self.members.insert(actor);
        self.gauge.insert(actor, value);
    }

    /// The SBA was started (the gauge dropped from full).
    pub fn sba_attempt(&mut self, timestamp: i64, actor: u32) {
        self.touch(timestamp);
        self.casting.insert(actor);
        self.sba_started(actor);
    }

    /// The SBA landed (the gauge reached 0); a start too if no attempt was seen before it.
    pub fn sba_perform(&mut self, timestamp: i64, actor: u32) {
        self.touch(timestamp);
        if !self.casting.remove(&actor) {
            self.sba_started(actor);
        }
        self.gauge.insert(actor, 0.0);
    }

    fn sba_started(&mut self, actor: u32) {
        self.members.insert(actor);
        self.stats.entry(actor).or_default().sbas += 1;
        self.chain.remove(&actor);
        self.gauge.insert(actor, 0.0);
        let until = self.now + CHAIN_WINDOW_MS;
        for (member, value) in &self.gauge {
            if *member != actor && *value >= FULL_GAUGE {
                let entry = self.chain.entry(*member).or_insert(until);
                *entry = (*entry).max(until);
            }
        }
    }

    pub fn death(&mut self, timestamp: i64, actor: u32) {
        self.touch(timestamp);
        self.members.insert(actor);
        self.downed.entry(actor).or_insert(timestamp);
        let auto = self
            .last_auto_revive_buff
            .get(&actor)
            .is_some_and(|buff| (timestamp - buff).abs() <= AUTO_REVIVE_PAIRING_MS);
        let stats = self.stats.entry(actor).or_default();
        if auto {
            stats.auto_revives += 1;
        } else {
            stats.deaths += 1;
        }
        self.last_down.insert(actor, (timestamp, auto));
    }

    /// A status appeared on a member: the auto-revive buff appearing with a down (in game 2.0.2
    /// timelines, at the very moment of the down) means the member got up by themselves.
    pub fn status_applied(&mut self, timestamp: i64, actor: u32, class_name: &str) {
        if class_name != AUTO_REVIVE_BUFF {
            return;
        }
        self.touch(timestamp);
        self.last_auto_revive_buff.insert(actor, timestamp);
        if let Some((down, counted)) = self.last_down.get_mut(&actor) {
            let during = self.downed.contains_key(&actor);
            if !*counted && (during || (timestamp - *down).abs() <= AUTO_REVIVE_PAIRING_MS) {
                // Counted as a real down a moment ago: it was an auto-revive after all.
                *counted = true;
                let stats = self.stats.entry(actor).or_default();
                stats.deaths = stats.deaths.saturating_sub(1);
                stats.auto_revives += 1;
            }
        }
    }

    pub fn revive(&mut self, timestamp: i64, actor: u32) {
        self.touch(timestamp);
        if let Some(since) = self.downed.remove(&actor) {
            if !self.auto_revived(actor) {
                self.stats.entry(actor).or_default().downed_ms += (timestamp - since).max(0) as u64;
            }
        }
    }

    /// Whether the member's last down was an auto-revive.
    fn auto_revived(&self, actor: u32) -> bool {
        self.last_down.get(&actor).is_some_and(|(_, auto)| *auto)
    }

    /// A member entered a link attack.
    pub fn link_attack(&mut self, timestamp: i64, actor: u32) {
        self.join_link(timestamp, actor, false);
    }

    /// A member dealt link attack damage: confirms the window as a real link.
    pub fn link_damage(&mut self, timestamp: i64, actor: u32) {
        self.join_link(timestamp, actor, true);
    }

    fn join_link(&mut self, timestamp: i64, actor: u32, damage: bool) {
        self.touch(timestamp);
        self.members.insert(actor);
        let window = self.link.get_or_insert_with(|| LinkWindow {
            start: timestamp,
            joined: HashSet::new(),
            confirmed: false,
        });
        window.joined.insert(actor);
        window.confirmed |= damage;
    }

    /// Per actor index, every member included.
    pub fn stats(&self) -> HashMap<u32, MissStats> {
        let mut stats = self.stats.clone();
        for member in &self.members {
            stats.entry(*member).or_default();
        }
        for (member, since) in &self.downed {
            if !self.auto_revived(*member) {
                stats.entry(*member).or_default().downed_ms += (self.now - since).max(0) as u64;
            }
        }
        stats
    }
}

impl Serialize for MissTracker {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.stats().serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downed_time_adds_up_and_includes_the_current_spell() {
        let mut tracker = MissTracker::default();
        tracker.death(1_000, 2);
        tracker.revive(4_500, 2);
        tracker.death(10_000, 2);
        tracker.touch(12_000);
        let stats = tracker.stats()[&2];
        assert_eq!(stats.deaths, 2);
        assert_eq!(stats.downed_ms, 5_500);
    }

    #[test]
    fn an_auto_revive_buff_with_a_down_counts_once() {
        let mut tracker = MissTracker::default();
        // The buff on its own (a skill granting it) is no auto-revive.
        tracker.status_applied(0, 1, AUTO_REVIVE_BUFF);
        tracker.death(5_000, 1);
        tracker.status_applied(5_000, 1, AUTO_REVIVE_BUFF);
        tracker.status_applied(5_100, 1, AUTO_REVIVE_BUFF);
        tracker.revive(9_000, 1);
        // Revived by a teammate: no buff.
        tracker.death(20_000, 1);
        tracker.revive(24_000, 1);
        let stats = tracker.stats()[&1];
        // Outside the brackets real downs only; downed time only theirs.
        assert_eq!(stats.deaths, 1);
        assert_eq!(stats.auto_revives, 1);
        assert_eq!(stats.downed_ms, 4_000);
    }

    #[test]
    fn members_outside_a_link_window_missed_it() {
        let mut tracker = MissTracker::default();
        for actor in [0, 1, 2, 3] {
            tracker.gauge(0, actor, 100.0);
        }
        tracker.link_attack(1_000, 0);
        tracker.link_attack(1_100, 0);
        tracker.link_attack(1_200, 1);
        tracker.link_attack(2_100, 2);
        tracker.link_damage(1_800, 0);
        tracker.touch(1_000 + LINK_WINDOW_MS + 1);
        let stats = tracker.stats();
        // Entering twice in one window is one link.
        assert_eq!(stats[&0].links, 1);
        assert_eq!(stats[&2].missed_links, 0);
        assert_eq!(stats[&3].missed_links, 1);
    }

    #[test]
    fn a_link_action_without_link_damage_is_no_link() {
        let mut tracker = MissTracker::default();
        for actor in [0, 1] {
            tracker.gauge(0, actor, 100.0);
        }
        tracker.link_attack(1_000, 1);
        tracker.touch(1_000 + LINK_WINDOW_MS + 1);
        assert_eq!(tracker.stats()[&0].missed_links, 0);
    }

    #[test]
    fn link_damage_alone_counts_as_joining() {
        let mut tracker = MissTracker::default();
        for actor in [0, 1, 2] {
            tracker.gauge(0, actor, 100.0);
        }
        // 1 entered the action; 2 (online, action unreadable) only dealt link damage.
        tracker.link_attack(1_000, 1);
        tracker.link_damage(1_700, 1);
        tracker.link_damage(1_800, 2);
        tracker.touch(1_000 + LINK_WINDOW_MS + 1);
        let stats = tracker.stats();
        assert_eq!(stats[&2].missed_links, 0);
        assert_eq!(stats[&0].missed_links, 1);
    }

    #[test]
    fn a_full_gauge_that_does_not_follow_an_sba_missed_the_chain() {
        let mut tracker = MissTracker::default();
        tracker.gauge(0, 0, 1000.0);
        tracker.gauge(0, 1, 1000.0);
        tracker.gauge(0, 2, 1000.0);
        tracker.gauge(0, 3, 400.0);
        tracker.sba_attempt(1_000, 0);
        // 1 chains in time; 2 waits, and the window opened again by 1 runs out.
        tracker.sba_attempt(3_000, 1);
        tracker.sba_perform(3_500, 0);
        tracker.touch(3_000 + CHAIN_WINDOW_MS - 1);
        assert_eq!(tracker.stats()[&2].missed_chains, 0);
        tracker.touch(3_000 + CHAIN_WINDOW_MS + 1);
        let stats = tracker.stats();
        assert_eq!(stats[&0].sbas, 1);
        assert_eq!(stats[&1].missed_chains, 0);
        assert_eq!(stats[&2].missed_chains, 1);
        // Not full: could not chain.
        assert_eq!(stats[&3].missed_chains, 0);
    }

    #[test]
    fn an_sba_landing_without_an_attempt_still_counts_as_started() {
        let mut tracker = MissTracker::default();
        tracker.gauge(0, 0, 1000.0);
        tracker.gauge(0, 1, 1000.0);
        tracker.sba_perform(1_000, 0);
        tracker.touch(1_000 + CHAIN_WINDOW_MS + 1);
        assert_eq!(tracker.stats()[&0].sbas, 1);
        assert_eq!(tracker.stats()[&1].missed_chains, 1);
    }
}
