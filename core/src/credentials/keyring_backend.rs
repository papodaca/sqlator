use crate::error::CoreError;
use keyring::Entry;

const SERVICE: &str = "sqlator";

pub struct KeyringBackend;

impl KeyringBackend {
    /// Returns true if the OS keyring is usable (a test round-trip succeeds).
    pub fn available() -> bool {
        let test_key = "__sqlator_keyring_probe__";
        let Ok(entry) = Entry::new(SERVICE, test_key) else {
            return false;
        };
        let ok = entry.set_password("probe").is_ok();
        let _ = entry.delete_credential();
        ok
    }
}

impl super::CredentialBackend for KeyringBackend {
    fn store(&self, key: &str, secret: &str) -> Result<(), CoreError> {
        Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(secret))
            .map_err(|e| CoreError {
                message: format!("Keyring store failed: {e}"),
                code: "KEYRING_ERROR".into(),
            })
    }

    fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        match Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError {
                message: format!("Keyring read failed: {e}"),
                code: "KEYRING_ERROR".into(),
            }),
        }
    }

    fn delete(&self, key: &str) -> Result<(), CoreError> {
        match Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError {
                message: format!("Keyring delete failed: {e}"),
                code: "KEYRING_ERROR".into(),
            }),
        }
    }
}
