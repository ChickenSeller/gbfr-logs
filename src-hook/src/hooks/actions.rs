//! The action a party member is in, to recognise being grabbed (throws, binds).
//!
//! A character's current action number is the u32 at `instance + 0x40` (then its phase at `+0x44`
//! and sub-action at `+0x48`; game 2.0.2, seen changing with every move). Its `ExActManager` base
//! maps action numbers to action objects in two `std::unordered_map<u32, ActBase*>`, at `+0x18`
//! and `+0x58` (the second holds the higher numbers): each with its list head (sentinel node
//! pointer) 8 bytes in and its size 16 bytes in, and nodes `{next, prev, key, value}`. The value
//! points into the character, inside the `ExAct*` base that implements the action, so RTTI
//! names it (the most derived class at that spot).

use std::collections::HashMap;
use std::sync::Mutex;

use super::read_process_value;

const CURRENT_ACTION: usize = 0x40;
/// The two maps: list head offsets; the size follows each.
const MAP_LISTS: [usize; 2] = [0x20, 0x60];
const MAX_ACTIONS: usize = 512;
/// Action objects lie this far into a character at most.
const MAX_ACTION_OFFSET: usize = 0x20000;

/// Actions in which an enemy holds the character: being grabbed, thrown or bound.
pub const GRABBED_ACTIONS: [&str; 7] = [
    "ExActDamageCapture",
    "ExActDamageForceBind",
    "ExActDamageEm8300WaterBallBind",
    "ExActCatchedEnemy",
    "ExActStopBind",
    "ExActAirBind",
    "ExActEm0080Catch",
];

/// Action number -> action class, per character (by its vtable: same class, same table).
static NAMES: Mutex<Vec<(usize, HashMap<u32, String>)>> = Mutex::new(Vec::new());

fn read_names(instance: usize) -> Option<HashMap<u32, String>> {
    let offset = super::rtti::base_offset(instance, "ExActManager")?;
    let manager = instance + offset;
    let mut names = HashMap::new();
    for list in MAP_LISTS {
        let Some(sentinel) = read_process_value::<usize>((manager + list) as *const usize) else {
            continue;
        };
        let size = read_process_value::<usize>((manager + list + 8) as *const usize).unwrap_or(0);
        if sentinel == 0 || size > MAX_ACTIONS {
            continue;
        }
        let Some(mut node) = read_process_value::<usize>(sentinel as *const usize) else {
            continue;
        };
        for _ in 0..size {
            if node == sentinel || node == 0 {
                break;
            }
            let Some([next, _prev, key, value]) =
                read_process_value::<[usize; 4]>(node as *const _)
            else {
                break;
            };
            if let Some(inside) = value
                .checked_sub(instance)
                .filter(|o| *o < MAX_ACTION_OFFSET)
            {
                if let Some((name, _)) = super::rtti::ex_base_at(instance, inside) {
                    names.entry(key as u32).or_insert(name);
                }
            }
            node = next;
        }
    }
    Some(names)
}

/// The number and class of the action `instance` is in, e.g. `(7, "ExActWait")`.
pub fn current(instance: usize) -> Option<(u32, String)> {
    let action = read_process_value::<u32>((instance + CURRENT_ACTION) as *const u32)?;
    let vtable = read_process_value::<usize>(instance as *const usize)?;
    let mut cache = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if !cache.iter().any(|(v, _)| *v == vtable) {
        let names = read_names(instance)?;
        if names.is_empty() {
            return None;
        }
        log::info!(
            "Actions of {}: {}",
            super::rtti::class_name(instance).unwrap_or_default(),
            {
                let mut list: Vec<_> = names.iter().collect();
                list.sort();
                list.iter()
                    .map(|(k, v)| format!("{k:#x}={v}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        );
        cache.push((vtable, names));
    }
    let (_, names) = cache.iter().find(|(v, _)| *v == vtable)?;
    Some((action, names.get(&action).cloned().unwrap_or_default()))
}

/// Whether an action class holds the character in an enemy's grab.
pub fn is_grabbed(name: &str) -> bool {
    GRABBED_ACTIONS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grabs_are_recognised() {
        assert!(is_grabbed("ExActDamageCapture"));
        assert!(!is_grabbed("ExActWait"));
        assert!(!is_grabbed(""));
    }
}
