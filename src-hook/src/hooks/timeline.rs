//! A timeline for the diagnostics export: what happened when, on one clock, to answer questions
//! static data cannot.
//!
//! - Party members: every action change, and every status applied or removed with the action
//!   the member was in at that moment (does a perfect dodge show as invincibility while dodging?).
//! - Hits enemies land on party members, with the class of the object that dealt them (a boss,
//!   or its shot).
//! - Enemies: fields of the biggest enemies that point at behaviour-tree objects (`*Action`,
//!   AI, behaviour classes), and how they change, to find where an enemy keeps the attack it is
//!   performing. Followed one level into AI and behaviour objects.
//!
//! Written as JSON lines to `<data dir>/timelines/<start>.jsonl`, one file per game session, the
//! newest `KEEP` kept, so the diagnostics export has them even after the game was closed. Each
//! line has `t` (milliseconds since the hook started) and `k` (`start`, `action`, `status+`,
//! `status-`, `hit`, `enemy`).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{info, warn};
use serde_json::{json, Value};

use super::read_process_value;

/// Timeline files kept; older ones are deleted when a session starts.
const KEEP: usize = 5;
/// A session's file stops growing here; enemy scans stop first, at `ENEMY_LIMIT`.
const FILE_LIMIT: u64 = 96 << 20;
const ENEMY_LIMIT: u64 = 64 << 20;
const ENEMY_INTERVAL: Duration = Duration::from_millis(250);
/// Enemies scanned, the biggest by max HP.
const ENEMIES_SCANNED: usize = 3;
const ENEMY_BYTES: usize = 0x10000;
/// How far into an AI or behaviour object the second level looks, and how many are followed.
const NESTED_BYTES: usize = 0x800;
const NESTED_PER_ENEMY: usize = 8;
const CHUNK: usize = 4096;
/// Vtables lie in the game image: this far from its base at most.
const IMAGE_SPAN: usize = 0x0A00_0000;

static START: OnceLock<Instant> = OnceLock::new();
static GAME_BASE: OnceLock<usize> = OnceLock::new();

struct Writer {
    file: BufWriter<File>,
    path: PathBuf,
    bytes: u64,
}

static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

/// Milliseconds on the timeline's clock.
pub fn ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Appends one line, with its kind and time. Enemy scans stop first when the file grows big.
pub fn record(kind: &str, mut entry: Value) {
    entry["k"] = json!(kind);
    entry["t"] = json!(ms());
    let mut writer = WRITER.lock().unwrap_or_else(|e| e.into_inner());
    let Some(writer) = writer.as_mut() else {
        return;
    };
    let limit = if kind == "enemy" {
        ENEMY_LIMIT
    } else {
        FILE_LIMIT
    };
    if writer.bytes >= limit {
        return;
    }
    let mut line = entry.to_string();
    line.push('\n');
    if writer.file.write_all(line.as_bytes()).is_ok() {
        writer.bytes += line.len() as u64;
    }
}

fn directory() -> Option<PathBuf> {
    Some(protocol::paths::data_dir()?.join(protocol::paths::TIMELINE_DIR))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn open() -> Option<Writer> {
    let directory = directory()?;
    std::fs::create_dir_all(&directory).ok()?;
    if let Ok(entries) = std::fs::read_dir(&directory) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        // Names start with the start time, so name order is age order.
        files.sort();
        while files.len() >= KEEP {
            let _ = std::fs::remove_file(files.remove(0));
        }
    }
    let path = directory.join(format!("{}.jsonl", unix_now()));
    let file = File::create(&path).ok()?;
    Some(Writer {
        file: BufWriter::new(file),
        path,
        bytes: 0,
    })
}

fn flush() {
    if let Some(writer) = WRITER.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writer.file.flush();
    }
}

pub fn setup(game_base: usize) {
    let _ = GAME_BASE.set(game_base);
    let _ = START.set(Instant::now());
    match open() {
        Some(writer) => {
            info!("Diagnostics timeline: {}", writer.path.display());
            *WRITER.lock().unwrap_or_else(|e| e.into_inner()) = Some(writer);
        }
        None => warn!("Could not open the diagnostics timeline"),
    }
    record("start", json!({ "unix": unix_now() }));
    crate::shutdown::spawn(|| {
        let mut previous: HashMap<usize, HashMap<String, String>> = HashMap::new();
        while crate::shutdown::sleep(ENEMY_INTERVAL) {
            scan_enemies(&mut previous);
            flush();
        }
        flush();
    });
}

