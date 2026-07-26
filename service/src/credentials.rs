//! Credential storage-mode switching and vault create/unlock/lock/settings.
//!
//! Auth resolvers (`build_auth_config_for_profile`) live in [`crate::ssh`] so
//! tunnel/profile helpers stay co-located. Vault unlock/create run under
//! [`spawn_blocking`](tokio::task::spawn_blocking) (Argon2 is expensive).

use crate::error::ServiceError;
use crate::service::{run_blocking, AppService};
use sqlator_core::credentials::{StorageMode, VaultSettings};
use std::sync::Arc;

impl AppService {
    pub fn keyring_available() -> bool {
        sqlator_core::credentials::CredentialStore::keyring_available()
    }

    pub async fn get_storage_mode(&self) -> Result<String, ServiceError> {
        Ok(self.credentials.mode().to_string())
    }

    pub async fn set_storage_mode(&self, mode: String, migrate: bool) -> Result<(), ServiceError> {
        let new_mode: StorageMode = mode
            .parse()
            .map_err(|e: sqlator_core::error::CoreError| ServiceError::from(e))?;
        let credentials = Arc::clone(&self.credentials);
        let config = Arc::clone(&self.config);

        run_blocking(move || {
            if migrate {
                let profiles = config.get_ssh_profiles()?;
                let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
                credentials.migrate_to(&new_mode, &ids)?;
            }
            credentials.set_mode(new_mode);
            config.save_storage_mode(&mode)?;
            Ok::<(), ServiceError>(())
        })
        .await
    }

    pub async fn vault_exists(&self) -> Result<bool, ServiceError> {
        Ok(self.credentials.vault.is_initialized())
    }

    pub async fn is_vault_locked(&self) -> Result<bool, ServiceError> {
        Ok(self.credentials.vault.is_locked())
    }

    pub async fn create_vault(&self, password: String) -> Result<(), ServiceError> {
        let credentials = Arc::clone(&self.credentials);
        let config = Arc::clone(&self.config);
        run_blocking(move || {
            credentials.vault.create(&password)?;
            credentials.set_mode(StorageMode::Vault);
            config.save_storage_mode("vault")?;
            Ok::<(), ServiceError>(())
        })
        .await
    }

    pub async fn unlock_vault(&self, password: String) -> Result<(), ServiceError> {
        let credentials = Arc::clone(&self.credentials);
        run_blocking(move || {
            credentials.vault.unlock(&password)?;
            Ok::<(), ServiceError>(())
        })
        .await
    }

    pub async fn lock_vault(&self) -> Result<(), ServiceError> {
        self.credentials.vault.lock();
        Ok(())
    }

    pub async fn get_vault_settings(&self) -> Result<VaultSettings, ServiceError> {
        let config = Arc::clone(&self.config);
        run_blocking(move || {
            let timeout_secs = config.get_vault_timeout_secs()?;
            Ok::<_, ServiceError>(VaultSettings { timeout_secs })
        })
        .await
    }

    pub async fn save_vault_settings(&self, settings: VaultSettings) -> Result<(), ServiceError> {
        let credentials = Arc::clone(&self.credentials);
        let config = Arc::clone(&self.config);
        run_blocking(move || {
            credentials.vault.set_timeout(settings.timeout_secs);
            config.save_vault_timeout_secs(settings.timeout_secs)?;
            Ok::<(), ServiceError>(())
        })
        .await
    }
}
