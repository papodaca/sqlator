use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroize;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Key,
    Password,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshAuthConfigData {
    pub method: AuthMethod,
    pub username: String,
    pub key_path: Option<String>,
    pub has_passphrase: bool,
    pub has_password: bool,
}

#[derive(Debug, Clone)]
pub struct SshAuthConfig {
    pub method: AuthMethod,
    pub username: String,
    pub key_path: Option<PathBuf>,
    pub key_passphrase: Option<String>,
    pub password: Option<String>,
}

impl SshAuthConfig {
    pub fn with_key(username: impl Into<String>, key_path: impl Into<PathBuf>) -> Self {
        Self {
            method: AuthMethod::Key,
            username: username.into(),
            key_path: Some(key_path.into()),
            key_passphrase: None,
            password: None,
        }
    }

    pub fn with_key_and_passphrase(
        username: impl Into<String>,
        key_path: impl Into<PathBuf>,
        passphrase: impl Into<String>,
    ) -> Self {
        Self {
            method: AuthMethod::Key,
            username: username.into(),
            key_path: Some(key_path.into()),
            key_passphrase: Some(passphrase.into()),
            password: None,
        }
    }

    pub fn with_password(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            method: AuthMethod::Password,
            username: username.into(),
            key_path: None,
            key_passphrase: None,
            password: Some(password.into()),
        }
    }

    pub fn with_agent(username: impl Into<String>) -> Self {
        Self {
            method: AuthMethod::Agent,
            username: username.into(),
            key_path: None,
            key_passphrase: None,
            password: None,
        }
    }

    pub fn to_data(&self) -> SshAuthConfigData {
        SshAuthConfigData {
            method: self.method.clone(),
            username: self.username.clone(),
            key_path: self
                .key_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            has_passphrase: self.key_passphrase.is_some(),
            has_password: self.password.is_some(),
        }
    }
}

impl Drop for SshAuthConfig {
    fn drop(&mut self) {
        if let Some(ref mut passphrase) = self.key_passphrase {
            passphrase.zeroize();
        }
        if let Some(ref mut password) = self.password {
            password.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshHostConfig {
    pub host: String,
    pub port: u16,
    pub auth: SshAuthConfigData,
}

impl SshHostConfig {
    pub fn new(host: impl Into<String>, port: u16, auth: SshAuthConfig) -> Self {
        Self {
            host: host.into(),
            port,
            auth: auth.to_data(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpHost {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub key_path: Option<String>,
}
