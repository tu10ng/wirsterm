use std::{borrow::Cow, path::PathBuf, sync::Arc};

use alacritty_terminal::{
    event::{Notify, WindowSize},
    event_loop::{Msg, Notifier},
};
use anyhow::Result;
use parking_lot::RwLock;

use super::{ConnectionState, ProcessInfoProvider, TerminalConnection};
use crate::pty_info::PtyProcessInfo;

/// A terminal connection backed by a local pseudo-terminal (PTY).
/// This wraps the alacritty PTY event loop and notifier.
pub struct PtyConnection {
    notifier: Notifier,
    info: Arc<PtyProcessInfo>,
    state: RwLock<ConnectionState>,
}

impl PtyConnection {
    pub fn new(notifier: Notifier, info: Arc<PtyProcessInfo>) -> Self {
        Self {
            notifier,
            info,
            state: RwLock::new(ConnectionState::Connected),
        }
    }

    pub fn notifier(&self) -> &Notifier {
        &self.notifier
    }

    pub fn info(&self) -> &Arc<PtyProcessInfo> {
        &self.info
    }
}

impl TerminalConnection for PtyConnection {
    fn write(&self, data: Cow<'static, [u8]>) -> Result<()> {
        self.notifier.notify(data);
        Ok(())
    }

    fn resize(&self, size: WindowSize) -> Result<()> {
        self.notifier.0.send(Msg::Resize(size)).ok();
        Ok(())
    }

    fn shutdown(&self) -> Result<()> {
        self.notifier.0.send(Msg::Shutdown).ok();
        *self.state.write() = ConnectionState::Disconnected;
        Ok(())
    }

    fn state(&self) -> ConnectionState {
        self.state.read().clone()
    }

    fn process_info(&self) -> Option<Arc<dyn ProcessInfoProvider>> {
        Some(self.info.clone())
    }
}

impl ProcessInfoProvider for PtyProcessInfo {
    fn pid(&self) -> Option<sysinfo::Pid> {
        PtyProcessInfo::pid(self)
    }

    fn working_directory(&self) -> Option<PathBuf> {
        self.current.read().as_ref().map(|info| info.cwd.clone())
    }

    fn process_name(&self) -> Option<String> {
        self.current.read().as_ref().map(|info| info.name.clone())
    }

    fn kill_foreground_process(&self) -> bool {
        self.kill_current_process()
    }

    fn kill_child_process(&self) -> bool {
        PtyProcessInfo::kill_child_process(self)
    }
}
