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

use log::info;

use super::read_process_value;

const HP_OFFSET: usize = 0x160;
const MAX_HP_OFFSET: usize = 0x168;

/// Breakable part records (game 2.0.2): a part id whose low 16 bits repeat the enemy's class
/// number (`Em1806` -> 0x21806) and, 8 bytes before it, the part's remaining break value as a
/// float, drained by damage to that part (most while the enemy is stunned and its stun damage is
/// zero, so not by stun). Found on Ewiyar at +0x27AC / +0x27A4: 1500 down to 0 as the part
/// broke, checked against a recorded fight. Other
/// enemies keep them elsewhere, so each enemy's first `PART_SCAN_SIZE` bytes are searched once.
const PART_SCAN_SIZE: usize = 0x10000;
const PART_DURABILITY_BEFORE_ID: usize = 8;
/// High half of a part id: small numbers only (Ewiyar's is 2).
const PART_ID_HIGH: std::ops::RangeInclusive<u32> = 1..=0x40;
const MAX_PARTS: usize = 16;
/// A fresh part has a real break value (Ewiyar's head: 1500); near-zero floats are other data.
const MIN_PART_DURABILITY: f32 = 1.0;
const MAX_PLAUSIBLE_DURABILITY: f32 = 10_000_000.0;
/// Sanity bound; real values are far below it.
const MAX_PLAUSIBLE_HP: i64 = 100_000_000_000;

/// How long a defeated enemy stays listed.
const DEFEATED_LINGER: Duration = Duration::from_secs(5);
/// Enemies neither hit nor listed as damageable for this long are dropped, e.g. ones left behind.
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
    /// Breakable parts, in memory order.
    pub parts: Vec<PartState>,
    /// The game's class name (`Em8000`, `We7300`), for enemies the game has no name for.
    pub class: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PartState {
    pub id: u32,
    /// The game's internal name for the part, where it has one (`ExEmBreakParts` definitions);
    /// there are no official part names in the game's text.
    pub name: Option<String>,
    pub durability: f32,
    /// Highest durability seen; the game keeps the starting value elsewhere.
    pub max_durability: f32,
}

impl PartState {
    pub fn broken(&self) -> bool {
        self.durability <= 0.0
    }
}

/// A part that broke on the last hit: (enemy type hash, part id).
pub type PartBreak = (u32, u32);

/// The enemy's class number from its RTTI name, e.g. `Em1806` -> 0x1806.
fn class_number(instance: usize) -> Option<u32> {
    let vtable = read_process_value::<usize>(instance as *const usize)?;
    let name = super::statuses::class_name(vtable)?;
    let digits = name.strip_prefix("Em")?.get(..4)?;
    u32::from_str_radix(digits, 16).ok()
}

fn plausible_durability(value: f32) -> bool {
    value.is_finite() && (-1.0..=MAX_PLAUSIBLE_DURABILITY).contains(&value)
}

/// Offsets of the part ids in an enemy instance, found by their shape: low half equal to the
/// class number, a small high half, and a plausible break value 8 bytes before.
fn find_parts(instance: usize, class: u32) -> Vec<usize> {
    let mut offsets = Vec::new();
    for block in (0..PART_SCAN_SIZE).step_by(0x1000) {
        let Some(words) =
            read_process_value::<[u32; 0x400]>((instance + block) as *const [u32; 0x400])
        else {
            continue;
        };
        for (index, word) in words.iter().enumerate() {
            let offset = block + index * 4;
            if word & 0xFFFF != class
                || !PART_ID_HIGH.contains(&(word >> 16))
                || offset < PART_DURABILITY_BEFORE_ID
            {
                continue;
            }
            let durability = read_process_value::<f32>(
                (instance + offset - PART_DURABILITY_BEFORE_ID) as *const f32,
            );
            if durability
                .is_some_and(|value| plausible_durability(value) && value >= MIN_PART_DURABILITY)
            {
                offsets.push(offset);
                if offsets.len() >= MAX_PARTS {
                    return offsets;
                }
            }
        }
    }
    offsets
}

/// The game's list of an enemy's broken parts: a vector of u32 part ids in its
/// `ExEmBreakPartsInfo`, which gains one entry as each part breaks (seen on Ewiyar and Proto
/// Bahamut, game 2.0.2). Works for every enemy, also those whose break values are not found.
const BROKEN_LIST_BEGIN: usize = 0x10;
const BROKEN_LIST_END: usize = 0x18;

