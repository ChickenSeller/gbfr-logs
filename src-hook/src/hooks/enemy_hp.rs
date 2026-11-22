//! Enemy HP, read straight from the game for the in-game HP bars.
//!
//! Every damaged enemy is remembered by its instance pointer. Its HP is then read again each
//! frame, so the bars follow the game without waiting for the app. Found by diffing memory
//! around damage events (game 2.0.2): current HP is at `instance + 0x160` and max HP right after
//! it, both 64-bit.
//!
//! Instances are freed when enemies despawn, so every read is checked: the object must keep its
//! vtable and max HP, and HP must stay in range. Anything else drops the entry.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::read_process_value;

const HP_OFFSET: usize = 0x160;
const MAX_HP_OFFSET: usize = 0x168;
/// Sanity bound; real values are far below it.
const MAX_PLAUSIBLE_HP: i64 = 100_000_000_000;

/// How long a defeated enemy stays listed.
const DEFEATED_LINGER: Duration = Duration::from_secs(5);
/// Enemies nobody has hit for this long are dropped, e.g. ones left behind.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TRACKED: usize = 32;

/// Player character hashes (see `CharacterType::from_hash` in the app). Players have the same
/// HP layout but are not shown as enemies.
const PLAYER_TYPE_HASHES: [u32; 32] = [
    0x26A4848A, 0x9498420D, 0x34D4FD8F, 0xF8D73D33, 0x7B5934AD, 0x443D46BB, 0xA9D6569E, 0xFBA6615D,
    0x63A7C3F0, 0xF96A90C2, 0x28AC1108, 0x94E2514E, 0x2B4AA114, 0xC97F3365, 0x601AA977, 0xBCC238DE,
    0xC3155079, 0xD16CFBDE, 0x6FDD6932, 0x8056ABCD, 0xF5755C0E, 0x9C89A455, 0x59DB0CD9, 0xDA5A8E25,
    0x4C714F77, 0xE330418F, 0xE3D1BE26, 0x91418145, 0x48ADDA36, 0x0A58FB4D, 0x2AF678E8, 0x8364C8BC,
];

#[derive(Clone, Debug, PartialEq)]
pub struct EnemyHp {
    /// Stable while the enemy lives; tells enemies of the same type apart.
    pub id: usize,
    pub type_hash: u32,
    pub hp: i64,
    pub max_hp: i64,
}

struct Tracked {
    instance: usize,
    vtable: usize,
    type_hash: u32,
    hp: i64,
    max_hp: i64,
    last_hit: Instant,
    defeated_at: Option<Instant>,
}

static TRACKED: Mutex<Vec<Tracked>> = Mutex::new(Vec::new());

fn lock() -> std::sync::MutexGuard<'static, Vec<Tracked>> {
    TRACKED.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_hp(instance: usize) -> Option<(i64, i64)> {
    let hp = read_process_value::<i64>((instance + HP_OFFSET) as *const i64)?;
    let max_hp = read_process_value::<i64>((instance + MAX_HP_OFFSET) as *const i64)?;
    plausible(hp, max_hp).then_some((hp, max_hp))
}

fn plausible(hp: i64, max_hp: i64) -> bool {
    max_hp > 0 && max_hp <= MAX_PLAUSIBLE_HP && (0..=max_hp).contains(&hp)
}

fn is_player(type_hash: u32) -> bool {
    PLAYER_TYPE_HASHES.contains(&type_hash)
}

/// Records a damaged target; called after the game applied the damage.
pub fn observe(instance: usize, type_hash: u32) {
    if instance == 0 || type_hash == 0 || is_player(type_hash) {
        return;
    }
    let Some((hp, max_hp)) = read_hp(instance) else {
        return;
    };
    let Some(vtable) = read_process_value::<usize>(instance as *const usize) else {
        return;
    };
    let now = Instant::now();
    let mut tracked = lock();
    if let Some(entry) = tracked.iter_mut().find(|t| t.instance == instance) {
        // The same address may now hold a different enemy.
        if entry.vtable != vtable || entry.max_hp != max_hp || entry.type_hash != type_hash {
            entry.vtable = vtable;
            entry.type_hash = type_hash;
            entry.max_hp = max_hp;
            entry.defeated_at = None;
        }
        entry.hp = hp;
        entry.last_hit = now;
        return;
    }
    if tracked.len() >= MAX_TRACKED {
        if let Some(oldest) = (0..tracked.len()).min_by_key(|&i| tracked[i].last_hit) {
            tracked.swap_remove(oldest);
        }
    }
    tracked.push(Tracked {
        instance,
        vtable,
        type_hash,
        hp,
        max_hp,
        last_hit: now,
        defeated_at: None,
    });
}

/// Re-reads every tracked enemy and returns the ones still worth showing, biggest max HP
/// first (bosses before adds), then by most recent hit.
pub fn snapshot() -> Vec<EnemyHp> {
    let now = Instant::now();
    let mut tracked = lock();
    tracked.retain_mut(|entry| {
        let still_same =
            read_process_value::<usize>(entry.instance as *const usize) == Some(entry.vtable);
        let Some((hp, max_hp)) = still_same.then(|| read_hp(entry.instance)).flatten() else {
            return false;
        };
        if max_hp != entry.max_hp {
            return false;
        }
        entry.hp = hp;
        if hp == 0 {
            let defeated_at = *entry.defeated_at.get_or_insert(now);
            return now.duration_since(defeated_at) < DEFEATED_LINGER;
        }
        entry.defeated_at = None;
        now.duration_since(entry.last_hit) < IDLE_TIMEOUT
    });

    let mut order: Vec<&Tracked> = tracked.iter().collect();
    order.sort_by(|a, b| {
        b.max_hp
            .cmp(&a.max_hp)
            .then_with(|| b.last_hit.cmp(&a.last_hit))
    });
    order
        .into_iter()
        .map(|entry| EnemyHp {
            id: entry.instance,
            type_hash: entry.type_hash,
            hp: entry.hp,
            max_hp: entry.max_hp,
        })
        .collect()
}

/// Forgets every enemy, e.g. at the result screen.
pub fn reset() {
    lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plausible_hp_is_within_max() {
        assert!(plausible(112_092_781, 113_810_780));
        assert!(plausible(0, 1));
        assert!(!plausible(5, 4));
        assert!(!plausible(-1, 10));
        assert!(!plausible(0, 0));
        assert!(!plausible(1, MAX_PLAUSIBLE_HP + 1));
    }

    #[test]
    fn players_are_not_enemies() {
        assert!(is_player(0x26A4848A));
        assert!(is_player(0x8364C8BC));
        assert!(!is_player(0x12345678));
    }
}
