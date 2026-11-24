//! "Export diagnostics" from the tray: everything needed to investigate a problem, in one zip
//! file that the player sends by hand.
//!
//! Contents: the hook's log, the app's logs, environment details, the in-game settings
//! (overlay.json), the latest saved encounters exactly as stored, and a runtime probe from the
//! hook (setup results, party and enemy objects, active statuses) if the game is running.
//! Nothing is removed or anonymised: the player decides what to send.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;
use zip::{write::FileOptions, CompressionMethod, ZipWriter};

use crate::overlay::OverlayFeed;

/// How long to wait for the hook's runtime probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Only the end of a log is kept; it holds the latest sessions.
const HOOK_LOG_TAIL: u64 = 8 * 1024 * 1024;
const APP_LOG_TAIL: u64 = 2 * 1024 * 1024;
const RECENT_ENCOUNTERS: u32 = 5;

/// Waits for `Message::DiagnosticsReport` from the hook.
static PENDING_PROBE: Mutex<Option<oneshot::Sender<String>>> = Mutex::new(None);

/// Hands the hook's runtime probe to a running export.
pub fn deliver_probe(report: String) {
    if let Some(sender) = PENDING_PROBE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        let _ = sender.send(report);
    }
}

/// `%AppData%\gbfr-logs-kaguya`, shared with the hook.
fn data_dir() -> Option<PathBuf> {
    protocol::paths::data_dir()
}

/// Writes the diagnostics zip and returns its path.
pub async fn export(app: AppHandle) -> Result<PathBuf> {
    let (sender, receiver) = oneshot::channel();
    *PENDING_PROBE.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
    app.state::<OverlayFeed>()
        .inner()
        .clone()
        .request_diagnostics()
        .await;
    let probe = match tokio::time::timeout(PROBE_TIMEOUT, receiver).await {
        Ok(Ok(report)) => {
            serde_json::from_str(&report).unwrap_or_else(|_| json!({ "unparsed": report }))
        }
        _ => json!({ "error": "No answer from the hook: the game is not running or not hooked." }),
    };
    PENDING_PROBE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();

    let app_version = app.package_info().version.to_string();
    tokio::task::spawn_blocking(move || write_zip(&app_version, &probe))
        .await
        .context("diagnostics export task failed")?
}

fn write_zip(app_version: &str, probe: &serde_json::Value) -> Result<PathBuf> {
    let dir = data_dir()
        .context("no %APPDATA% folder")?
        .join("diagnostics");
    std::fs::create_dir_all(&dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("gbfr-logs-diagnostics-{stamp}.zip"));

    let mut zip = ZipWriter::new(std::fs::File::create(&path)?);
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut add = |name: &str, bytes: &[u8]| -> Result<()> {
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
        Ok(())
    };

    let environment = json!({
        "app_version": app_version,
        "exported_at": chrono::Local::now().to_rfc3339(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "windows_version": windows_version(),
        "working_directory": std::env::current_dir().ok().map(|dir| dir.display().to_string()),
    });
    add(
        "environment.json",
        &serde_json::to_vec_pretty(&environment)?,
    )?;
    add("runtime-probe.json", &serde_json::to_vec_pretty(probe)?)?;

    if let Some(data) = data_dir() {
        if let Some(log) = tail(&data.join(protocol::paths::HOOK_LOG_FILE), HOOK_LOG_TAIL) {
            add("hook-log.txt", &log)?;
        }
        if let Ok(settings) = std::fs::read(data.join("overlay.json")) {
            add("overlay.json", &settings)?;
        }
        // The hook's recordings of bosses' memory over recent fights.
        if let Ok(entries) = std::fs::read_dir(data.join(protocol::paths::BOSS_TRACE_DIR)) {
            for entry in entries.flatten() {
                if let Ok(trace) = std::fs::read(entry.path()) {
                    add(
                        &format!("boss-traces/{}", entry.file_name().to_string_lossy()),
                        &trace,
                    )?;
                }
            }
        }
    }
    // The app's own logs (tauri-plugin-log writes next to the working directory).
    if let Ok(entries) = std::fs::read_dir("logs") {
        for entry in entries.flatten() {
            if let Some(log) = tail(&entry.path(), APP_LOG_TAIL) {
                add(
                    &format!("app-logs/{}", entry.file_name().to_string_lossy()),
                    &log,
                )?;
            }
        }
    }

    let (index, blobs) = recent_encounters()
        .unwrap_or_else(|error| (json!({ "error": error.to_string() }), Vec::new()));
    add("encounters/index.json", &serde_json::to_vec_pretty(&index)?)?;
    for (name, blob) in blobs {
        add(&format!("encounters/{name}"), &blob)?;
    }

    zip.finish()?;
    Ok(path)
}

/// The latest saved encounters, exactly as stored (compressed blob plus its format version), so
/// they can be reparsed with the app's own code.
fn recent_encounters() -> Result<(serde_json::Value, Vec<(String, Vec<u8>)>)> {
    let conn = crate::db::connect_to_db()?;
    let mut statement = conn.prepare(
        "SELECT id, name, time, duration, version, quest_id, data FROM logs ORDER BY id DESC LIMIT ?",
    )?;
    let rows = statement.query_map([RECENT_ENCOUNTERS], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, u8>(4)?,
            row.get::<_, Option<u32>>(5)?,
            row.get::<_, Vec<u8>>(6)?,
        ))
    })?;

    let mut index = Vec::new();
    let mut blobs = Vec::new();
    for row in rows {
        let (id, name, time, duration, version, quest_id, data) = row?;
        let file = format!("{id}-v{version}.bin");
        index.push(json!({
            "id": id,
            "name": name,
            "time": time,
            "duration": duration,
            "version": version,
            "quest_id": quest_id,
            "file": file,
            "bytes": data.len(),
        }));
        blobs.push((file, data));
    }
    Ok((json!(index), blobs))
}

/// The last `limit` bytes of a file.
fn tail(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(limit)))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// `ver` output, e.g. "Microsoft Windows [Version 10.0.26300.1]".
fn windows_version() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("cmd")
        .args(["/c", "ver"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Shows the file in Explorer, ready to be sent.
pub fn reveal(path: &Path) {
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}
