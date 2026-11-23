//! Runtime probe for the app's diagnostics export.
//!
//! Setup steps record what they found (hooks installed, functions located) with `note`. When the
//! app asks (`OverlayMessage::DiagnosticsRequest`), `request_report` reads the live state of the
//! tracked party members and enemies and sends everything back as JSON in
//! `Message::DiagnosticsReport`. Reads go through `read_process_value`, never game code.

use std::sync::{Mutex, OnceLock};

use protocol::Message;
use serde_json::{json, Value};

use crate::event;

static TX: OnceLock<event::Tx> = OnceLock::new();
static GAME_BASE: OnceLock<usize> = OnceLock::new();
static NOTES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The game's base address, once the hooks are set up.
pub fn game_base() -> Option<usize> {
    GAME_BASE.get().copied()
}

pub fn setup(game_base: usize, tx: event::Tx) {
    let _ = GAME_BASE.set(game_base);
    let _ = TX.set(tx);
}

/// Records a setup result, e.g. `note("damage hook", "installed")`. Later notes with the same
/// key replace earlier ones.
pub fn note(key: impl Into<String>, value: impl Into<String>) {
    let (key, value) = (key.into(), value.into());
    let mut notes = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    notes.retain(|(existing, _)| *existing != key);
    notes.push((key, value));
}

/// Builds the report on its own thread, so the pipe reader is never held up.
pub fn request_report() {
    std::thread::spawn(|| {
        let report = report().to_string();
        if let Some(tx) = TX.get() {
            let _ = tx.send(Message::DiagnosticsReport(report));
        }
    });
}

fn report() -> Value {
    let notes: Vec<Value> = NOTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(key, value)| json!({ "key": key, "value": value }))
        .collect();
    json!({
        "hook_version": env!("CARGO_PKG_VERSION"),
        "game": game_info(),
        "setup": notes,
        "party": crate::hooks::statuses::probe(),
        "enemies": crate::hooks::enemy_hp::probe(),
        "cameras": cameras(),
        "collisions": collisions(),
        "party_floats": crate::hooks::statuses::instance_floats(PARTY_FLOATS),
        "enemy_floats": enemy_floats(),
        "characters": characters(),
        "terrain": terrain(),
    })
}

/// Global `cCameraApp` objects (RVAs of the six statics destroyed at exit, game 2.0.2).
const CAMERA_RVAS: [usize; 6] = [
    0x7C2_4860, 0x701_E1D0, 0x701_E600, 0x701_E9F0, 0x701_EDE0, 0x701_F1A0,
];
const CAMERA_SIZE: usize = 0x440;
/// Bytes of each party member instance dumped as floats, to find world positions.
const PARTY_FLOATS: usize = 0x2000;
/// Bytes of each tracked enemy dumped as floats, to find orientations.
const ENEMY_FLOATS: usize = 0x2000;

/// Floats of `len` bytes at `address`, rounded; unreadable words are null.
pub fn floats(address: usize, len: usize) -> Value {
    Value::Array(
        (0..len)
            .step_by(4)
            .map(|offset| {
                crate::hooks::read_value::<f32>((address + offset) as *const f32)
                    .filter(|value| value.is_finite() && value.abs() < 1.0e9)
                    .map_or(Value::Null, |value| {
                        json!((value * 10_000.0).round() / 10_000.0)
                    })
            })
            .collect(),
    )
}

/// The global camera objects as float arrays, to find the view and projection matrices.
fn cameras() -> Value {
    let base = GAME_BASE.get().copied().unwrap_or(0);
    if base == 0 {
        return Value::Null;
    }
    Value::Array(
        CAMERA_RVAS
            .iter()
            .map(|rva| {
                let address = base + rva;
                json!({
                    "rva": format!("{rva:#x}"),
                    "vtable_class": crate::hooks::read_value::<usize>(address as *const usize)
                        .and_then(crate::hooks::statuses::class_name_of_vtable),
                    "floats": floats(address, CAMERA_SIZE),
                })
            })
            .collect(),
    )
}

