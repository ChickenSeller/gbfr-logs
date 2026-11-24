//! Records how each boss's memory changes over a fight, to find part durability and break flags
//! without anyone timing an export: players just fight, and the diagnostics export bundles the
//! recordings.
//!
//! Every enemy with more than `BOSS_HP` max HP is traced once a second while it is tracked. A
//! trace watches regions of memory (the boss object and its hurtbox records), writes them whole
//! once, then only the u32 words that changed since the previous sample. Files go to
//! `<data dir>/boss-traces/`, newest `KEEP` kept.
//!
//! File format, little endian:
//! - `b"GBTR"`, u32 version (1), u32 type hash, u64 instance, u16 + bytes class name;
//! - u32 region count, then per region: u16 + bytes name, u64 address, u32 length, bytes;
//! - samples until the end: u32 milliseconds since the start, i64 HP, u32 change count, then
//!   per change: u8 region index, u32 byte offset, u32 new value.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{info, warn};

use super::read_process_value;

const BOSS_HP: i64 = 100_000_000;
const INTERVAL: Duration = Duration::from_secs(1);
/// The boss object, from its start.
const OBJECT_BYTES: usize = 0x10000;
/// Hurtbox records are 0x290 bytes each; at most this much of them.
const MAX_HURTBOX_BYTES: usize = 0x20000;
/// Traces kept on disk; older ones are deleted when a new one starts.
const KEEP: usize = 5;
/// A trace stops after this many samples (an hour), however long the fight.
const MAX_SAMPLES: u32 = 3600;
const CHUNK: usize = 4096;

struct Region {
    name: &'static str,
    address: usize,
    bytes: Vec<u8>,
}

struct Trace {
    instance: usize,
    started: Instant,
    regions: Vec<Region>,
    writer: BufWriter<File>,
    samples: u32,
}

static TRACES: Mutex<Vec<Trace>> = Mutex::new(Vec::new());

pub fn setup() {
    crate::shutdown::spawn(|| {
        while crate::shutdown::sleep(INTERVAL) {
            sample();
        }
        // Each trace's writer flushes as it is dropped.
        TRACES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    });
    info!("Boss tracing enabled");
}

fn directory() -> Option<PathBuf> {
    Some(protocol::paths::data_dir()?.join(protocol::paths::BOSS_TRACE_DIR))
}

/// `len` bytes at `address`, read in chunks; `None` if any chunk is unreadable.
fn read(address: usize, len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    let mut offset = 0;
    while offset + CHUNK <= len {
        out.extend_from_slice(&read_process_value::<[u8; CHUNK]>(
            (address + offset) as *const _,
        )?);
        offset += CHUNK;
    }
    while offset < len {
        out.push(read_process_value::<u8>((address + offset) as *const u8)?);
        offset += 1;
    }
    Some(out)
}

/// The regions watched for a boss: its object and its hurtbox records.
fn regions(instance: usize) -> Vec<(&'static str, usize, usize)> {
    let mut regions = vec![("object", instance, OBJECT_BYTES)];
    if let Some((records, bytes)) = super::hurtboxes::records_region(instance) {
        regions.push(("hurtboxes", records, bytes.min(MAX_HURTBOX_BYTES)));
    }
    regions
}

fn write_str(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    let bytes = &text.as_bytes()[..text.len().min(u16::MAX as usize)];
    out.write_all(&(bytes.len() as u16).to_le_bytes())?;
    out.write_all(bytes)
}

/// Deletes the oldest traces beyond `KEEP - 1`, to make room for a new one.
fn prune(directory: &PathBuf) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "gbtr"))
        .collect();
    // Names start with the start time, so name order is age order.
    files.sort();
    while files.len() >= KEEP {
        let _ = std::fs::remove_file(files.remove(0));
    }
}

fn start(instance: usize, type_hash: u32) -> Option<Trace> {
    let directory = directory()?;
    std::fs::create_dir_all(&directory).ok()?;
    prune(&directory);
    let class = super::rtti::class_name(instance).unwrap_or_default();
    let mut regions = Vec::new();
    for (name, address, len) in self::regions(instance) {
        regions.push(Region {
            name,
            address,
            bytes: read(address, len)?,
        });
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = directory.join(format!("{stamp}-{class}-{type_hash:08x}.gbtr"));
    let mut writer = BufWriter::new(File::create(&path).ok()?);
    let header = (|| -> std::io::Result<()> {
        writer.write_all(b"GBTR")?;
        writer.write_all(&1u32.to_le_bytes())?;
        writer.write_all(&type_hash.to_le_bytes())?;
        writer.write_all(&(instance as u64).to_le_bytes())?;
        write_str(&mut writer, &class)?;
        writer.write_all(&(regions.len() as u32).to_le_bytes())?;
        for region in &regions {
            write_str(&mut writer, region.name)?;
            writer.write_all(&(region.address as u64).to_le_bytes())?;
            writer.write_all(&(region.bytes.len() as u32).to_le_bytes())?;
            writer.write_all(&region.bytes)?;
        }
        writer.flush()
    })();
    if let Err(error) = header {
        warn!("Could not start a boss trace: {error}");
        return None;
    }
    info!(
        "Tracing boss {class} ({type_hash:#x}) to {}",
        path.display()
    );
    Some(Trace {
        instance,
        started: Instant::now(),
        regions,
        writer,
        samples: 0,
    })
}

/// Writes one sample; `false` once the boss can no longer be read.
fn record(trace: &mut Trace) -> bool {
    let Some(hp) = read_process_value::<i64>((trace.instance + 0x160) as *const i64) else {
        return false;
    };
    let mut changes: Vec<(u8, u32, u32)> = Vec::new();
    for (index, region) in trace.regions.iter_mut().enumerate() {
        let Some(now) = read(region.address, region.bytes.len()) else {
            continue;
        };
        for word in 0..now.len() / 4 {
            let at = word * 4;
            if now[at..at + 4] != region.bytes[at..at + 4] {
                let value = u32::from_le_bytes(now[at..at + 4].try_into().unwrap());
                changes.push((index as u8, at as u32, value));
            }
        }
        region.bytes = now;
    }
    let elapsed = trace.started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    let written = (|| -> std::io::Result<()> {
        trace.writer.write_all(&elapsed.to_le_bytes())?;
        trace.writer.write_all(&hp.to_le_bytes())?;
        trace
            .writer
            .write_all(&(changes.len() as u32).to_le_bytes())?;
        for (region, offset, value) in &changes {
            trace.writer.write_all(&[*region])?;
            trace.writer.write_all(&offset.to_le_bytes())?;
            trace.writer.write_all(&value.to_le_bytes())?;
        }
        trace.writer.flush()
    })();
    trace.samples += 1;
    written.is_ok() && trace.samples < MAX_SAMPLES
}

fn sample() {
    let mut traces = TRACES.lock().unwrap_or_else(|e| e.into_inner());
    let bosses: Vec<(u32, usize)> = super::enemy_hp::instances()
        .into_iter()
        .filter(|(_, instance)| {
            read_process_value::<i64>((instance + 0x168) as *const i64)
                .is_some_and(|max| max > BOSS_HP)
        })
        .collect();
    // Bosses no longer tracked (defeated a while ago, or gone) end their trace.
    traces.retain(|trace| bosses.iter().any(|(_, i)| *i == trace.instance));
    for (type_hash, instance) in bosses {
        if !traces.iter().any(|trace| trace.instance == instance) {
            if let Some(trace) = start(instance, type_hash) {
                traces.push(trace);
            }
        }
    }
    traces.retain_mut(record);
}
