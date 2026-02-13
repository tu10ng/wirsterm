mod protocol;
mod session;
mod terminal;

pub use protocol::{TelnetNegotiator, escape_data_for_send};
pub use session::TelnetSession;
pub use terminal::TelnetTerminalConnection;

#[derive(Clone, Debug)]
pub struct TelnetConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub encoding: Option<String>,
    pub terminal_type: String,
}

impl TelnetConfig {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            username: None,
            password: None,
            encoding: None,
            terminal_type: "xterm-256color".to_string(),
        }
    }

    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = Some(encoding.into());
        self
    }

    pub fn with_terminal_type(mut self, terminal_type: impl Into<String>) -> Self {
        self.terminal_type = terminal_type.into();
        self
    }
}
