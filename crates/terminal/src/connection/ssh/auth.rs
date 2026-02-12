use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use russh::client::{AuthResult, Handle};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};

/// SSH authentication configuration.
#[derive(Clone, Debug)]
pub enum SshAuthConfig {
    /// Use password authentication.
    Password(String),
    /// Use private key file for authentication.
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
    /// Try authentication methods in order: keys -> password prompt.
    Auto,
}

/// Result of an authentication attempt.
#[derive(Debug)]
pub enum SshAuthMethod {
    PrivateKey(PathBuf),
    Password,
    None,
}

/// Authenticate an SSH session with the given configuration.
pub async fn authenticate<H: russh::client::Handler>(
    session: &mut Handle<H>,
    username: &str,
    config: &SshAuthConfig,
) -> Result<SshAuthMethod> {
    match config {
        SshAuthConfig::Password(password) => {
            authenticate_with_password(session, username, password).await?;
            Ok(SshAuthMethod::Password)
        }
        SshAuthConfig::PrivateKey { path, passphrase } => {
            authenticate_with_key(session, username, path, passphrase.as_deref()).await?;
            Ok(SshAuthMethod::PrivateKey(path.clone()))
        }
        SshAuthConfig::Auto => authenticate_auto(session, username).await,
    }
}

async fn authenticate_with_password<H: russh::client::Handler>(
    session: &mut Handle<H>,
    username: &str,
    password: &str,
) -> Result<()> {
    let result = session
        .authenticate_password(username, password)
        .await
        .context("password authentication failed")?;

    check_auth_result(result, "password")
}

async fn authenticate_with_key<H: russh::client::Handler>(
    session: &mut Handle<H>,
    username: &str,
    key_path: &PathBuf,
    passphrase: Option<&str>,
) -> Result<()> {
    let key_pair: PrivateKey = russh::keys::load_secret_key(key_path, passphrase)
        .context("failed to load private key")?;

    let key_with_hash = PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);

    let result = session
        .authenticate_publickey(username, key_with_hash)
        .await
        .context("public key authentication failed")?;

    check_auth_result(result, "public key")
}

fn check_auth_result(result: AuthResult, method_name: &str) -> Result<()> {
    match result {
        AuthResult::Success => Ok(()),
        AuthResult::Failure {
            remaining_methods,
            partial_success,
        } => {
            if partial_success {
                anyhow::bail!(
                    "{} authentication partial success, remaining methods: {:?}",
                    method_name,
                    remaining_methods
                )
            } else {
                anyhow::bail!(
                    "{} authentication rejected, remaining methods: {:?}",
                    method_name,
                    remaining_methods
                )
            }
        }
    }
}

async fn authenticate_auto<H: russh::client::Handler>(
    session: &mut Handle<H>,
    username: &str,
) -> Result<SshAuthMethod> {
    for key_path in find_default_ssh_keys() {
        if let Ok(()) = authenticate_with_key(session, username, &key_path, None).await {
            return Ok(SshAuthMethod::PrivateKey(key_path));
        }
    }

    anyhow::bail!("auto authentication failed: no default key files could authenticate")
}

fn find_default_ssh_keys() -> Vec<PathBuf> {
    let mut keys = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let ssh_dir = home.join(".ssh");
        for key_name in ["id_ed25519", "id_rsa", "id_ecdsa", "id_dsa"] {
            let key_path = ssh_dir.join(key_name);
            if key_path.exists() {
                keys.push(key_path);
            }
        }
    }
    keys
}
