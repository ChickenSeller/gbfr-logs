//! Status effects (buffs, debuffs, ailments) on party members.
//!
//! Player characters derive from `ExStatus` (multiple inheritance): its sub-object sits at
//! `instance + 0xAE0`, set up by the player base constructor (game 2.0.2), and its active statuses
//! are a vector of `StatusBase*` at `+0x18..+0x20` of that sub-object. The derived class replaces
//! the sub-object's vtable with its own, so it is checked through RTTI: the vtable's complete
//! object locator must place it at offset `0xAE0`. Party members seen in damage events are
//! polled every `POLL_INTERVAL`; a status object that appears is reported as applied, one that
//! disappears as removed. Durations come from these two moments, so they do not depend on the
//! status' own timer fields.
//!
//! Status classes are named by their RTTI, e.g. `StatusAilmentPoison`. Reads go through
//! `read_process_value`, so a freed actor or status never crashes the game; the `ExStatus`
//! vtable check drops actors that are gone.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use log::info;
use protocol::{Message, StatusEvent};

use super::read_process_value;
use crate::event;

/// Offset of the `ExStatus` base sub-object in player character instances.
const EX_STATUS_OFFSET: usize = 0xAE0;
/// Active statuses, a vector of `StatusBase*`, inside `ExStatus`.
const STATUS_VECTOR_BEGIN: usize = 0x18;
const STATUS_VECTOR_END: usize = 0x20;
const MAX_STATUSES: usize = 128;

/// `StatusBase` fields.
const STATUS_TYPE_OFFSET: usize = 0x40;
/// Entity handle of whoever applied the status: index, entity pointer, generation.
const STATUS_SOURCE_ENTITY_OFFSET: usize = 0x30;
/// `entity + 0x70` is the actor instance, as in damage events.
const ENTITY_INSTANCE_OFFSET: usize = 0x70;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_TRACKED_ACTORS: usize = 16;

struct TrackedActor {
    instance: usize,
    actor_index: u32,
    actor_type: u32,
    /// Active statuses by object address.
    active: HashMap<usize, StatusEvent>,
}

static GAME_BASE: OnceLock<usize> = OnceLock::new();
static ACTORS: Mutex<Vec<TrackedActor>> = Mutex::new(Vec::new());
static CLASS_NAMES: OnceLock<Mutex<HashMap<usize, Option<String>>>> = OnceLock::new();

/// Starts the polling thread.
pub fn setup(game_base: usize, tx: event::Tx) {
    let _ = GAME_BASE.set(game_base);
    crate::shutdown::spawn(move || {
        while crate::shutdown::sleep(POLL_INTERVAL) {
            poll(&tx);
            log_action_changes();
        }
    });
    info!("Status tracking enabled");
}

/// Remembers a party member to watch; called from damage events.
pub fn track(instance: usize, actor_index: u32, actor_type: u32) {
    if instance == 0 {
        return;
    }
    let mut actors = ACTORS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(actor) = actors.iter_mut().find(|actor| actor.instance == instance) {
        actor.actor_index = actor_index;
        actor.actor_type = actor_type;
        return;
    }
    if !has_ex_status(instance) {
        return;
    }
    if actors.len() >= MAX_TRACKED_ACTORS {
        actors.remove(0);
    }
    info!("Tracking statuses of actor {actor_index} ({actor_type:#x})");
    actors.push(TrackedActor {
        instance,
        actor_index,
        actor_type,
        active: HashMap::new(),
    });
}

