//! Receives meter snapshots from the desktop app over [`protocol::OVERLAY_PIPE_NAME`].
//!
//! The pipe task stores the newest snapshot; the render thread copies it with `try_lock` and
//! keeps drawing its previous copy when the lock is busy, so it never waits on the pipe.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::StreamExt;
use interprocess::os::windows::named_pipe::tokio::PipeListenerOptionsExt;
use interprocess::os::windows::named_pipe::{PipeListenerOptions, PipeMode};
use log::{info, warn};
use protocol::{OverlayMessage, OverlayNotice, OverlayState, OverlayWindow};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

struct Latest {
    state: OverlayState,
    /// Bumped on every update so the render thread only clones changed snapshots.
    revision: u64,
}

static LATEST: Mutex<Option<Latest>> = Mutex::new(None);
static NOTICE: Mutex<Option<(OverlayNotice, Instant)>> = Mutex::new(None);

/// Snapshot held by the render thread.
pub struct Snapshot {
    pub state: OverlayState,
    revision: u64,
}

/// Starts the pipe server. Must be called from inside the hook's tokio runtime.
pub fn spawn() {
    tokio::spawn(async {
        let listener = match PipeListenerOptions::new()
            .path(protocol::OVERLAY_PIPE_NAME)
            .mode(PipeMode::Bytes)
            .accept_remote(false)
            .create_tokio_recv_only()
        {
            Ok(listener) => listener,
            Err(error) => {
                warn!("Could not create the overlay pipe listener: {error:?}");
                return;
            }
        };

        loop {
            match listener.accept().await {
                Ok(stream) => {
                    info!("Overlay feed connected");
                    tokio::spawn(async move {
                        let mut frames = FramedRead::new(stream, LengthDelimitedCodec::new());
                        while let Some(Ok(frame)) = frames.next().await {
                            match protocol::bincode::deserialize::<OverlayMessage>(&frame) {
                                Ok(OverlayMessage::State(state)) => store(state),
                                Ok(OverlayMessage::Notice(notice)) => store_notice(notice),
                                Ok(OverlayMessage::ToggleMenu) => {
                                    MENU_TOGGLE.store(true, Ordering::Relaxed)
                                }
                                Ok(OverlayMessage::ToggleWindow(window)) => {
                                    request_window_toggle(window)
                                }
                                Ok(OverlayMessage::DiagnosticsRequest) => {
                                    crate::diagnostics::request_report()
                                }
                                Ok(OverlayMessage::Unload) => crate::shutdown::request(),
                                Err(error) => warn!("Bad overlay message: {error}"),
                            }
                        }
                        info!("Overlay feed disconnected");
                    });
                }
                Err(error) => {
                    warn!("Error accepting overlay feed: {error:?}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
}

fn store(state: OverlayState) {
    let mut latest = LATEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let revision = latest.as_ref().map_or(1, |latest| latest.revision + 1);
    *latest = Some(Latest { state, revision });
}

/// Set by the app (tray menu) to open or close the in-game menu.
static MENU_TOGGLE: AtomicBool = AtomicBool::new(false);

/// Whether the app asked to toggle the menu since the last call.
pub fn take_menu_toggle() -> bool {
    MENU_TOGGLE.swap(false, Ordering::Relaxed)
}

/// Windows the app (tray menu) asked to show or hide, one bit per `OverlayWindow::ALL` entry.
static WINDOW_TOGGLES: AtomicU32 = AtomicU32::new(0);

fn window_bit(window: OverlayWindow) -> u32 {
    let index = OverlayWindow::ALL
        .iter()
        .position(|w| *w == window)
        .unwrap_or(0);
    1 << index
}

fn request_window_toggle(window: OverlayWindow) {
    // Two clicks before the next frame cancel out, like toggling twice.
    WINDOW_TOGGLES.fetch_xor(window_bit(window), Ordering::Relaxed);
}

/// The windows the app asked to toggle since the last call.
pub fn take_window_toggles() -> Vec<OverlayWindow> {
    let bits = WINDOW_TOGGLES.swap(0, Ordering::Relaxed);
    OverlayWindow::ALL
        .into_iter()
        .filter(|window| bits & window_bit(*window) != 0)
        .collect()
}

fn store_notice(notice: OverlayNotice) {
    *NOTICE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((notice, Instant::now()));
}

/// The newest notice if it arrived within `max_age`. Never blocks.
pub fn notice(max_age: Duration) -> Option<OverlayNotice> {
    let notice = NOTICE.try_lock().ok()?;
    let (notice, received) = notice.as_ref()?;
    (received.elapsed() <= max_age).then(|| notice.clone())
}

/// Refreshes `snapshot` from the newest state without blocking.
pub fn refresh(snapshot: &mut Option<Snapshot>) {
    let Ok(latest) = LATEST.try_lock() else {
        return;
    };
    let Some(latest) = latest.as_ref() else {
        return;
    };
    if snapshot
        .as_ref()
        .is_some_and(|current| current.revision == latest.revision)
    {
        return;
    }
    *snapshot = Some(Snapshot {
        state: latest.state.clone(),
        revision: latest.revision,
    });
}
