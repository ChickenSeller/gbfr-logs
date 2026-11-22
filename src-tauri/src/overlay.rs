//! Forwards meter snapshots from the front-end to the in-game overlay.
//!
//! The meter window already resolves translated names, colours, sorting and streamer mode, so
//! it builds the display-ready [`OverlayState`] and invokes [`push_overlay_state`] on every
//! change. Pacing lives here rather than in the page: a hidden or minimised WebView throttles
//! its timers, while Tauri events (and so these invokes) still arrive. A background task sends
//! the newest snapshot at most every [`PUSH_INTERVAL`], and re-sends it every
//! [`RESEND_INTERVAL`] so a freshly injected hook gets data while nothing changes.
//!
//! The pipe is connected lazily: while the hook is not injected (or its overlay is disabled)
//! connection attempts are rate-limited and messages are dropped.
//!
//! Copy buttons in the overlay arrive as [`protocol::Message::OverlayCopyRequest`] on the
//! hook's main pipe; the meter window builds the text and reports back with
//! [`push_overlay_notice`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::SinkExt;
use interprocess::os::windows::named_pipe::{pipe_mode, tokio::SendPipeStream};
use protocol::{OverlayMessage, OverlayNotice, OverlayState};
use tokio::sync::Mutex;
use tokio_util::codec::{FramedWrite, LengthDelimitedCodec};

const PUSH_INTERVAL: Duration = Duration::from_millis(250);
const RESEND_INTERVAL: Duration = Duration::from_secs(2);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

type Writer = FramedWrite<SendPipeStream<pipe_mode::Bytes>, LengthDelimitedCodec>;

#[derive(Default)]
struct Connection {
    writer: Option<Writer>,
    last_attempt: Option<Instant>,
}

impl Connection {
    async fn send(&mut self, message: &OverlayMessage) {
        if self.writer.is_none() {
            if self
                .last_attempt
                .is_some_and(|attempt| attempt.elapsed() < RECONNECT_INTERVAL)
            {
                return;
            }
            self.last_attempt = Some(Instant::now());

            match SendPipeStream::<pipe_mode::Bytes>::connect_by_path(protocol::OVERLAY_PIPE_NAME)
                .await
            {
                Ok(stream) => {
                    self.writer = Some(FramedWrite::new(stream, LengthDelimitedCodec::new()))
                }
                // The hook is not injected yet, or its overlay is disabled.
                Err(_) => return,
            }
        }

        let Ok(bytes) = protocol::bincode::serialize(message) else {
            return;
        };
        if let Some(writer) = self.writer.as_mut() {
            if writer.send(bytes.into()).await.is_err() {
                // The game closed; reconnect on a later message.
                self.writer = None;
            }
        }
    }
}

#[derive(Default)]
struct Pending {
    state: Option<OverlayState>,
    changed: bool,
}

#[derive(Default, Clone)]
pub struct OverlayFeed {
    pending: Arc<std::sync::Mutex<Pending>>,
    connection: Arc<Mutex<Connection>>,
}

impl OverlayFeed {
    fn set_state(&self, state: OverlayState) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.state = Some(state);
        pending.changed = true;
    }

    /// Takes the snapshot to send now: a changed one, or the unchanged one when a resend is
    /// due.
    fn take_due(&self, resend_due: bool) -> Option<OverlayState> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if !pending.changed && !resend_due {
            return None;
        }
        pending.changed = false;
        pending.state.clone()
    }

    /// Paces snapshots to the overlay. Runs for the lifetime of the app.
    pub async fn run(self) {
        let mut ticker = tokio::time::interval(PUSH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_sent = Instant::now();

        loop {
            ticker.tick().await;
            let Some(state) = self.take_due(last_sent.elapsed() >= RESEND_INTERVAL) else {
                continue;
            };
            last_sent = Instant::now();
            self.connection
                .lock()
                .await
                .send(&OverlayMessage::State(state))
                .await;
        }
    }
}

impl OverlayFeed {
    /// Opens or closes the in-game menu, for keyboards without the configured hotkey.
    pub async fn toggle_menu(&self) {
        self.connection
            .lock()
            .await
            .send(&OverlayMessage::ToggleMenu)
            .await;
    }

    /// Shows or hides one of the in-game windows.
    pub async fn toggle_window(&self, window: protocol::OverlayWindow) {
        self.connection
            .lock()
            .await
            .send(&OverlayMessage::ToggleWindow(window))
            .await;
    }
}

#[tauri::command]
pub fn push_overlay_state(state: OverlayState, feed: tauri::State<'_, OverlayFeed>) {
    feed.set_state(state);
}

/// Shows a short status line in the overlay: saves, copy results and connection alerts.
#[tauri::command]
pub async fn push_overlay_notice(
    notice: OverlayNotice,
    feed: tauri::State<'_, OverlayFeed>,
) -> Result<(), ()> {
    feed.connection
        .lock()
        .await
        .send(&OverlayMessage::Notice(notice))
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_or_due_snapshots_are_taken() {
        let feed = OverlayFeed::default();
        assert!(feed.take_due(true).is_none(), "nothing to send yet");

        feed.set_state(OverlayState::default());
        assert!(feed.take_due(false).is_some(), "a new snapshot goes out");
        assert!(feed.take_due(false).is_none(), "unchanged snapshots wait");
        assert!(feed.take_due(true).is_some(), "but are re-sent when due");
    }
}