/// Where the timeline is and how far it got, for the report.
pub fn report() -> Value {
    let writer = WRITER.lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "now": ms(),
        "file": writer.as_ref().map(|w| w.path.display().to_string()),
        "bytes": writer.as_ref().map(|w| w.bytes),
    })
}

fn read(address: usize, len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    let mut offset = 0;
    while offset < len {
        out.extend_from_slice(&read_process_value::<[u8; CHUNK]>(
            (address + offset) as *const _,
        )?);
        offset += CHUNK;
    }
    out.truncate(len);
    Some(out)
}

/// The class of the object at `pointer`, if it is one (its vtable lies in the game image).
fn object_class(pointer: usize) -> Option<String> {
    if pointer < 0x10000 || pointer > 0x7FFF_FFFF_FFF0 || pointer % 8 != 0 {
        return None;
    }
    let base = *GAME_BASE.get()?;
    let vtable = read_process_value::<usize>(pointer as *const usize)?;
    if vtable < base || vtable >= base + IMAGE_SPAN {
        return None;
    }
    super::statuses::class_name_of_vtable(vtable)
}

fn is_behaviour(class: &str) -> bool {
    class.contains("Action")
        || class.contains("@BT")
        || class.contains("Behavior")
        || class.contains("AI")
        || class.contains("Ai")
}

fn follows(class: &str) -> bool {
    !class.contains("Action")
        && (class.contains("Behavior") || class.contains("AI") || class.contains("Ai"))
}

/// Pointer fields in `[address, address + len)` aimed at behaviour objects: offset -> (class,
/// pointer).
fn behaviour_fields(address: usize, len: usize) -> Vec<(usize, String, usize)> {
    let Some(bytes) = read(address, len) else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    for offset in (0..len).step_by(8) {
        let pointer = usize::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if let Some(class) = object_class(pointer) {
            if is_behaviour(&class) {
                fields.push((offset, class, pointer));
            }
        }
    }
    fields
}

fn scan_enemies(previous: &mut HashMap<usize, HashMap<String, String>>) {
    let mut enemies: Vec<(u32, usize, i64)> = super::enemy_hp::instances()
        .into_iter()
        .filter_map(|(hash, instance)| {
            let max = read_process_value::<i64>((instance + 0x168) as *const i64)?;
            Some((hash, instance, max))
        })
        .collect();
    enemies.sort_by_key(|(_, _, max)| std::cmp::Reverse(*max));
    enemies.truncate(ENEMIES_SCANNED);
    previous.retain(|instance, _| enemies.iter().any(|(_, i, _)| i == instance));

    for (hash, instance, _) in enemies {
        let mut now: HashMap<String, String> = HashMap::new();
        let mut followed = 0;
        for (offset, class, pointer) in behaviour_fields(instance, ENEMY_BYTES) {
            if follows(&class) && followed < NESTED_PER_ENEMY {
                followed += 1;
                for (inner, inner_class, _) in behaviour_fields(pointer, NESTED_BYTES) {
                    now.insert(format!("{offset:#x}>{inner:#x}"), inner_class);
                }
            }
            now.insert(format!("{offset:#x}"), class);
        }
        let before = previous.entry(instance).or_default();
        let mut changes = Vec::new();
        for (field, class) in &now {
            if before.get(field) != Some(class) {
                changes.push(json!([field, class]));
            }
        }
        for field in before.keys() {
            if !now.contains_key(field) {
                changes.push(json!([field, Value::Null]));
            }
        }
        if !changes.is_empty() {
            record(
                "enemy",
                json!({
                    "enemy": format!("{hash:08x}"),
                    "instance": format!("{instance:#x}"),
                    "hp": read_process_value::<i64>((instance + 0x160) as *const i64),
                    "changes": changes,
                }),
            );
        }
        *before = now;
    }
}
