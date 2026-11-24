//! Where the app and the hook keep their files: `%AppData%\gbfr-logs-kaguya`.
//!
//! The folder was `gbfr-logs-awa` before the Kaguya Edition rename; `migrate_legacy_data_dir`
//! moves it over once, so in-game settings and logs survive the upgrade. (The app's identifier,
//! and with it the WebView storage holding the meter settings, keeps its old name on purpose.)

use std::io;
use std::path::{Path, PathBuf};

pub const DATA_DIR_NAME: &str = "gbfr-logs-kaguya";
/// The hook's log file inside the data folder.
pub const HOOK_LOG_FILE: &str = "gbfr-logs-kaguya.txt";
/// Folder in the data folder where the hook records bosses' memory over a fight.
pub const BOSS_TRACE_DIR: &str = "boss-traces";
/// Folder in the data folder where the hook writes its diagnostics timeline, one file per session.
pub const TIMELINE_DIR: &str = "timelines";

const LEGACY_DATA_DIR_NAME: &str = "gbfr-logs-awa";
const LEGACY_HOOK_LOG_FILE: &str = "gbfr-logs-awa.txt";

fn app_data() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// `%AppData%\gbfr-logs-kaguya`.
pub fn data_dir() -> Option<PathBuf> {
    app_data().map(|dir| dir.join(DATA_DIR_NAME))
}

/// Moves `%AppData%\gbfr-logs-awa` to the new name if the new folder does not exist yet.
/// Returns whether anything was migrated.
pub fn migrate_legacy_data_dir() -> io::Result<bool> {
    match app_data() {
        Some(base) => migrate_in(&base),
        None => Ok(false),
    }
}

fn migrate_in(base: &Path) -> io::Result<bool> {
    let legacy = base.join(LEGACY_DATA_DIR_NAME);
    let current = base.join(DATA_DIR_NAME);
    if current.exists() || !legacy.is_dir() {
        return Ok(false);
    }

    // A rename is instant and atomic. It fails if an older hook still running in the game holds
    // its log file open; then the files are copied and the old folder is left for later.
    if std::fs::rename(&legacy, &current).is_err() {
        copy_dir(&legacy, &current)?;
        let _ = std::fs::remove_dir_all(&legacy);
    }

    let old_log = current.join(LEGACY_HOOK_LOG_FILE);
    let new_log = current.join(HOOK_LOG_FILE);
    if old_log.exists() && !new_log.exists() {
        let _ = std::fs::rename(old_log, new_log);
    }
    Ok(true)
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gbfr-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn moves_the_legacy_folder_and_renames_the_log() {
        let base = scratch("move");
        let legacy = base.join(LEGACY_DATA_DIR_NAME);
        std::fs::create_dir_all(legacy.join("diagnostics")).unwrap();
        std::fs::write(legacy.join("overlay.json"), "{}").unwrap();
        std::fs::write(legacy.join(LEGACY_HOOK_LOG_FILE), "log").unwrap();

        assert!(migrate_in(&base).unwrap());
        let current = base.join(DATA_DIR_NAME);
        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(current.join("overlay.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(current.join(HOOK_LOG_FILE)).unwrap(),
            "log"
        );
        assert!(current.join("diagnostics").is_dir());

        // Only once.
        assert!(!migrate_in(&base).unwrap());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn keeps_an_existing_new_folder() {
        let base = scratch("keep");
        std::fs::create_dir_all(base.join(LEGACY_DATA_DIR_NAME)).unwrap();
        std::fs::create_dir_all(base.join(DATA_DIR_NAME)).unwrap();
        std::fs::write(base.join(DATA_DIR_NAME).join("overlay.json"), "new").unwrap();

        assert!(!migrate_in(&base).unwrap());
        assert_eq!(
            std::fs::read_to_string(base.join(DATA_DIR_NAME).join("overlay.json")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn nothing_to_migrate() {
        let base = scratch("none");
        assert!(!migrate_in(&base).unwrap());
        assert!(!base.join(DATA_DIR_NAME).exists());
        let _ = std::fs::remove_dir_all(base);
    }
}
