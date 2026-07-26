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

/// SSH authentication material. Secrets are zeroized on [`Drop`].
///
/// Intentionally does **not** implement [`Clone`] so passwords/passphrases
/// cannot be accidentally retained via copies (sqlator-33j).
///
/// ```compile_fail
/// use sqlator_core::SshAuthConfig;
/// fn assert_clone<T: Clone>() {}
/// assert_clone::<SshAuthConfig>();
/// ```
#[derive(Debug)]
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
    pub fn new(host: impl Into<String>, port: u16, auth: &SshAuthConfig) -> Self {
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

#[cfg(test)]
mod group_b_tests {
    use super::*;
    use zeroize::Zeroize;

    /// Pin Drop's zeroization effect on the same types Drop touches.
    /// We cannot safely read heap after a full `drop` (UB); instead we run the
    /// identical `zeroize()` calls while the `String`s are still allocated.
    #[test]
    fn secret_bytes_zeroed_matching_drop_body() {
        let mut key_passphrase = Some(String::from("passphrase-characterization!!"));
        let mut password = Some(String::from("password-characterization!!!"));

        let pp_ptr = key_passphrase.as_ref().unwrap().as_ptr();
        let pp_len = key_passphrase.as_ref().unwrap().len();
        let pw_ptr = password.as_ref().unwrap().as_ptr();
        let pw_len = password.as_ref().unwrap().len();

        // Same operations as `SshAuthConfig::drop`
        if let Some(ref mut passphrase) = key_passphrase {
            passphrase.zeroize();
        }
        if let Some(ref mut password) = password {
            password.zeroize();
        }

        assert_eq!(key_passphrase.as_ref().unwrap().len(), 0);
        assert_eq!(password.as_ref().unwrap().len(), 0);
        unsafe {
            assert!(
                std::slice::from_raw_parts(pp_ptr, pp_len)
                    .iter()
                    .all(|&b| b == 0),
                "passphrase bytes must be zeroed"
            );
            assert!(
                std::slice::from_raw_parts(pw_ptr, pw_len)
                    .iter()
                    .all(|&b| b == 0),
                "password bytes must be zeroed"
            );
        }

        // Smoke: real Drop path runs without panic
        drop(SshAuthConfig::with_password("u", "x"));
        drop(SshAuthConfig::with_key_and_passphrase("u", "/tmp/k", "y"));
    }

    /// sqlator-33j: Clone removed so Drop zeroization cannot be defeated by copies.
    /// Stable Rust has no `T: !Clone` bound; the `compile_fail` doctest on
    /// [`SshAuthConfig`] is the compile-time guard. This test documents the
    /// invariant and exercises construction/drop without cloning.
    #[test]
    fn ssh_auth_config_does_not_rely_on_clone() {
        let a = SshAuthConfig::with_password("u", "secret");
        assert_eq!(a.password.as_deref(), Some("secret"));
        assert_eq!(a.username, "u");
        drop(a);
    }
}
