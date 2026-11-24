//! Hurtboxes ("damage hits") of an actor, for the overlay's debug view.
//!
//! They come from the model's `_N_DamageHit.bxm` (`typeName_` SPHERE / CAPSULE / OBB, `pos1_`,
//! `pos2_`, `radius_`, `rotate_`, `jointNo1_`, `jointNo2_`, `uniqueId_`) and live in the actor's
//! `ExDamage` base (`ExEmDamage` for enemies, `ExPlDamage` for players; its offset varies per
//! class, so it is found through RTTI): records at `+0x78`, count at `+0x88`. Each 0x290-byte
//! record holds its local shape and the world shape the game recomputes from the bound joints
//! (game 2.0.2, from `ExDamage` setup at 0x141FB61A0 and the update at 0x1421BA080):
//!
//! - `+0x00` u8 enabled, `+0x10` u32 unique id, `+0x28` u32 shape (0 sphere, 2 capsule, 3 OBB);
//! - sphere: world centre `+0x70`, radius `+0x80`;
//! - capsule: world ends `+0x100` and `+0x110`, radius `+0x160`;
//! - OBB: world centre `+0x1D0`, unit axes `+0x1E0`, `+0x1F0`, `+0x200`, half extents `+0x210`;
//! - world AABB `+0x250` (min) and `+0x260` (max).

use super::read_process_value;

const EX_DAMAGE: &str = "ExDamage";
const RECORDS: usize = 0x78;
const COUNT: usize = 0x88;
const RECORD_SIZE: usize = 0x290;
/// More than any model defines; guards against reading a wrong offset.
const MAX_RECORDS: usize = 512;

const SHAPE_CAPSULE: u32 = 2;
const SHAPE_OBB: u32 = 3;

pub type Vec3 = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere {
        center: Vec3,
        radius: f32,
    },
    Capsule {
        a: Vec3,
        b: Vec3,
        radius: f32,
    },
    Obb {
        center: Vec3,
        axes: [Vec3; 3],
        half: Vec3,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hurtbox {
    pub enabled: bool,
    pub unique_id: u32,
    pub shape: Shape,
}

fn f32_at(record: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap())
}

fn u32_at(record: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap())
}

fn vec3_at(record: &[u8], offset: usize) -> Vec3 {
    [
        f32_at(record, offset),
        f32_at(record, offset + 4),
        f32_at(record, offset + 8),
    ]
}

fn plausible(v: f32) -> bool {
    v.is_finite() && v.abs() < 1.0e6
}

fn plausible_size(v: f32) -> bool {
    v.is_finite() && (0.0..1000.0).contains(&v)
}

/// The world shape of one record, if its numbers are plausible.
pub fn parse(record: &[u8]) -> Option<Hurtbox> {
    if record.len() < RECORD_SIZE {
        return None;
    }
    let shape = match u32_at(record, 0x28) {
        SHAPE_CAPSULE => Shape::Capsule {
            a: vec3_at(record, 0x100),
            b: vec3_at(record, 0x110),
            radius: f32_at(record, 0x160),
        },
        SHAPE_OBB => Shape::Obb {
            center: vec3_at(record, 0x1D0),
            axes: [
                vec3_at(record, 0x1E0),
                vec3_at(record, 0x1F0),
                vec3_at(record, 0x200),
            ],
            half: vec3_at(record, 0x210),
        },
        _ => Shape::Sphere {
            center: vec3_at(record, 0x70),
            radius: f32_at(record, 0x80),
        },
    };
    let ok = match shape {
        Shape::Sphere { center, radius } => {
            center.iter().all(|v| plausible(*v)) && plausible_size(radius)
        }
        Shape::Capsule { a, b, radius } => {
            a.iter().chain(b.iter()).all(|v| plausible(*v)) && plausible_size(radius)
        }
        Shape::Obb { center, axes, half } => {
            center.iter().all(|v| plausible(*v))
                && axes
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && v.abs() <= 1.01)
                && half.iter().all(|v| plausible_size(*v))
        }
    };
    ok.then_some(Hurtbox {
        enabled: record[0] != 0,
        unique_id: u32_at(record, 0x10),
        shape,
    })
}

/// Every live `ExDamage` registers itself in a global vector (begin, end) at this RVA (game
/// 2.0.2; pushed in 0x141FBAC10, removed in 0x141FBB1F0), so damageable actors can be found
/// before anyone hits them.
const REGISTRY_RVA: usize = 0x736_A9F8;
const MAX_ACTORS: usize = 1024;

#[derive(Clone, Debug)]
pub struct DamageableActor {
    pub instance: usize,
    pub class: String,
    pub enemy: bool,
    pub player: bool,
}

/// Every actor that can currently take damage: enemies (`ExEmDamage`), players (`ExPlDamage`)
/// and anything else with an `ExDamage`, such as breakable objects.
pub fn actors() -> Vec<DamageableActor> {
    let Some(base) = super::rtti::game_base() else {
        return Vec::new();
    };
    let registry = base + REGISTRY_RVA;
    let (Some(begin), Some(end)) = (
        read_process_value::<usize>(registry as *const usize),
        read_process_value::<usize>((registry + 8) as *const usize),
    ) else {
        return Vec::new();
    };
    if begin == 0 || end < begin || (end - begin) % 8 != 0 || (end - begin) / 8 > MAX_ACTORS {
        return Vec::new();
    }
    let mut actors: Vec<DamageableActor> = Vec::new();
    for index in 0..(end - begin) / 8 {
        let Some(ex_damage) = read_process_value::<usize>((begin + index * 8) as *const usize)
        else {
            continue;
        };
        let Some(instance) = super::rtti::complete_object(ex_damage) else {
            continue;
        };
        // The derived class must place ExDamage exactly there, or this is not an actor.
        if super::rtti::base_offset(instance, EX_DAMAGE) != Some(ex_damage - instance) {
            continue;
        }
        if actors.iter().any(|actor| actor.instance == instance) {
            continue;
        }
        actors.push(DamageableActor {
            instance,
            class: super::rtti::class_name(instance).unwrap_or_default(),
            enemy: super::rtti::has_base(instance, "ExEmDamage"),
            player: super::rtti::has_base(instance, "ExPlDamage"),
        });
    }
    actors
}

