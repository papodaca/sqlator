pub mod config;
pub mod credentials;
pub mod db;
pub mod docker;
pub mod error;
pub mod models;
pub mod ssh;

pub use credentials::{CredentialStore, StorageMode, VaultSettings};
pub use models::{BatchResult, BatchError, ColumnMeta, ConnectionGroup, ConnectionType, PrimaryKeyMeta, SqlBatch, TableMeta,
    SchemaInfo, TableInfo, SchemaColumnInfo, SortSpec, FilterSpec, TableQueryParams, TableQueryResult};

pub use db::{detect_database_type, DatabaseType, DbManager};
pub use docker::{ContainerInfo, ContainerPort, ContainerStatus, ContainerSummary, DockerError};
pub use ssh::{
    AuthMethod, HostEntry, JumpHost, SshAuthConfig, SshAuthConfigData, SshCommand, SshCommandResult,
    SshError, SshHostConfig, SshResult, SshTunnel, TunnelHandle,
};
