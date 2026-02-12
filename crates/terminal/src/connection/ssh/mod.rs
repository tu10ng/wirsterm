mod auth;
mod manager;
mod session;
mod terminal;

pub use auth::{SshAuthConfig, SshAuthMethod};
pub use manager::SshSessionManager;
pub use session::SshSession;
pub use terminal::SshTerminalConnection;

use std::hash::Hash;

/// Configuration for an SSH connection.
#[derive(Clone, Debug)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub auth: SshAuthConfig,
    pub env: collections::HashMap<String, String>,
    pub keepalive_interval: Option<std::time::Duration>,
    pub initial_command: Option<String>,
}

impl SshConfig {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            username: None,
            auth: SshAuthConfig::Auto,
            env: collections::HashMap::default(),
            keepalive_interval: Some(std::time::Duration::from_secs(30)),
            initial_command: None,
        }
    }

    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn with_auth(mut self, auth: SshAuthConfig) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_env(mut self, env: collections::HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    pub fn with_keepalive(mut self, interval: std::time::Duration) -> Self {
        self.keepalive_interval = Some(interval);
        self
    }

    pub fn with_initial_command(mut self, command: impl Into<String>) -> Self {
        self.initial_command = Some(command.into());
        self
    }
}

/// Identifies a unique SSH host for session reuse.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SshHostKey {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
}

impl From<&SshConfig> for SshHostKey {
    fn from(config: &SshConfig) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            username: config.username.clone(),
        }
    }
}