fn broken_part_ids(instance: usize) -> Vec<u32> {
    let Some(base) = super::rtti::base_offset(instance, "ExEmBreakPartsInfo") else {
        return Vec::new();
    };
    let at = instance + base;
    let (Some(begin), Some(end)) = (
        read_process_value::<usize>((at + BROKEN_LIST_BEGIN) as *const usize),
        read_process_value::<usize>((at + BROKEN_LIST_END) as *const usize),
    ) else {
        return Vec::new();
    };
    if begin == 0 || end < begin {
        return Vec::new();
    }
    let count = ((end - begin) / 4).min(MAX_PARTS);
    (0..count)
        .filter_map(|i| read_process_value::<u32>((begin + i * 4) as *const u32))
        .collect()
}

/// The enemy's `ExEmBreakParts`, the generic breakable-parts component (Proto Bahamut uses it).
/// It is not part of the enemy object: the actor keeps its components in a `std::map` (type
/// index -> component) at +0xC0, whose head node pointer is at +0xD0; nodes are
/// `{left, parent, right, colour, is_nil @+25, key @+32, component @+40}`. The type index is
/// assigned at run time, so the component is found by its class name.
const COMPONENT_MAP_HEAD: usize = 0xD0;
const MAX_MAP_NODES: usize = 512;

fn break_parts_component(instance: usize) -> Option<usize> {
    let head = read_process_value::<usize>((instance + COMPONENT_MAP_HEAD) as *const usize)?;
    let root = read_process_value::<usize>((head + 8) as *const usize)?;
    let mut stack = vec![root];
    let mut visited = 0;
    while let Some(node) = stack.pop() {
        if node == 0 || node == head || visited >= MAX_MAP_NODES {
            continue;
        }
        visited += 1;
        if read_process_value::<u8>((node + 25) as *const u8) != Some(0) {
            continue;
        }
        if let Some(component) = read_process_value::<usize>((node + 40) as *const usize) {
            if super::timeline::object_class(component).as_deref() == Some("ExEmBreakParts") {
                return Some(component);
            }
        }
        for link in [0, 16] {
            if let Some(child) = read_process_value::<usize>((node + link) as *const usize) {
                stack.push(child);
            }
        }
    }
    None
}

/// `ExEmBreakParts` layout (game 2.0.2, from its hit handler, vtable slot 17):
/// - +0x10/+0x18: a vector of 12-byte part states, `{remaining i32, ? i32, broken u8 @+8}`;
///   each hit on a part subtracts the hit's value from `remaining` (clamped at 0), and the part
///   breaks at 0 (then its index is added to `ExEmBreakPartsInfo`'s list);
/// - +0x28/+0x30: a vector of 56-byte part definitions, `breakable u8 @+8`.
const PART_STATES: usize = 0x10;
const PART_STATE_SIZE: usize = 12;
const PART_DEFINITIONS: usize = 0x28;
const PART_DEFINITION_SIZE: usize = 56;

/// An MSVC `std::string` (16-byte inline buffer or pointer, then size and capacity) holding a
/// short printable name.
fn read_name(at: usize) -> Option<String> {
    let size = read_process_value::<usize>((at + 16) as *const usize)?;
    let capacity = read_process_value::<usize>((at + 24) as *const usize)?;
    if size == 0 || size > 64 || capacity < size {
        return None;
    }
    let text = if capacity >= 16 {
        read_process_value::<usize>(at as *const usize)?
    } else {
        at
    };
    let bytes: Vec<u8> = (0..size)
        .map(|i| read_process_value::<u8>((text + i) as *const u8))
        .collect::<Option<_>>()?;
    let name = String::from_utf8(bytes).ok()?;
    name.chars()
        .all(|c| c.is_ascii_graphic() || c == ' ')
        .then_some(name)
}

/// Where a part definition keeps its name: the same 56-byte layout as the component's hit
/// effect records, whose string is at +16.
const PART_DEFINITION_NAME: usize = 16;

fn vector(at: usize, element: usize) -> Option<(usize, usize)> {
    let begin = read_process_value::<usize>(at as *const usize)?;
    let end = read_process_value::<usize>((at + 8) as *const usize)?;
    (begin != 0 && end >= begin).then(|| (begin, ((end - begin) / element).min(MAX_PARTS)))
}

