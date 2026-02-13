use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacTermEvent, WindowSize};
use anyhow::Result;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::FutureExt;
use parking_lot::{Mutex, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::JoinHandle;

use super::protocol::{TelnetNegotiator, escape_data_for_send};
use super::session::TelnetSession;
use super::TelnetConfig;
use crate::connection::{ConnectionState, ProcessInfoProvider, TerminalConnection};

pub enum TelnetChannelCommand {
    Write(Vec<u8>),
    Resize(WindowSize),
    Close,
}

pub struct TelnetTerminalConnection {
    command_tx: UnboundedSender<TelnetChannelCommand>,
    state: Arc<RwLock<ConnectionState>>,
    #[allow(dead_code)]
    channel_task: Mutex<Option<JoinHandle<()>>>,
    #[allow(dead_code)]
    initial_size: WindowSize,
    incoming_buffer: Arc<Mutex<Vec<u8>>>,
}

impl TelnetTerminalConnection {
    pub async fn new(
        session: TelnetSession,
        read_half: OwnedReadHalf,
        write_half: OwnedWriteHalf,
        config: &TelnetConfig,
        initial_size: WindowSize,
        event_tx: UnboundedSender<AlacTermEvent>,
        tokio_handle: tokio::runtime::Handle,
    ) -> Result<Self> {
        let state = Arc::new(RwLock::new(session.state()));

        let (command_tx, command_rx) = unbounded();

        let incoming_buffer = Arc::new(Mutex::new(Vec::new()));

        let channel_task = spawn_channel_task(
            read_half,
            write_half,
            command_rx,
            event_tx,
            state.clone(),
            config.terminal_type.clone(),
            initial_size,
            incoming_buffer.clone(),
            tokio_handle,
        );

        Ok(Self {
            command_tx,
            state,
            channel_task: Mutex::new(Some(channel_task)),
            initial_size,
            incoming_buffer,
        })
    }
}

impl TerminalConnection for TelnetTerminalConnection {
    fn write(&self, data: Cow<'static, [u8]>) -> Result<()> {
        self.command_tx
            .unbounded_send(TelnetChannelCommand::Write(data.into_owned()))
            .map_err(|_| anyhow::anyhow!("Telnet channel closed"))
    }

    fn resize(&self, size: WindowSize) -> Result<()> {
        self.command_tx
            .unbounded_send(TelnetChannelCommand::Resize(size))
            .map_err(|_| anyhow::anyhow!("Telnet channel closed"))
    }

    fn shutdown(&self) -> Result<()> {
        *self.state.write() = ConnectionState::Disconnected;
        self.command_tx
            .unbounded_send(TelnetChannelCommand::Close)
            .ok();
        Ok(())
    }

    fn state(&self) -> ConnectionState {
        self.state.read().clone()
    }

    fn process_info(&self) -> Option<Arc<dyn ProcessInfoProvider>> {
        None
    }

    fn read(&self) -> Option<Vec<u8>> {
        let mut buffer = self.incoming_buffer.lock();
        if buffer.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut *buffer))
        }
    }
}

impl Drop for TelnetTerminalConnection {
    fn drop(&mut self) {
        self.command_tx
            .unbounded_send(TelnetChannelCommand::Close)
            .ok();
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_channel_task(
    mut read_half: OwnedReadHalf,
    mut write_half: OwnedWriteHalf,
    mut command_rx: UnboundedReceiver<TelnetChannelCommand>,
    event_tx: UnboundedSender<AlacTermEvent>,
    state: Arc<RwLock<ConnectionState>>,
    terminal_type: String,
    initial_size: WindowSize,
    incoming_buffer: Arc<Mutex<Vec<u8>>>,
    tokio_handle: tokio::runtime::Handle,
) -> JoinHandle<()> {
    tokio_handle.spawn(async move {
        use futures::StreamExt;

        let mut negotiator = TelnetNegotiator::new(terminal_type);
        let mut read_buf = [0u8; 4096];
        let mut sent_initial_naws = false;

        loop {
            futures::select_biased! {
                command = command_rx.next() => {
                    match command {
                        Some(TelnetChannelCommand::Write(data)) => {
                            let escaped = escape_data_for_send(&data);
                            if let Err(error) = write_half.write_all(&escaped).await {
                                log::error!("Failed to write to Telnet connection: {}", error);
                                *state.write() = ConnectionState::Error(error.to_string());
                                break;
                            }
                        }
                        Some(TelnetChannelCommand::Resize(size)) => {
                            let naws_packet = negotiator.build_naws(size);
                            if !naws_packet.is_empty() {
                                if let Err(error) = write_half.write_all(&naws_packet).await {
                                    log::warn!("Failed to send NAWS: {}", error);
                                }
                            }
                        }
                        Some(TelnetChannelCommand::Close) | None => {
                            *state.write() = ConnectionState::Disconnected;
                            break;
                        }
                    }
                }
                result = read_half.read(&mut read_buf).fuse() => {
                    match result {
                        Ok(0) => {
                            // Connection closed
                            *state.write() = ConnectionState::Disconnected;
                            event_tx.unbounded_send(AlacTermEvent::Exit).ok();
                            break;
                        }
                        Ok(n) => {
                            let process_result = negotiator.process_incoming(&read_buf[..n]);

                            // Send any protocol responses
                            if !process_result.responses.is_empty() {
                                if let Err(error) = write_half.write_all(&process_result.responses).await {
                                    log::error!("Failed to send Telnet responses: {}", error);
                                    *state.write() = ConnectionState::Error(error.to_string());
                                    break;
                                }

                                // After NAWS is enabled, send initial window size
                                if !sent_initial_naws && negotiator.is_naws_enabled() {
                                    let naws_packet = negotiator.build_naws(initial_size);
                                    if !naws_packet.is_empty() {
                                        if let Err(error) = write_half.write_all(&naws_packet).await {
                                            log::warn!("Failed to send initial NAWS: {}", error);
                                        }
                                        sent_initial_naws = true;
                                    }
                                }
                            }

                            // Buffer terminal data
                            if !process_result.data.is_empty() {
                                incoming_buffer.lock().extend_from_slice(&process_result.data);
                                event_tx.unbounded_send(AlacTermEvent::Wakeup).ok();
                            }
                        }
                        Err(error) => {
                            log::error!("Telnet read error: {}", error);
                            *state.write() = ConnectionState::Error(error.to_string());
                            event_tx.unbounded_send(AlacTermEvent::Exit).ok();
                            break;
                        }
                    }
                }
            }
        }
    })
}