/// Shapes bigger than this (radius or half extent) are area volumes, not bodies.
const MAX_BODY_SIZE: f32 = 30.0;

impl Shape {
    /// Its largest radius or half extent.
    pub fn size(&self) -> f32 {
        match *self {
            Shape::Sphere { radius, .. } | Shape::Capsule { radius, .. } => radius,
            Shape::Obb { half, .. } => half[0].max(half[1]).max(half[2]),
        }
    }

    /// Whether it is body-sized rather than an area volume spanning the scene.
    pub fn is_body_sized(&self) -> bool {
        self.size() <= MAX_BODY_SIZE
    }
}

impl DamageableActor {
    /// Whether it is a field object such as `Ba4002`: two letters and four hex digits. Shots and
    /// summons (`Behavior*`) and helper classes nested in others (`...@ExPlayerAI`) also take
    /// damage, but they are not things on the field worth outlining.
    pub fn is_field_object(&self) -> bool {
        let bytes = self.class.as_bytes();
        bytes.len() == 6
            && bytes[..2].iter().all(u8::is_ascii_alphabetic)
            && bytes[2..].iter().all(u8::is_ascii_hexdigit)
    }
}

/// Where `instance`'s hurtbox records are and how many bytes they take, for tracing.
pub fn records_region(instance: usize) -> Option<(usize, usize)> {
    let offset = super::rtti::base_offset(instance, EX_DAMAGE)?;
    let ex_damage = instance + offset;
    let records = read_process_value::<usize>((ex_damage + RECORDS) as *const usize)?;
    let count = read_process_value::<usize>((ex_damage + COUNT) as *const usize)?;
    (records != 0 && count > 0 && count <= MAX_RECORDS).then_some((records, count * RECORD_SIZE))
}

/// Every hurtbox of `instance`, enabled or not. Empty if it has no `ExDamage`.
pub fn of(instance: usize) -> Vec<Hurtbox> {
    let Some(offset) = super::rtti::base_offset(instance, EX_DAMAGE) else {
        return Vec::new();
    };
    let ex_damage = instance + offset;
    let (Some(records), Some(count)) = (
        read_process_value::<usize>((ex_damage + RECORDS) as *const usize),
        read_process_value::<usize>((ex_damage + COUNT) as *const usize),
    ) else {
        return Vec::new();
    };
    if records == 0 || count > MAX_RECORDS {
        return Vec::new();
    }
    (0..count)
        .filter_map(|index| {
            let address = records + index * RECORD_SIZE;
            let bytes = read_process_value::<[u8; RECORD_SIZE]>(address as *const _)?;
            parse(&bytes)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_f32(record: &mut [u8], offset: usize, v: f32) {
        record[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put_vec3(record: &mut [u8], offset: usize, v: Vec3) {
        for (i, x) in v.iter().enumerate() {
            put_f32(record, offset + i * 4, *x);
        }
    }

    #[test]
    fn parses_each_shape() {
        let mut record = vec![0u8; RECORD_SIZE];
        record[0] = 1;
        record[0x10..0x14].copy_from_slice(&7u32.to_le_bytes());
        put_vec3(&mut record, 0x70, [1.0, 2.0, 3.0]);
        put_f32(&mut record, 0x80, 0.5);
        assert_eq!(
            parse(&record),
            Some(Hurtbox {
                enabled: true,
                unique_id: 7,
                shape: Shape::Sphere {
                    center: [1.0, 2.0, 3.0],
                    radius: 0.5
                }
            })
        );

        record[0x28..0x2C].copy_from_slice(&SHAPE_CAPSULE.to_le_bytes());
        put_vec3(&mut record, 0x100, [0.0, 0.0, 0.0]);
        put_vec3(&mut record, 0x110, [0.0, 2.0, 0.0]);
        put_f32(&mut record, 0x160, 0.3);
        assert!(matches!(
            parse(&record).unwrap().shape,
            Shape::Capsule { radius, .. } if radius == 0.3
        ));

        record[0x28..0x2C].copy_from_slice(&SHAPE_OBB.to_le_bytes());
        put_vec3(&mut record, 0x1D0, [5.0, 1.0, 5.0]);
        put_vec3(&mut record, 0x1E0, [1.0, 0.0, 0.0]);
        put_vec3(&mut record, 0x1F0, [0.0, 1.0, 0.0]);
        put_vec3(&mut record, 0x200, [0.0, 0.0, 1.0]);
        put_vec3(&mut record, 0x210, [1.0, 2.0, 3.0]);
        assert!(matches!(
            parse(&record).unwrap().shape,
            Shape::Obb { half, .. } if half == [1.0, 2.0, 3.0]
        ));
    }

    #[test]
    fn rejects_garbage() {
        let mut record = vec![0u8; RECORD_SIZE];
        put_f32(&mut record, 0x70, f32::NAN);
        put_f32(&mut record, 0x80, 1.0);
        assert_eq!(parse(&record), None);
        put_f32(&mut record, 0x70, 0.0);
        put_f32(&mut record, 0x80, -1.0);
        assert_eq!(parse(&record), None);
    }
}