fn component_parts(component: usize, previous: &[PartState]) -> Vec<PartState> {
    let Some((states, count)) = vector(component + PART_STATES, PART_STATE_SIZE) else {
        return Vec::new();
    };
    let definitions = vector(component + PART_DEFINITIONS, PART_DEFINITION_SIZE);
    let mut parts = Vec::new();
    let mut raw = Vec::new();
    for index in 0..count {
        let state = states + index * PART_STATE_SIZE;
        let (Some(remaining), Some(other), Some(broken)) = (
            read_process_value::<i32>(state as *const i32),
            read_process_value::<i32>((state + 4) as *const i32),
            read_process_value::<u8>((state + 8) as *const u8),
        ) else {
            continue;
        };
        raw.push(serde_json::json!([index, remaining, other, broken]));

        let name = definitions
            .filter(|(_, count)| index < *count)
            .and_then(|(begin, _)| {
                read_name(begin + index * PART_DEFINITION_SIZE + PART_DEFINITION_NAME)
            });
        let breakable = definitions.map_or(true, |(begin, count)| {
            index >= count
                || read_process_value::<u8>((begin + index * PART_DEFINITION_SIZE + 8) as *const u8)
                    .is_some_and(|flag| flag != 0)
        });
        if !breakable {
            continue;
        }
        let durability = if broken != 0 {
            0.0
        } else {
            remaining.max(0) as f32
        };
        let id = index as u32;
        let max_durability = previous
            .iter()
            .find(|part| part.id == id)
            .map_or(durability, |part| part.max_durability.max(durability));
        parts.push(PartState {
            id,
            name,
            durability,
            max_durability,
        });
    }
    // For the diagnostics timeline: the raw states, whenever a value changes (to confirm what
    // the second field holds).
    let changed = parts.len() != previous.len()
        || parts
            .iter()
            .zip(previous)
            .any(|(now, before)| now.durability != before.durability);
    if changed {
        super::timeline::record(
            "break_parts",
            serde_json::json!({
                "component": format!("{component:#x}"),
                "states": raw,
                "names": parts.iter().map(|part| (part.id, part.name.clone())).collect::<Vec<_>>(),
            }),
        );
    }
    parts
}

/// The parts to show: their break values where those are known (in the enemy object, or in its
/// `ExEmBreakParts`), otherwise the parts the game lists as broken (shown as broken, without a
/// value).
fn current_parts(
    instance: usize,
    offsets: &[usize],
    component: Option<usize>,
    previous: &[PartState],
) -> Vec<PartState> {
    if !offsets.is_empty() {
        return read_parts(instance, offsets, previous);
    }
    if let Some(component) = component {
        let parts = component_parts(component, previous);
        if !parts.is_empty() {
            return parts;
        }
    }
    broken_part_ids(instance)
        .into_iter()
        .map(|id| PartState {
            id,
            name: None,
            durability: 0.0,
            max_durability: 0.0,
        })
        .collect()
}

/// Current state of each part; `previous` keeps the highest break value seen per part.
fn read_parts(instance: usize, offsets: &[usize], previous: &[PartState]) -> Vec<PartState> {
    offsets
        .iter()
        .filter_map(|offset| {
            let id = read_process_value::<u32>((instance + offset) as *const u32)?;
            let durability = read_process_value::<f32>(
                (instance + offset - PART_DURABILITY_BEFORE_ID) as *const f32,
            )?;
            if !plausible_durability(durability) {
                return None;
            }
            let max_durability = previous
                .iter()
                .find(|part| part.id == id)
                .map_or(durability, |part| part.max_durability.max(durability));
            Some(PartState {
                id,
                name: None,
                durability: durability.max(0.0),
                max_durability,
            })
        })
        .collect()
}

struct Tracked {
    instance: usize,
    vtable: usize,
    type_hash: u32,
    hp: i64,
    max_hp: i64,
    last_hit: Instant,
    /// Last time it was hit or seen in the game's list of damageable actors.
    last_seen: Instant,
    defeated_at: Option<Instant>,
    /// Offsets of the part ids, found when the enemy is first hit.
    part_offsets: Vec<usize>,
    /// The enemy's `ExEmBreakParts`, looked up once when it is first tracked (the lookup walks a
    /// map, too slow for every hit).
    break_parts: Option<usize>,
    parts: Vec<PartState>,
    /// RTTI class name, e.g. `Em8000` or `We8091`.
    class: Option<String>,
    /// Whether it ever lost HP.
    damaged: bool,
}

