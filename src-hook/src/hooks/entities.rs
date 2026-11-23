//! Every character on the field (players, NPCs, enemies), damageable or not, for the overlay's
//! debug view.
//!
//! The game resolves entity handles through a global table (game 2.0.2): `*(base + 0x7020988)`
//! points at a header with plain arrays of entity pointers at `+0x20` and generations at `+0x48`,
//! and a slot bitmap allocator at `+0x50` whose first field is the capacity. Registering an
//! entity (0x1402304E0) writes its slot `+ 1` to `entity + 0x38`, the entity itself to `+0x40`
//! and the generation to `+0x48`, which lets a slot be checked. An entity's actor object is at
//! `entity + 0x70`. Actors are told
//! apart by their RTTI class name (`Pl0300`, `Np0000`, `Em1806`, ...).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::read_process_value;

const TABLE_RVA: usize = 0x702_0988;
const ENTITIES: usize = 0x20;
const CAPACITY: usize = 0x50;
const ENTITY_SLOT: usize = 0x38;
const ENTITY_ACTOR: usize = 0x70;
const MAX_ENTITIES: usize = 1 << 17;
const CHUNK: usize = 256;
/// Scanning the table is a few thousand reads; the list is refreshed at most this often.
const REFRESH_EVERY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Player,
    Npc,
    Enemy,
}

#[derive(Clone, Debug)]
pub struct Character {
    pub instance: usize,
    pub class: String,
    pub kind: Kind,
}

struct Cache {
    at: Option<Instant>,
    entities: usize,
    characters: Vec<Character>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    at: None,
    entities: 0,
    characters: Vec::new(),
});

/// Townsfolk and animals in towns (seen in the entity table of Folca); the navigation and
/// model-less helpers of village NPCs are left out.
const TOWNSFOLK_CLASSES: [&str; 6] = [
    "VillageNPCObject",
    "VillageNPCEx",
    "BehaviorNormalNpcBase",
    "BehaviorNormalNpcPlayer",
    "NpReceptionist",
    "VillageCreature",
];

/// `Pl0300` -> player, `Np0000` -> NPC, `Em1806` -> enemy (two letters and four hex digits),
/// and the townsfolk classes -> NPC.
pub fn kind_of(class: &str) -> Option<Kind> {
    if TOWNSFOLK_CLASSES.contains(&class) {
        return Some(Kind::Npc);
    }
    let bytes = class.as_bytes();
    if bytes.len() != 6 || !bytes[2..].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    match &class[..2] {
        "Pl" => Some(Kind::Player),
        "Np" => Some(Kind::Npc),
        "Em" => Some(Kind::Enemy),
        _ => None,
    }
}

/// Logs each class found in the entity table that is not a character, once, to learn which
/// prefixes other characters (townsfolk, companions) use.
fn note_other_class(class: &str) {
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() >= 200 || seen.iter().any(|c| c == class) {
        return;
    }
    seen.push(class.to_string());
    log::info!("Entity table class: {class}");
}

fn scan() -> Option<(usize, Vec<Character>)> {
    let base = super::rtti::game_base()?;
    let header = read_process_value::<usize>((base + TABLE_RVA) as *const usize)?;
    let begin = read_process_value::<usize>((header + ENTITIES) as *const usize)?;
    let count = read_process_value::<usize>((header + CAPACITY) as *const usize)?;
    if header == 0 || begin == 0 || count == 0 || count > MAX_ENTITIES {
        return None;
    }
    let mut characters = Vec::new();
    for chunk_start in (0..count).step_by(CHUNK) {
        let Some(slots) =
            read_process_value::<[usize; CHUNK]>((begin + chunk_start * 8) as *const _)
        else {
            continue;
        };
        for (offset, &entity) in slots.iter().take(count - chunk_start).enumerate() {
            if entity == 0 || entity % 8 != 0 {
                continue;
            }
            // A live entity records its own slot (+ 1) and itself; anything else is stale.
            let slot = chunk_start + offset;
            let Some([index, own]) =
                read_process_value::<[usize; 2]>((entity + ENTITY_SLOT) as *const [usize; 2])
            else {
                continue;
            };
            if index as u32 as usize != slot + 1 || own != entity {
                continue;
            }
            let Some(instance) =
                read_process_value::<usize>((entity + ENTITY_ACTOR) as *const usize)
            else {
                continue;
            };
            if instance == 0 || instance % 8 != 0 {
                continue;
            }
            let Some(class) = super::rtti::class_name(instance) else {
                continue;
            };
            if kind_of(&class).is_none() {
                note_other_class(&class);
            }
            if let Some(kind) = kind_of(&class) {
                if !characters
                    .iter()
                    .any(|c: &Character| c.instance == instance)
                {
                    characters.push(Character {
                        instance,
                        class,
                        kind,
                    });
                }
            }
        }
    }
    Some((count, characters))
}

/// Characters on the field and the size of the entity table (0 if it could not be read).
pub fn characters() -> (usize, Vec<Character>) {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.at.map_or(true, |at| at.elapsed() >= REFRESH_EVERY) {
        let (entities, characters) = scan().unwrap_or_default();
        cache.at = Some(Instant::now());
        cache.entities = entities;
        cache.characters = characters;
    }
    (cache.entities, cache.characters.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_character_classes() {
        assert_eq!(kind_of("Pl0300"), Some(Kind::Player));
        assert_eq!(kind_of("Np0000"), Some(Kind::Npc));
        assert_eq!(kind_of("Em1806"), Some(Kind::Enemy));
        assert_eq!(kind_of("Em76a0"), Some(Kind::Enemy));
        assert_eq!(kind_of("Pl0700Ghost"), None);
        assert_eq!(kind_of("BehaviorShot"), None);
        assert_eq!(kind_of("Ba4002"), None);
        assert_eq!(kind_of("VillageNPCObject"), Some(Kind::Npc));
        assert_eq!(kind_of("VillageNPCNavAvoidObject"), None);
    }
}
