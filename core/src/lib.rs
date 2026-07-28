pub mod config;
pub mod credentials;
pub mod db;
pub mod docker;
pub mod error;
pub mod models;
pub mod ssh;

pub use credentials::{CredentialStore, StorageMode, VaultSettings};
pub use models::{
    BatchError, BatchResult, ColumnMeta, ConnectionGroup, ConnectionType, FilterSpec,
    PrimaryKeyMeta, SchemaColumnInfo, SchemaInfo, SortSpec, SqlBatch, TableInfo, TableMeta,
    TableQueryParams, TableQueryResult,
};

pub use db::{detect_database_type, DatabaseType, DbManager};
pub use docker::{ContainerInfo, ContainerPort, ContainerStatus, ContainerSummary, DockerError};
pub use ssh::{
    AuthMethod, HostEntry, JumpHost, LocalForward, SshAuthConfig, SshAuthConfigData, SshCommand,
    SshCommandResult, SshError, SshHostConfig, SshResult, SshSession, SshTunnel, TunnelHandle,
};
