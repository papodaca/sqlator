//! Credential storage-mode switching and vault create/unlock/lock/settings.
//!
//! Auth resolvers (`build_auth_config_for_profile`) live in [`crate::ssh`] so
//! tunnel/profile helpers stay co-located. Vault/storage `AppService` entry
//! points (including `spawn_blocking` for Argon2 unlock) arrive when frontends
//! migrate onto `AppService` in Phase 1e.

#![allow(dead_code)]
