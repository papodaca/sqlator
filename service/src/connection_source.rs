//! Connection-set abstraction (multi-db store vs fixed singleton).
//!
//! Resolves web `single_db` / future GTK `--config` without baking policy into
//! each frontend. Implementations land with the connections module move.

use crate::error::ServiceError;
use sqlator_core::models::SavedConnection;

/// Source of truth for which connections the service can list and mutate.
pub trait ConnectionSource: Send + Sync {
    /// All connections visible in the current mode.
    fn list_connections(&self) -> Result<Vec<SavedConnection>, ServiceError>;

    /// Look up one connection by id.
    fn get_connection(&self, id: &str) -> Result<Option<SavedConnection>, ServiceError>;

    /// Whether connection CRUD (add/remove/import) is allowed.
    ///
    /// Single-db / fixed-config mode returns `false`.
    fn allows_mutation(&self) -> bool;

    /// Guard used by multi-db-only operations (`require_multi_db`).
    fn require_multi_db(&self) -> Result<(), ServiceError> {
        if self.allows_mutation() {
            Ok(())
        } else {
            Err(ServiceError::app(
                "MULTI_DB_REQUIRED",
                "This operation is not available in single-database mode",
            ))
        }
    }
}