/// Loaded map collision files: hash, length and their first bytes as hex, to work out the
/// `.col` format.
fn terrain() -> Value {
    const DUMP: usize = 128 << 10;
    const DUMPED_FILES: usize = 4;
    let files = crate::hooks::terrain::loaded_files();
    Value::Array(
        files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let head = if index < DUMPED_FILES {
                    let bytes = crate::hooks::terrain::bytes(file, DUMP);
                    let mut hex = String::with_capacity(bytes.len() * 2);
                    for byte in bytes {
                        hex.push_str(&format!("{byte:02x}"));
                    }
                    Some(hex)
                } else {
                    None
                };
                json!({
                    "hash": format!("{:#x}", file.hash),
                    "data": format!("{:#x}", file.data),
                    "length": file.length,
                    "head_hex": head,
                })
            })
            .collect(),
    )
}

/// Every character and field object in the entity table: class, position, its first bytes as
/// floats, and its body collision records (offset, raw records as floats and as u32s).
fn characters() -> Value {
    const CHARACTER_FLOATS: usize = 0x400;
    let (entities, characters) = crate::hooks::entities::characters();
    let list: Vec<Value> = characters
        .iter()
        .map(|character| {
            let instance = character.instance;
            let body = crate::hooks::body_collision::raw(instance);
            let as_f32 = |bytes: &[u8]| -> Vec<Value> {
                bytes
                    .chunks_exact(4)
                    .map(|c| {
                        let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                        if v.is_finite() && v.abs() < 1.0e7 {
                            json!((v * 10_000.0).round() / 10_000.0)
                        } else {
                            Value::Null
                        }
                    })
                    .collect()
            };
            let as_u32 = |bytes: &[u8]| -> Vec<String> {
                bytes
                    .chunks_exact(4)
                    .map(|c| format!("{:x}", u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                    .collect()
            };
            // The joint the first body record is bound to, and the joints around it, to find
            // where the animated body is (walking townsfolk drift from their root).
            let joint = body
                .as_ref()
                .and_then(|(_, raw)| raw.first())
                .map(|r| usize::from_le_bytes(r[0x38..0x40].try_into().unwrap()))
                .filter(|j| *j > 0x10000 && j % 16 == 0);
            json!({
                "joint": joint.map(|j| format!("{j:#x}")),
                "joints_from_minus_0x400": joint.map(|j| floats(j - 0x400, 0x1400)),
                "class": character.class,
                "kind": format!("{:?}", character.kind),
                "instance": format!("{instance:#x}"),
                "floats": floats(instance, CHARACTER_FLOATS),
                "ex_obj_hit": body.as_ref().map(|(offset, _)| format!("+{offset:#x}")),
                "body_records_f32": body.as_ref().map(|(_, raw)| raw.iter().map(|r| as_f32(r)).collect::<Vec<_>>()),
                "body_records_u32": body.as_ref().map(|(_, raw)| raw.iter().map(|r| as_u32(r)).collect::<Vec<_>>()),
            })
        })
        .collect();
    let others: Vec<Value> = crate::hooks::entities::other_classes()
        .into_iter()
        .map(|(class, count)| json!({ "class": class, "count": count }))
        .collect();
    json!({ "entity_capacity": entities, "characters": list, "other_classes": others })
}

/// The first `ENEMY_FLOATS` bytes of every tracked enemy as floats.
fn enemy_floats() -> Value {
    Value::Array(
        crate::hooks::enemy_hp::instances()
            .into_iter()
            .map(|(hash, instance)| {
                json!({
                    "type_hash": format!("{hash:#x}"),
                    "floats": floats(instance, ENEMY_FLOATS),
                    "ex_damage": crate::hooks::rtti::base_offset(instance, "ExDamage")
                        .map(|offset| format!("+{offset:#x}")),
                    "hurtboxes": crate::hooks::hurtboxes::of(instance)
                        .iter()
                        .map(|hurtbox| format!("{hurtbox:?}"))
                        .collect::<Vec<_>>(),

                    "break_parts_info": break_parts_info(instance),
                })
            })
            .collect(),
    )
}

/// An enemy's `ExEmBreakPartsInfo`: its offset, and the vector at `+0x10` (begin, end, capacity)
/// of plain part records, dumped raw, to work out the record layout.
fn break_parts_info(instance: usize) -> Value {
    const DUMP: usize = 0x400;
    let read_usize = |address: usize| crate::hooks::read_value::<usize>(address as *const usize);
    let Some(offset) = crate::hooks::rtti::base_offset(instance, "ExEmBreakPartsInfo") else {
        return Value::Null;
    };
    let info = instance + offset;
    let begin = read_usize(info + 0x10).unwrap_or(0);
    let end = read_usize(info + 0x18).unwrap_or(0);
    let capacity = read_usize(info + 0x20).unwrap_or(0);
    let bytes = end.saturating_sub(begin);
    let words: Vec<String> = (0..bytes.min(DUMP) / 4)
        .filter_map(|i| {
            crate::hooks::read_value::<u32>((begin + i * 4) as *const u32).map(|w| format!("{w:x}"))
        })
        .collect();
    json!({
        "offset": format!("+{offset:#x}"),
        "begin": format!("{begin:#x}"),
        "bytes": bytes,
        "capacity_bytes": capacity.saturating_sub(begin),
        "header_floats": floats(info, 0x40),
        "records_f32": floats(begin, bytes.min(DUMP)),
        "records_u32": words,
    })
}

/// `ExCollision` of every tracked enemy and party member: where it is, its element vector at
/// +0x90 and the first elements (pointees' classes and floats if they are pointers).
fn collisions() -> Value {
    const VECTOR: usize = 0x90;
    const ELEMENTS: usize = 12;
    const ELEMENT_FLOATS: usize = 0x100;
    let mut actors: Vec<(String, usize)> = crate::hooks::enemy_hp::instances()
        .into_iter()
        .map(|(hash, instance)| (format!("enemy {hash:#x}"), instance))
        .collect();
    actors.extend(
        crate::hooks::statuses::party_instances()
            .into_iter()
            .map(|(index, instance)| (format!("party {index}"), instance)),
    );
    let read_usize = |address: usize| crate::hooks::read_value::<usize>(address as *const usize);
    Value::Array(
        actors
            .into_iter()
            .map(|(who, instance)| {
                let class = read_usize(instance).and_then(crate::hooks::statuses::class_name_of_vtable);
                let Some(offset) = crate::hooks::rtti::base_offset(instance, "ExCollision") else {
                    return json!({ "actor": who, "class": class, "ex_collision": null });
                };
                let collision = instance + offset;
                let begin = read_usize(collision + VECTOR).unwrap_or(0);
                let end = read_usize(collision + VECTOR + 8).unwrap_or(0);
                let bytes = end.saturating_sub(begin);
                let words: Vec<usize> = (0..(bytes / 8).min(ELEMENTS * 8))
                    .filter_map(|i| read_usize(begin + i * 8))
                    .collect();
                let pointees: Vec<Value> = words
                    .iter()
                    .take(ELEMENTS)
                    .filter(|w| **w > 0x10000 && **w < 0x7FFF_FFFF_0000 && **w % 8 == 0)
                    .map(|pointer| {
                        json!({
                            "address": format!("{pointer:#x}"),
                            "class": read_usize(*pointer).and_then(crate::hooks::statuses::class_name_of_vtable),
                            "floats": floats(*pointer, ELEMENT_FLOATS),
                        })
                    })
                    .collect();
                json!({
                    "actor": who,
                    "class": class,
                    "ex_collision": format!("+{offset:#x}"),
                    "header_floats": floats(collision, 0xB0),
                    "vector_bytes": bytes,
                    "vector_words": words.iter().map(|w| format!("{w:#x}")).collect::<Vec<_>>(),
                    "vector_floats": floats(begin, bytes.min(0x400)),
                    "pointees": pointees,
                    "feet": floats(instance + 0x30, 12),
                })
            })
            .collect(),
    )
}

fn game_info() -> Value {
    let exe = std::env::current_exe().ok();
    let metadata = exe.as_ref().and_then(|path| std::fs::metadata(path).ok());
    let base = GAME_BASE.get().copied().unwrap_or(0);
    // IMAGE_NT_HEADERS: e_lfanew at +0x3C, then "PE\0\0" and IMAGE_FILE_HEADER.TimeDateStamp at +8.
    let pe_timestamp = (base != 0)
        .then(|| {
            let nt_offset = crate::hooks::read_value::<u32>((base + 0x3C) as *const u32)?;
            crate::hooks::read_value::<u32>((base + nt_offset as usize + 8) as *const u32)
        })
        .flatten();
    json!({
        "path": exe.map(|path| path.display().to_string()),
        "size": metadata.as_ref().map(|m| m.len()),
        "modified": metadata
            .and_then(|m| m.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
        "base_address": format!("{base:#x}"),
        "pe_timestamp": pe_timestamp,
    })
}
