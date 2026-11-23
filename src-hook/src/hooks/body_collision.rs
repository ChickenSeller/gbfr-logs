//! Body collision ("obj hit") of characters: the capsules that keep characters apart and that
//! NPCs, players and enemies have whether or not they can take damage.
//!
//! They live in the actor's `ExObjHit` base (offset varies per class, found through RTTI):
//! 0x190-byte records at `+0x28`, count at `+0x38` (game 2.0.2, from its clear at 0x140A06EB0).
//! A capsule record (set up by 0x14219FEC0, type 2 at `+0x14`) keeps its local ends at `+0x60` /
//! `+0x70` and radius at `+0xC0`, and the world ends the game recomputes from the bound joint
//! (0x1421A01D0) at `+0xD0` / `+0xE0` with the radius at `+0x130`. Other record types are not
//! decoded yet and are only counted.

use super::hurtboxes::Shape;
use super::read_process_value;

const EX_OBJ_HIT: &str = "ExObjHit";
const RECORDS: usize = 0x28;
const COUNT: usize = 0x38;
const RECORD_SIZE: usize = 0x190;
const MAX_RECORDS: usize = 64;
const TYPE_CAPSULE: u32 = 2;

fn f32_at(record: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap())
}

fn vec3_at(record: &[u8], offset: usize) -> [f32; 3] {
    [
        f32_at(record, offset),
        f32_at(record, offset + 4),
        f32_at(record, offset + 8),
    ]
}

/// The world capsule of one record, if it is a capsule with plausible numbers.
pub fn parse(record: &[u8]) -> Option<Shape> {
    if record.len() < RECORD_SIZE {
        return None;
    }
    let kind = u32::from_le_bytes(record[0x14..0x18].try_into().unwrap());
    if kind != TYPE_CAPSULE {
        return None;
    }
    let a = vec3_at(record, 0xD0);
    let b = vec3_at(record, 0xE0);
    let radius = f32_at(record, 0x130);
    let plausible = a
        .iter()
        .chain(b.iter())
        .all(|v| v.is_finite() && v.abs() < 1.0e6)
        && radius.is_finite()
        && (0.0..30.0).contains(&radius);
    plausible.then_some(Shape::Capsule { a, b, radius })
}

/// Where `instance` keeps its `ExObjHit` records, and the raw records (at most 4), for
/// diagnostics.
pub fn raw(instance: usize) -> Option<(usize, Vec<Vec<u8>>)> {
    let offset = super::rtti::base_offset(instance, EX_OBJ_HIT)?;
    let ex_obj_hit = instance + offset;
    let records = read_process_value::<usize>((ex_obj_hit + RECORDS) as *const usize)?;
    let count = read_process_value::<usize>((ex_obj_hit + COUNT) as *const usize)?;
    if records == 0 || count > MAX_RECORDS {
        return Some((offset, Vec::new()));
    }
    let raw = (0..count.min(4))
        .filter_map(|index| {
            read_process_value::<[u8; RECORD_SIZE]>((records + index * RECORD_SIZE) as *const _)
                .map(|bytes| bytes.to_vec())
        })
        .collect();
    Some((offset, raw))
}

/// Body capsules of `instance`, plus how many records were of a type not decoded yet.
pub fn of(instance: usize) -> (Vec<Shape>, usize) {
    let Some(offset) = super::rtti::base_offset(instance, EX_OBJ_HIT) else {
        return (Vec::new(), 0);
    };
    let ex_obj_hit = instance + offset;
    let (Some(records), Some(count)) = (
        read_process_value::<usize>((ex_obj_hit + RECORDS) as *const usize),
        read_process_value::<usize>((ex_obj_hit + COUNT) as *const usize),
    ) else {
        return (Vec::new(), 0);
    };
    if records == 0 || count > MAX_RECORDS {
        return (Vec::new(), 0);
    }
    let mut shapes = Vec::new();
    let mut other = 0;
    for index in 0..count {
        let address = records + index * RECORD_SIZE;
        let Some(bytes) = read_process_value::<[u8; RECORD_SIZE]>(address as *const _) else {
            continue;
        };
        match parse(&bytes) {
            Some(shape) => shapes.push(shape),
            None => other += 1,
        }
    }
    (shapes, other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capsules_only() {
        let mut record = vec![0u8; RECORD_SIZE];
        record[0x14..0x18].copy_from_slice(&TYPE_CAPSULE.to_le_bytes());
        for (offset, v) in [(0xD0, 1.0f32), (0xE4, 2.0), (0x130, 0.4)] {
            record[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert_eq!(
            parse(&record),
            Some(Shape::Capsule {
                a: [1.0, 0.0, 0.0],
                b: [0.0, 2.0, 0.0],
                radius: 0.4
            })
        );
        record[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(parse(&record), None);
    }
}
