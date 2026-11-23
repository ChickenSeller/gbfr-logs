//! MSVC RTTI helpers: where a base class sits inside an object.
//!
//! An object's primary vtable is preceded by its complete object locator, whose class hierarchy
//! descriptor lists every base class with its displacement (`mdisp`). That gives the offset of a
//! mix-in base such as `ExCollision` in any actor class, without per-class offsets. Results are
//! cached per primary vtable. All reads go through `read_process_value`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::read_process_value;

static GAME_BASE: OnceLock<usize> = OnceLock::new();
static BASES: OnceLock<Mutex<HashMap<usize, Vec<(String, usize)>>>> = OnceLock::new();

pub fn setup(game_base: usize) {
    let _ = GAME_BASE.set(game_base);
}

fn read_name(address: usize) -> Option<String> {
    let bytes = read_process_value::<[u8; 128]>(address as *const [u8; 128])?;
    let length = bytes.iter().position(|byte| *byte == 0)?;
    let raw = std::str::from_utf8(&bytes[..length]).ok()?;
    Some(
        raw.trim_start_matches(".?AV")
            .trim_end_matches("@@")
            .to_string(),
    )
}

/// Every base class of the object with this primary vtable, with its offset in the object.
fn bases_of(vtable: usize) -> Option<Vec<(String, usize)>> {
    let cache = BASES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(bases) = cache.lock().ok()?.get(&vtable) {
        return Some(bases.clone());
    }
    let base = *GAME_BASE.get()?;
    let locator = read_process_value::<usize>(vtable.checked_sub(8)? as *const usize)?;
    // Only a primary vtable (offset 0) describes the complete object from its start.
    if read_process_value::<u32>(locator as *const u32)? != 1
        || read_process_value::<u32>((locator + 4) as *const u32)? != 0
    {
        return None;
    }
    let hierarchy = base + read_process_value::<u32>((locator + 0x10) as *const u32)? as usize;
    let count = read_process_value::<u32>((hierarchy + 8) as *const u32)? as usize;
    if count == 0 || count > 1024 {
        return None;
    }
    let array = base + read_process_value::<u32>((hierarchy + 0x0C) as *const u32)? as usize;
    let mut bases = Vec::with_capacity(count);
    for index in 0..count {
        let descriptor =
            base + read_process_value::<u32>((array + index * 4) as *const u32)? as usize;
        let type_descriptor = base + read_process_value::<u32>(descriptor as *const u32)? as usize;
        let mdisp = read_process_value::<i32>((descriptor + 8) as *const i32)?;
        if let (Some(name), Ok(offset)) =
            (read_name(type_descriptor + 0x10), usize::try_from(mdisp))
        {
            bases.push((name, offset));
        }
    }
    if let Ok(mut cache) = cache.lock() {
        cache.insert(vtable, bases.clone());
    }
    Some(bases)
}

/// The game's base address, once `setup` ran.
pub fn game_base() -> Option<usize> {
    GAME_BASE.get().copied()
}

/// The object a sub-object (such as a mix-in base) belongs to, from the offset recorded in its
/// vtable's complete object locator.
pub fn complete_object(sub_object: usize) -> Option<usize> {
    let vtable = read_process_value::<usize>(sub_object as *const usize)?;
    let locator = read_process_value::<usize>(vtable.checked_sub(8)? as *const usize)?;
    if read_process_value::<u32>(locator as *const u32)? != 1 {
        return None;
    }
    let offset = read_process_value::<u32>((locator + 4) as *const u32)? as usize;
    sub_object.checked_sub(offset)
}

/// Undecorated class name of `instance` (the first entry of its class hierarchy).
pub fn class_name(instance: usize) -> Option<String> {
    let vtable = read_process_value::<usize>(instance as *const usize)?;
    bases_of(vtable)?.into_iter().next().map(|(name, _)| name)
}

/// Whether `instance` derives from `name`.
pub fn has_base(instance: usize, name: &str) -> bool {
    base_offset(instance, name).is_some()
}

/// Offset of the base class `name` (undecorated, e.g. `ExCollision`) inside `instance`.
pub fn base_offset(instance: usize, name: &str) -> Option<usize> {
    let vtable = read_process_value::<usize>(instance as *const usize)?;
    bases_of(vtable)?
        .into_iter()
        .find(|(base, _)| base == name)
        .map(|(_, offset)| offset)
}