/// Forgets every actor, e.g. at the result screen. Their statuses end with the encounter.
pub fn reset() {
    ACTORS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Class name from a vtable's RTTI (complete object locator at `vtable - 8`).
pub(super) fn class_name(vtable: usize) -> Option<String> {
    let cache = CLASS_NAMES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(name) = cache.lock().ok()?.get(&vtable) {
        return name.clone();
    }
    let name = (|| {
        let base = *GAME_BASE.get()?;
        let locator = read_process_value::<usize>((vtable - 8) as *const usize)?;
        if read_process_value::<u32>(locator as *const u32)? != 1 {
            return None;
        }
        let descriptor_rva = read_process_value::<u32>((locator + 0x0C) as *const u32)? as usize;
        let bytes =
            read_process_value::<[u8; 96]>((base + descriptor_rva + 0x10) as *const [u8; 96])?;
        let length = bytes.iter().position(|byte| *byte == 0)?;
        let raw = std::str::from_utf8(&bytes[..length]).ok()?;
        Some(
            raw.trim_start_matches(".?AV")
                .trim_end_matches("@@")
                .to_string(),
        )
    })();
    if let Ok(mut cache) = cache.lock() {
        cache.insert(vtable, name.clone());
    }
    name
}

/// Address of the actor's `ExStatus` sub-object, or `None` if it has none (or is gone).
///
/// The sub-object's vtable belongs to the derived class, so it cannot be compared with
/// `ExStatus::vftable`; its RTTI complete object locator records where the sub-object sits in
/// the complete object, which must be `EX_STATUS_OFFSET`.
fn ex_status(instance: usize) -> Option<usize> {
    let address = instance + EX_STATUS_OFFSET;
    let vtable = read_process_value::<usize>(address as *const usize)?;
    let locator = read_process_value::<usize>(vtable.checked_sub(8)? as *const usize)?;
    let signature = read_process_value::<u32>(locator as *const u32)?;
    let offset = read_process_value::<u32>((locator + 4) as *const u32)?;
    (signature == 1 && offset as usize == EX_STATUS_OFFSET).then_some(address)
}

fn has_ex_status(instance: usize) -> bool {
    ex_status(instance).is_some()
}

/// Type of whoever applied the status, if it is a known party member or a damaged enemy.
/// Game code is never called from this thread: the source may already be freed.
fn source_type(status: usize, actors: &[TrackedActor]) -> Option<u32> {
    let entity =
        read_process_value::<usize>((status + STATUS_SOURCE_ENTITY_OFFSET) as *const usize)?;
    if entity == 0 {
        return None;
    }
    let instance = read_process_value::<usize>((entity + ENTITY_INSTANCE_OFFSET) as *const usize)?;
    if instance == 0 {
        return None;
    }
    actors
        .iter()
        .find(|actor| actor.instance == instance)
        .map(|actor| actor.actor_type)
        .or_else(|| super::enemy_hp::type_of(instance))
}

/// The statuses currently on an actor, or `None` if the actor is gone.
fn read_statuses(
    actor: &TrackedActor,
    actors: &[TrackedActor],
) -> Option<HashMap<usize, StatusEvent>> {
    let manager = ex_status(actor.instance)?;
    let begin = read_process_value::<usize>((manager + STATUS_VECTOR_BEGIN) as *const usize)?;
    let end = read_process_value::<usize>((manager + STATUS_VECTOR_END) as *const usize)?;
    if begin == 0 || end < begin {
        return Some(HashMap::new());
    }
    let count = ((end - begin) / 8).min(MAX_STATUSES);

    let mut statuses = HashMap::new();
    for i in 0..count {
        let Some(status) = read_process_value::<usize>((begin + i * 8) as *const usize) else {
            continue;
        };
        if status == 0 {
            continue;
        }
        // Keep what is already known: the source may be gone by now.
        if let Some(known) = actor.active.get(&status) {
            statuses.insert(status, known.clone());
            continue;
        }
        let Some(name) = read_process_value::<usize>(status as *const usize).and_then(class_name)
        else {
            continue;
        };
        let type_id =
            read_process_value::<u32>((status + STATUS_TYPE_OFFSET) as *const u32).unwrap_or(0);
        statuses.insert(
            status,
            StatusEvent {
                actor_index: actor.actor_index,
                actor_type: actor.actor_type,
                status_id: status as u64,
                class_name: name,
                type_id,
                source_type: source_type(status, actors),
            },
        );
    }
    // Being held in an enemy's grab counts as a debuff of its own, so the damage taken view
    // counts throws with their time held and the damage taken meanwhile.
    if let Some((action, name)) = super::actions::current(actor.instance) {
        if super::actions::is_grabbed(&name) {
            // Odd, so never a status object's address.
            let key = actor.instance | 1;
            let grab = actor.active.get(&key).cloned().unwrap_or(StatusEvent {
                actor_index: actor.actor_index,
                actor_type: actor.actor_type,
                status_id: key as u64,
                class_name: GRAB_STATUS.to_string(),
                type_id: GRAB_TYPE_BASE | action,
                source_type: None,
            });
            statuses.insert(key, grab);
        }
    }
    Some(statuses)
}

/// The status a grab is reported as; the app's damage taken view lists it with the debuffs.
const GRAB_STATUS: &str = "StatusDebuffGrab";
/// `type_id` of a grab: this, with the action number in the low bits.
const GRAB_TYPE_BASE: u32 = 0xFFFF_0000;

fn poll(tx: &event::Tx) {
    let mut actors = ACTORS.lock().unwrap_or_else(|e| e.into_inner());
    let mut gone = Vec::new();
    for index in 0..actors.len() {
        let Some(current) = read_statuses(&actors[index], &actors) else {
            gone.push(index);
            continue;
        };
        let actor = &mut actors[index];
        for (id, status) in &actor.active {
            if !current.contains_key(id) {
                let _ = tx.send(Message::StatusRemoved(status.clone()));
            }
        }
        for (id, status) in &current {
            if !actor.active.contains_key(id) {
                let _ = tx.send(Message::StatusApplied(status.clone()));
            }
        }
        actor.active = current;
    }
    for index in gone.into_iter().rev() {
        actors.remove(index);
    }
}

/// The actor that last acted for each party member, e.g. Id's dragon form instead of Id.
static ACTIVE: Mutex<Vec<(u32, usize, std::time::Instant)>> = Mutex::new(Vec::new());
/// How long an acting form stands in for the member after its last hit.
const ACTIVE_FOR: Duration = Duration::from_secs(5);

/// Records the instance that just dealt damage for a party member.
pub fn note_active(actor_index: u32, instance: usize) {
    if instance == 0 {
        return;
    }
    let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    active.retain(|(index, _, _)| *index != actor_index);
    active.push((actor_index, instance, std::time::Instant::now()));
}

/// Tracked party members as (actor index, instance), for the overlay's debug view. A form that
/// acted recently (Id's dragon) replaces the member's own instance.
pub fn party_instances() -> Vec<(u32, usize)> {
    let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    ACTORS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|actor| {
            let acting = active
                .iter()
                .find(|(index, _, at)| *index == actor.actor_index && at.elapsed() < ACTIVE_FOR)
                .map_or(actor.instance, |(_, instance, _)| *instance);
            (actor.actor_index, acting)
        })
        .collect()
}

