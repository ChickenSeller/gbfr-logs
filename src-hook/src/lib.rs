#![recursion_limit = "256"]

use std::io::Write;

use anyhow::{Context, Result};
use futures::sink::SinkExt;
use interprocess::os::windows::named_pipe::tokio::{PipeListenerOptionsExt, SendPipeStream};
use interprocess::os::windows::named_pipe::{pipe_mode, PipeListenerOptions, PipeMode};
use log::{info, warn};
use tokio::sync::broadcast;

mod diagnostics;
mod event;
mod hooks;
#[cfg(feature = "overlay")]
mod overlay;
mod process;
mod shutdown;

use protocol::Message;
use tokio_util::codec::{FramedWrite, LengthDelimitedCodec};

async fn handle_client(
    mut stream: FramedWrite<SendPipeStream<pipe_mode::Bytes>, LengthDelimitedCodec>,
    mut rx: event::Rx,
) -> Result<()> {
    let mut damage_events: u64 = 0;
    loop {
        let msg = match rx.recv().await {
            Ok(msg) => msg,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                warn!("Pipe client lagged, dropped {skipped} events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        if matches!(msg, Message::DamageEvent(_)) {
            damage_events += 1;
            if damage_events == 1 || damage_events % 1000 == 0 {
                info!("Forwarded {damage_events} damage events to the pipe client");
            }
        }
        let bytes = protocol::bincode::serialize(&msg)?;
        stream.send(bytes.into()).await?;
    }

    Ok(())
}

#[derive(Debug)]
struct Server {
    tx: event::Tx,
}

impl Server {
    fn new() -> Self {
        let (tx, _) = broadcast::channel::<Message>(1024);
        Server { tx }
    }

    async fn run(&self) {
        let listener = match PipeListenerOptions::new()
            .path(protocol::PIPE_NAME)
            .mode(PipeMode::Bytes)
            .accept_remote(false)
            .create_tokio_send_only()
        {
            Ok(listener) => listener,
            Err(error) => {
                warn!("Could not create the named pipe listener: {error:?}");
                return;
            }
        };
        info!("Named pipe listener ready");

        loop {
            let read_pipe = tokio::select! {
                read_pipe = listener.accept() => read_pipe,
                () = shutdown::wait() => break,
            };
            match read_pipe {
                Ok(stream) => {
                    info!("Pipe client connected");
                    let rx = self.tx.subscribe();
                    hooks::reset_client_identity_emissions();
                    tokio::spawn(async move {
                        let encoder = LengthDelimitedCodec::new();
                        let writer = FramedWrite::new(stream, encoder);

                        match handle_client(writer, rx).await {
                            Ok(()) => info!("Pipe client finished"),
                            Err(error) => info!("Pipe client disconnected: {error}"),
                        }
                    });
                }
                Err(e) => {
                    warn!("Error accepting client: {:?}", e);
                }
            }
        }
    }
}

/// Runs the hook until the app asks it to unload.
fn setup() {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            warn!("Could not start the hook runtime: {error}");
            return;
        }
    };
    runtime.block_on(run());
    // Pipe tasks are dropped here, which closes the app's connection.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
}

async fn run() {
    info!("Setting up named pipe listener");

    let server = Server::new();
    let tx = server.tx.clone();

    info!("Setting up hooks...");

    match hooks::setup_hooks(tx) {
        Ok(_) => info!("Hooks initialized"),
        Err(e) => warn!("Error initializing hooks: {:?}", e),
    }

    #[cfg(feature = "overlay")]
    overlay::setup(server.tx.clone());

    #[cfg(feature = "console")]
    println!("Hook library initialized");

    let _ = std::io::stdout().flush();

    server.run().await;
}

fn initialize_logger() -> anyhow::Result<()> {
    // The app migrates the old folder before injecting; this covers an older app.
    let _ = protocol::paths::migrate_legacy_data_dir();
    let data_dir = protocol::paths::data_dir().context("Could not find data folder")?;
    std::fs::create_dir_all(&data_dir)?;
    let log_file = data_dir.join(protocol::paths::HOOK_LOG_FILE);

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {}] {}",
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(fern::log_file(log_file)?)
        .apply()?;

    Ok(())
}

#[ctor::ctor]
fn entry() {
    #[cfg(feature = "console")]
    unsafe {
        let _ = windows::Win32::System::Console::AllocConsole();
    }

    let _ = initialize_logger();
    shutdown::spawn(setup);
}
