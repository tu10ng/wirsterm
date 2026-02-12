mod pty;
pub mod ssh;

use std::{borrow::Cow, path::PathBuf, sync::Arc};

use alacritty_terminal::event::WindowSize;
use anyhow::Result;

pub use pty::PtyConnection;

/// State of a terminal connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}

impl ConnectionState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionState::Connected)
    }
}

/// Trait for providing process information from a terminal connection.
/// Implemented by local PTY connections. SSH connections return None for most methods
/// since we cannot query remote process state.
pub trait ProcessInfoProvider: Send + Sync {
    fn pid(&self) -> Option<sysinfo::Pid>;
    fn working_directory(&self) -> Option<PathBuf>;
    fn process_name(&self) -> Option<String>;
    fn kill_foreground_process(&self) -> bool;
    fn kill_child_process(&self) -> bool;
}

/// Abstract trait for terminal connections.
/// This allows the terminal to work with different connection backends
/// (local PTY, SSH, and future Telnet) without knowing the implementation details.
pub trait TerminalConnection: Send + Sync {
    /// Write data to the connection.
    fn write(&self, data: Cow<'static, [u8]>) -> Result<()>;

    /// Resize the terminal.
    fn resize(&self, size: WindowSize) -> Result<()>;

    /// Shutdown the connection gracefully.
    fn shutdown(&self) -> Result<()>;

    /// Get the current connection state.
    fn state(&self) -> ConnectionState;

    /// Get process info provider if available.
    /// Returns None for connections that don't support process info (like SSH).
    fn process_info(&self) -> Option<Arc<dyn ProcessInfoProvider>> {
        None
    }

    /// Read pending data from the connection.
    /// Returns None if no data is available or if this connection
    /// doesn't buffer incoming data (e.g., PTY handles this internally).
    fn read(&self) -> Option<Vec<u8>> {
        None
    }
}