/// RTTI class name of a vtable, for the diagnostics export.
pub fn class_name_of_vtable(vtable: usize) -> Option<String> {
    class_name(vtable)
}

/// The first `len` bytes of every tracked party member as floats, for the diagnostics export.
pub fn instance_floats(len: usize) -> serde_json::Value {
    let actors = ACTORS.lock().unwrap_or_else(|e| e.into_inner());
    serde_json::Value::Array(
        actors
            .iter()
            .map(|actor| {
                serde_json::json!({
                    "actor_index": actor.actor_index,
                    "actor_type": format!("{:#x}", actor.actor_type),
                    "floats": crate::diagnostics::floats(actor.instance, len),
                })
            })
            .collect(),
    )
}

/// Live state of the tracked party members, for the diagnostics export: their classes, the
/// `ExStatus` check and each active status with the raw fields around its timers.
pub fn probe() -> serde_json::Value {
    use serde_json::json;
    let actors = ACTORS.lock().unwrap_or_else(|e| e.into_inner());
    let raw = |status: usize| {
        json!({
            "type_0x40": read_process_value::<u32>((status + 0x40) as *const u32),
            "u32_0x4c": read_process_value::<u32>((status + 0x4C) as *const u32),
            "u32_0x50": read_process_value::<u32>((status + 0x50) as *const u32),
            "u8_0x5c": read_process_value::<u8>((status + 0x5C) as *const u8),
            "f32_0x7c": read_process_value::<f32>((status + 0x7C) as *const f32),
            "f32_0x80": read_process_value::<f32>((status + 0x80) as *const f32),
        })
    };
    serde_json::Value::Array(
        actors
            .iter()
            .map(|actor| {
                let statuses: Vec<_> = actor
                    .active
                    .iter()
                    .map(|(address, status)| {
                        json!({
                            "address": format!("{address:#x}"),
                            "class": status.class_name,
                            "type_id": status.type_id,
                            "source_type": status.source_type.map(|t| format!("{t:#x}")),
                            "raw": raw(*address),
                        })
                    })
                    .collect();
                json!({
                    "actor_index": actor.actor_index,
                    "actor_type": format!("{:#x}", actor.actor_type),
                    "instance": format!("{:#x}", actor.instance),
                    "instance_class": read_process_value::<usize>(actor.instance as *const usize)
                        .and_then(class_name),
                    "ex_status": ex_status(actor.instance).map(|address| format!("{address:#x}")),
                    "hp_0x160": read_process_value::<i64>((actor.instance + 0x160) as *const i64),
                    "hp_0x168": read_process_value::<i64>((actor.instance + 0x168) as *const i64),
                    "statuses": statuses,
                })
            })
            .collect(),
    )
}

/// Logs each party member's action changes by class (e.g. `ExActWait -> ExActDamageCapture`),
/// to confirm which actions grabs use. Diagnostic; stops after `MAX_LINES`.
fn log_action_changes() {
    const MAX_LINES: usize = 600;
    static LAST: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());
    static LINES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if LINES.load(std::sync::atomic::Ordering::Relaxed) >= MAX_LINES {
        return;
    }
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    for (index, instance) in party_instances() {
        let Some((action, name)) = super::actions::current(instance) else {
            continue;
        };
        match last.iter_mut().find(|(i, _)| *i == instance) {
            Some((_, previous)) if *previous == action => continue,
            Some((_, previous)) => *previous = action,
            None => last.push((instance, action)),
        }
        LINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let grabbed = if super::actions::is_grabbed(&name) {
            " (grabbed)"
        } else {
            ""
        };
        info!("Party member {index} action {action:#x} {name}{grabbed}");
    }
}
