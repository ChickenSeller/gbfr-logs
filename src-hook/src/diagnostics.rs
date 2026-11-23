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
        "party_floats": crate::hooks::statuses::instance_floats(PARTY_FLOATS),
    })
}

/// Global `cCameraApp` objects (RVAs of the six statics destroyed at exit, game 2.0.2).
const CAMERA_RVAS: [usize; 6] = [
    0x7C2_4860, 0x701_E1D0, 0x701_E600, 0x701_E9F0, 0x701_EDE0, 0x701_F1A0,
];
const CAMERA_SIZE: usize = 0x440;
/// Bytes of each party member instance dumped as floats, to find world positions.
const PARTY_FLOATS: usize = 0x800;

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
