use std::sync::Arc;

use anyhow::Result;
use gpui::BackgroundExecutor;
use parking_lot::RwLock;

use super::session::SshSession;
use super::{SshConfig, SshHostKey};

/// Manages SSH sessions, allowing connection reuse across terminal, SFTP, and tunnels.
/// Uses a hub-and-spoke model where sessions are pooled by host.
pub struct SshSessionManager {
    sessions: RwLock<collections::HashMap<SshHostKey, Arc<SshSession>>>,
    executor: BackgroundExecutor,
}

impl SshSessionManager {
    pub fn new(executor: BackgroundExecutor) -> Self {
        Self {
            sessions: RwLock::new(collections::HashMap::default()),
            executor,
        }
    }

    /// Get an existing session or create a new one for the given configuration.
    /// If an existing connected session exists for this host, it will be reused.
    pub async fn get_or_create_session(&self, config: &SshConfig) -> Result<Arc<SshSession>> {
        let key = SshHostKey::from(config);

        if let Some(session) = self.sessions.read().get(&key) {
            if session.is_connected() {
                return Ok(session.clone());
            }
        }

        let session = SshSession::connect(config, self.executor.clone()).await?;
        self.sessions.write().insert(key, session.clone());
        Ok(session)
    }

    /// Remove a session from the manager.
    pub fn remove_session(&self, host_key: &SshHostKey) {
        self.sessions.write().remove(host_key);
    }

    /// Get an existing session if one exists and is connected.
    pub fn get_session(&self, host_key: &SshHostKey) -> Option<Arc<SshSession>> {
        self.sessions
            .read()
            .get(host_key)
            .filter(|s| s.is_connected())
            .cloned()
    }

    /// Close and remove all sessions.
    pub async fn close_all(&self) {
        let sessions: Vec<_> = self.sessions.write().drain().collect();
        for (_, session) in sessions {
            session.close().await;
        }
    }

    /// Get the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }
}