static TRACKED: Mutex<Vec<Tracked>> = Mutex::new(Vec::new());
/// How often `snapshot` looks for enemies nobody hit yet.
const DISCOVER_EVERY: Duration = Duration::from_millis(500);
static LAST_DISCOVERY: Mutex<Option<Instant>> = Mutex::new(None);

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

/// Shots, bits and other behaviour objects (`Behavior*`, e.g. a boss's `BehaviorBh` bits with
/// 5,000 HP): they take damage but are not enemies worth a bar.
fn is_behavior_object(instance: usize) -> bool {
    super::rtti::class_name(instance).is_some_and(|class| class.starts_with("Behavior"))
}

fn is_player(type_hash: u32) -> bool {
    PLAYER_TYPE_HASHES.contains(&type_hash)
}

/// Records a damaged target; called after the game applied the damage. Returns the part that
/// this hit broke, if any.
pub fn observe(instance: usize, type_hash: u32) -> Option<PartBreak> {
    if instance == 0 || type_hash == 0 || is_player(type_hash) || is_behavior_object(instance) {
        return None;
    }
    let (hp, max_hp) = read_hp(instance)?;
    let vtable = read_process_value::<usize>(instance as *const usize)?;
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
        entry.damaged |= hp < max_hp;
        entry.last_hit = now;
        entry.last_seen = now;
        let before = std::mem::take(&mut entry.parts);
        entry.parts = current_parts(instance, &entry.part_offsets, entry.break_parts, &before);
        // Newly broken: broken now, and not already broken before this hit.
        let broke = entry.parts.iter().find(|after| {
            after.broken()
                && !before
                    .iter()
                    .any(|part| part.id == after.id && part.broken())
        });
        if let Some(part) = broke {
            info!("Enemy {type_hash:#x}: part {:#x} broke", part.id);
        }
        return broke.map(|part| (type_hash, part.id));
    }
    track(&mut tracked, instance, vtable, type_hash, hp, max_hp, now);
    None
}

/// Starts tracking an enemy: finds its part records once.
fn track(
    tracked: &mut Vec<Tracked>,
    instance: usize,
    vtable: usize,
    type_hash: u32,
    hp: i64,
    max_hp: i64,
    now: Instant,
) {
    if tracked.len() >= MAX_TRACKED {
        if let Some(oldest) = (0..tracked.len()).min_by_key(|&i| tracked[i].last_seen) {
            tracked.swap_remove(oldest);
        }
    }
    let class = class_number(instance);
    let part_offsets = class.map_or_else(Vec::new, |class| find_parts(instance, class));
    let break_parts = part_offsets
        .is_empty()
        .then(|| break_parts_component(instance))
        .flatten();
    let parts = current_parts(instance, &part_offsets, break_parts, &[]);
    info!(
        "Enemy {type_hash:#x} (class {}): {} part record(s) {}, ExEmBreakParts {}",
        class.map_or("?".into(), |class| format!("Em{class:04X}")),
        parts.len(),
        part_offsets
            .iter()
            .zip(&parts)
            .map(|(offset, part)| format!("+{offset:#x} id {:#x} {:.1}", part.id, part.durability))
            .collect::<Vec<_>>()
            .join(", "),
        break_parts.map_or("none".to_string(), |c| format!("{c:#x}"))
    );
    tracked.push(Tracked {
        instance,
        vtable,
        type_hash,
        hp,
        max_hp,
        last_hit: now,
        last_seen: now,
        defeated_at: None,
        part_offsets,
        break_parts,
        parts,
        class: super::statuses::class_name(vtable),
        damaged: hp < max_hp,
    });
}

/// Tracks enemies that are in the game's list of damageable actors, hit or not, and marks the
/// tracked ones still listed as seen. Throttled to `DISCOVER_EVERY`.
fn discover(tracked: &mut Vec<Tracked>, now: Instant) {
    {
        let mut last = LAST_DISCOVERY.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| now.duration_since(at) < DISCOVER_EVERY) {
            return;
        }
        *last = Some(now);
    }
    for actor in super::hurtboxes::actors() {
        if !actor.enemy || is_behavior_object(actor.instance) {
            continue;
        }
        if let Some(entry) = tracked.iter_mut().find(|t| t.instance == actor.instance) {
            entry.last_seen = now;
            continue;
        }
        let Some(type_hash) = super::static_type_id(actor.instance) else {
            continue;
        };
        if is_player(type_hash) {
            continue;
        }
        let (Some((hp, max_hp)), Some(vtable)) = (
            read_hp(actor.instance),
            read_process_value::<usize>(actor.instance as *const usize),
        ) else {
            continue;
        };
        // Already defeated ones that linger in the list are not worth a bar.
        if hp == 0 {
            continue;
        }
        track(tracked, actor.instance, vtable, type_hash, hp, max_hp, now);
    }
}

