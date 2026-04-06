use crate::error::CoreError;
use keyring::Entry;

const SERVICE: &str = "sqlator";

fn credential_key(profile_id: &str, kind: &str) -> String {
    format!("ssh-profile:{}:{}", profile_id, kind)
}

/// Store a credential in the OS keyring.
/// `kind` is either `"password"` or `"passphrase"`.
pub fn store_credential(profile_id: &str, kind: &str, secret: &str) -> Result<(), CoreError> {
    let key = credential_key(profile_id, kind);
    Entry::new(SERVICE, &key)
        .and_then(|e| e.set_password(secret))
        .map_err(|e| CoreError {
            message: format!("Keyring store failed: {e}"),
            code: "KEYRING_ERROR".into(),
        })
}

/// Retrieve a credential from the OS keyring.
/// Returns `None` when the entry doesn't exist (not an error).
pub fn get_credential(profile_id: &str, kind: &str) -> Result<Option<String>, CoreError> {
    let key = credential_key(profile_id, kind);
    match Entry::new(SERVICE, &key).and_then(|e| e.get_password()) {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(CoreError {
            message: format!("Keyring read failed: {e}"),
            code: "KEYRING_ERROR".into(),
        }),
    }
}

/// Delete a credential from the OS keyring (idempotent — missing entry is OK).
pub fn delete_credential(profile_id: &str, kind: &str) -> Result<(), CoreError> {
    let key = credential_key(profile_id, kind);
    match Entry::new(SERVICE, &key).and_then(|e| e.delete_credential()) {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CoreError {
            message: format!("Keyring delete failed: {e}"),
            code: "KEYRING_ERROR".into(),
        }),
    }
}

/// Delete all credentials associated with an SSH profile.
pub fn delete_all_credentials(profile_id: &str) -> Result<(), CoreError> {
    delete_credential(profile_id, "password")?;
    delete_credential(profile_id, "passphrase")?;
    Ok(())
}
