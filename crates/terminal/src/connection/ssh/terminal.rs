use std::borrow::Cow;
use std::sync::{Arc, Weak};

use alacritty_terminal::event::{Event as AlacTermEvent, WindowSize};
use anyhow::Result;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::FutureExt;
use gpui::{BackgroundExecutor, Task};
use parking_lot::{Mutex, RwLock};

use super::session::{SshChannel, SshSession};
use super::SshConfig;
use crate::connection::{ConnectionState, ProcessInfoProvider, TerminalConnection};

/// Commands sent to the SSH channel task.
pub enum ChannelCommand {
    Write(Vec<u8>),
    Resize(WindowSize),
    Close,
}

/// A terminal connection over SSH.
/// Implements the TerminalConnection trait to allow transparent use
/// by the Terminal struct.
pub struct SshTerminalConnection {
    session: Weak<SshSession>,
    command_tx: UnboundedSender<ChannelCommand>,
    state: Arc<RwLock<ConnectionState>>,
    #[allow(dead_code)]
    channel_task: Mutex<Option<Task<()>>>,
    #[allow(dead_code)]
    initial_size: WindowSize,
    incoming_buffer: Arc<Mutex<Vec<u8>>>,
}

impl SshTerminalConnection {
    pub async fn new(
        session: Arc<SshSession>,
        config: &SshConfig,
        initial_size: WindowSize,
        event_tx: UnboundedSender<AlacTermEvent>,
        executor: BackgroundExecutor,
    ) -> Result<Self> {
        let state = Arc::new(RwLock::new(ConnectionState::Connecting));

        let channel = session
            .open_terminal_channel(initial_size, &config.env)
            .await?;

        let (command_tx, command_rx) = unbounded();

        *state.write() = ConnectionState::Connected;

        let incoming_buffer = Arc::new(Mutex::new(Vec::new()));

        let channel_task = spawn_channel_task(
            channel,
            command_rx,
            event_tx,
            state.clone(),
            config.initial_command.clone(),
            incoming_buffer.clone(),
            executor,
        );

        Ok(Self {
            session: Arc::downgrade(&session),
            command_tx,
            state,
            channel_task: Mutex::new(Some(channel_task)),
            initial_size,
            incoming_buffer,
        })
    }

    pub fn session(&self) -> Option<Arc<SshSession>> {
        self.session.upgrade()
    }
}

impl TerminalConnection for SshTerminalConnection {
    fn write(&self, data: Cow<'static, [u8]>) -> Result<()> {
        self.command_tx
            .unbounded_send(ChannelCommand::Write(data.into_owned()))
            .map_err(|_| anyhow::anyhow!("SSH channel closed"))
    }

    fn resize(&self, size: WindowSize) -> Result<()> {
        self.command_tx
            .unbounded_send(ChannelCommand::Resize(size))
            .map_err(|_| anyhow::anyhow!("SSH channel closed"))
    }

    fn shutdown(&self) -> Result<()> {
        *self.state.write() = ConnectionState::Disconnected;
        self.command_tx.unbounded_send(ChannelCommand::Close).ok();
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

impl Drop for SshTerminalConnection {
    fn drop(&mut self) {
        self.command_tx.unbounded_send(ChannelCommand::Close).ok();
    }
}

fn spawn_channel_task(
    mut channel: SshChannel,
    mut command_rx: UnboundedReceiver<ChannelCommand>,
    event_tx: UnboundedSender<AlacTermEvent>,
    state: Arc<RwLock<ConnectionState>>,
    initial_command: Option<String>,
    incoming_buffer: Arc<Mutex<Vec<u8>>>,
    executor: BackgroundExecutor,
) -> Task<()> {
    executor.spawn(async move {
        use futures::StreamExt;

        if let Some(command) = initial_command {
            let command_with_newline = format!("{}\n", command);
            if let Err(error) = channel.write(command_with_newline.as_bytes()).await {
                log::error!("Failed to send initial command: {}", error);
            }
        }

        loop {
            futures::select_biased! {
                command = command_rx.next() => {
                    match command {
                        Some(ChannelCommand::Write(data)) => {
                            if let Err(error) = channel.write(&data).await {
                                log::error!("Failed to write to SSH channel: {}", error);
                                *state.write() = ConnectionState::Error(error.to_string());
                                break;
                            }
                        }
                        Some(ChannelCommand::Resize(size)) => {
                            if let Err(error) = channel.resize(size).await {
                                log::warn!("Failed to resize SSH channel: {}", error);
                            }
                        }
                        Some(ChannelCommand::Close) | None => {
                            let _ = channel.close().await;
                            *state.write() = ConnectionState::Disconnected;
                            break;
                        }
                    }
                }
                data = channel.channel.wait().fuse() => {
                    match data {
                        Some(russh::ChannelMsg::Data { data }) => {
                            incoming_buffer.lock().extend_from_slice(&data);
                            event_tx.unbounded_send(AlacTermEvent::Wakeup).ok();
                        }
                        Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                            incoming_buffer.lock().extend_from_slice(&data);
                            event_tx.unbounded_send(AlacTermEvent::Wakeup).ok();
                        }
                        Some(russh::ChannelMsg::Eof) => {
                            *state.write() = ConnectionState::Disconnected;
                            event_tx.unbounded_send(AlacTermEvent::Exit).ok();
                            break;
                        }
                        Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                            log::debug!("SSH channel exit status: {}", exit_status);
                            event_tx.unbounded_send(AlacTermEvent::ChildExit(exit_status as i32)).ok();
                        }
                        Some(russh::ChannelMsg::Close) => {
                            *state.write() = ConnectionState::Disconnected;
                            event_tx.unbounded_send(AlacTermEvent::Exit).ok();
                            break;
                        }
                        None => {
                            *state.write() = ConnectionState::Disconnected;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    })
}