/// Logs each kind of damaged non-player object once (class, type, HP fields), to find objects
/// such as separate breakable parts that do not show up as enemies.
pub fn describe_target_once(instance: usize, type_hash: u32) {
    static SEEN: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
    let class = read_process_value::<usize>(instance as *const usize)
        .and_then(super::statuses::class_name)
        .unwrap_or_else(|| "?".into());
    if instance == 0 || is_player(type_hash) {
        return;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() >= 64 || seen.iter().any(|(c, t)| *c == class && *t == type_hash) {
        return;
    }
    seen.push((class.clone(), type_hash));
    info!(
        "Damaged object {class} type {type_hash:#x}: +0x160 {:?} +0x168 {:?}",
        read_process_value::<i64>((instance + HP_OFFSET) as *const i64),
        read_process_value::<i64>((instance + MAX_HP_OFFSET) as *const i64)
    );
}

/// Re-reads every tracked enemy and returns the ones still worth showing, biggest max HP
/// first (bosses before adds), then by most recent hit.
pub fn snapshot() -> Vec<EnemyHp> {
    let now = Instant::now();
    let mut tracked = lock();
    discover(&mut tracked, now);
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
        entry.damaged |= hp < max_hp;
        let previous = std::mem::take(&mut entry.parts);
        entry.parts = current_parts(
            entry.instance,
            &entry.part_offsets,
            entry.break_parts,
            &previous,
        );
        if hp == 0 {
            let defeated_at = *entry.defeated_at.get_or_insert(now);
            return now.duration_since(defeated_at) < DEFEATED_LINGER;
        }
        entry.defeated_at = None;
        now.duration_since(entry.last_seen) < IDLE_TIMEOUT
    });

    // `We*` objects that never lose HP are not enemies of their own: on Ghiyeti & Thorn
    // (game 2.0.2) Thorn's weapons (`We8090`, six `We8091`) take hits but keep Thorn's full HP
    // for the whole fight. `We*` targets that do lose HP (`We7300`) are kept.
    let mut order: Vec<&Tracked> = tracked
        .iter()
        .filter(|entry| entry.damaged || !entry.class.as_deref().is_some_and(is_hp_linked_class))
        .collect();
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
            parts: entry.parts.clone(),
            class: entry.class.clone(),
        })
        .collect()
}

/// Classes of objects that may only mirror a boss's HP (weapons and the like).
fn is_hp_linked_class(class: &str) -> bool {
    class.starts_with("We")
}

/// Tracked enemies as (type hash, instance), for diagnostics.
pub fn instances() -> Vec<(u32, usize)> {
    lock()
        .iter()
        .map(|entry| (entry.type_hash, entry.instance))
        .collect()
}

/// Type hash of a tracked enemy instance.
pub fn type_of(instance: usize) -> Option<u32> {
    lock()
        .iter()
        .find(|entry| entry.instance == instance)
        .map(|entry| entry.type_hash)
}

/// Whether a type hash is a player character.
pub fn is_player_type(type_hash: u32) -> bool {
    is_player(type_hash)
}

/// Forgets every enemy, e.g. at the result screen.
pub fn reset() {
    lock().clear();
}

/// Live state of the tracked enemies, for the diagnostics export.
pub fn probe() -> serde_json::Value {
    use serde_json::json;
    let tracked = lock();
    serde_json::Value::Array(
        tracked
            .iter()
            .map(|entry| {
                json!({
                    "instance": format!("{:#x}", entry.instance),
                    "class": read_process_value::<usize>(entry.instance as *const usize)
                        .and_then(super::statuses::class_name),
                    "type_hash": format!("{:#x}", entry.type_hash),
                    "hp_tracked": entry.hp,
                    "max_hp_tracked": entry.max_hp,
                    "hp_live": read_hp(entry.instance).map(|(hp, _)| hp),
                })
            })
            .collect(),
    )
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
