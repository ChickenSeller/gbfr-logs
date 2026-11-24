//! Unloading the hook from a running game, asked for by the app when it quits, so the game
//! loses the overlay with it and the next app can inject a fresh hook.
//!
//! Freeing the DLL while any of its code can still run crashes the game, so before it goes:
//! - every game and Windows function hook is put back, so no new call lands in this DLL;
//! - the overlay's Present hook is removed and its window procedure restored;
//! - every thread this DLL started finishes (the tokio runtime, the pollers, the short-lived
//!   save and report threads);
//! - calls already inside a hook get time to return.
//!
//! If a thread does not finish in time (a font "Open" dialog left open), the DLL stays loaded
//! with its hooks off rather than risk freeing code that is still running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use log::{info, warn};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static WORKERS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// How long threads get to finish after the unload is asked for.
const WORKER_TIMEOUT: Duration = Duration::from_secs(5);
/// Time for calls already inside a hook (a frame, a damage event) to return.
const IN_FLIGHT_GRACE: Duration = Duration::from_millis(500);
const STEP: Duration = Duration::from_millis(100);

/// Whether the hook is being unloaded; long-running loops end when it is.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Sleeps for `duration`, waking early for an unload. `false` once the unload is asked for.
pub fn sleep(duration: Duration) -> bool {
    let until = Instant::now() + duration;
    while !requested() {
        let now = Instant::now();
        if now >= until {
            return true;
        }
        std::thread::sleep((until - now).min(STEP));
    }
    false
}

/// Resolves once the unload is asked for, for tasks on the hook's tokio runtime.
pub async fn wait() {
    while !requested() {
        tokio::time::sleep(STEP).await;
    }
}

/// Starts a thread the unload waits for. Every thread running this DLL's code goes through here.
pub fn spawn(f: impl FnOnce() + Send + 'static) {
    let handle = std::thread::spawn(f);
    let mut workers = WORKERS.lock().unwrap_or_else(|e| e.into_inner());
    workers.retain(|worker| !worker.is_finished());
    workers.push(handle);
}

/// Asks the hook to unload itself. Returns at once; the unload runs on a thread of its own.
pub fn request() {
    if REQUESTED.swap(true, Ordering::SeqCst) {
        return;
    }
    info!("Unload requested by the app");
    std::thread::spawn(unload);
}

fn unload() {
    crate::hooks::disable_detours();
    #[cfg(feature = "overlay")]
    crate::overlay::shutdown();

    let deadline = Instant::now() + WORKER_TIMEOUT;
    loop {
        let workers = WORKERS.lock().unwrap_or_else(|e| e.into_inner());
        if workers.iter().all(|worker| worker.is_finished()) {
            break;
        }
        drop(workers);
        if Instant::now() >= deadline {
            warn!("Threads still running after {WORKER_TIMEOUT:?}; staying loaded with hooks off");
            log::logger().flush();
            return;
        }
        std::thread::sleep(STEP);
    }
    for worker in std::mem::take(&mut *WORKERS.lock().unwrap_or_else(|e| e.into_inner())) {
        let _ = worker.join();
    }

    std::thread::sleep(IN_FLIGHT_GRACE);
    info!("Hook unloaded");
    log::logger().flush();
    free_self();
}

/// Drops the reference the app's injection holds, ending this thread; the loader then unmaps
/// the DLL. Never returns.
fn free_self() {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        FreeLibraryAndExitThread, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    let mut module = HMODULE::default();
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(free_self as *const u16),
            &mut module,
        )
    };
    if found.is_err() {
        warn!("Could not find the hook module; staying loaded with hooks off");
        return;
    }
    unsafe { FreeLibraryAndExitThread(module, 0) };
}
