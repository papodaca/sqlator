---
title: "feat: Docker Container Database Connections"
type: feat
status: active
date: 2026-04-05
origin: docs/plans/2026-04-04-002-feat-enhanced-connection-manager-plan.md
brainstorm: docs/brainstorms/2026-04-04-docker-container-database-connections-requirements.md
---

# feat: Docker Container Database Connections

## Overview

Extend the Enhanced Connection Manager (Plan 002) to support connections to databases running inside Docker containers that don't expose ports to the host. This enhancement builds on the SSH tunneling infrastructure from Plan 002, adding Docker container discovery and IP-based tunneling to reach container-internal databases.

**Relationship to existing plans:**
- **Depends on Plan 002 (Enhanced Connection Manager):** Uses SSH profiles, russh tunneling, and credential storage from that plan
- **Compatible with Plan 007 (Web Version):** Docker connections work identically in desktop and web modes

## Problem Statement

Users with databases in Docker containers face a connectivity gap when containers don't expose ports. Today they manually SSH into servers and use `docker exec` to run database commands, losing the benefits of a SQL client. This feature bridges that gap by extending the SSH tunnel infrastructure to reach container-internal IPs.

## Proposed Solution

### Architecture (Extends Plan 002)

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Svelte 5 Frontend                               │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │  API Adapter Layer (from Plan 007)                             │ │
│  │  - invoke_command() → fetch() [web] or Tauri IPC [desktop]     │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │  Container Connection Wizard (NEW)                             │ │
│  │  1. SSH Profile Selection (uses profiles from Plan 002)        │ │
│  │  2. Container Name Input                                       │ │
│  │  3. Container Discovery (docker inspect over SSH)              │ │
│  │  4. Database Credentials + Container Port                      │ │
│  │  5. Connection Test → Save as regular connection               │ │
│  └────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
┌───────────────────────────────▼─────────────────────────────────────┐
│           Tauri 2 App / Web Server (Plan 007)                        │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │  Commands (extend Plan 002):                                    │ │
│  │  - discover_container(ssh_profile_id, container_name)          │ │
│  │  - connect_database() → handles DockerContainer type (NEW)     │ │
│  │  - disconnect_database() → closes tunnel + pool (existing)     │ │
│  └────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
┌───────────────────────────────▼─────────────────────────────────────┐
│           Core Library (Pure Rust)                                   │
│                                                                       │
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────────────┐ │
│  │  SSH (Plan 002) │  │  Docker (NEW)   │  │  DbManager           │ │
│  │  - SshProfile   │  │  - Inspector    │  │  - AnyPool           │ │
│  │  - SshTunnel    │  │  - docker       │  │  - Query execution   │ │
│  │  - TunnelMgr    │  │    inspect      │  │                      │ │
│  └─────────────────┘  └─────────────────┘  └──────────────────────┘ │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │  Config Manager (Plan 002)                                     │ │
│  │  - SavedConnection (extended with container fields)            │ │
│  │  - SshProfile (from Plan 002, no changes)                      │ │
│  │  - CredentialManager (from Plan 002, no changes)               │ │
│  └────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

### Connection Flow

**Desktop Mode:**
1. User creates connection via wizard → selects SSH profile (from Plan 002) → enters container name
2. `discover_container` runs `docker inspect` over SSH → extracts container IP
3. User enters DB credentials + container port → test → save
4. On connect: SSH tunnel created to `container_ip:container_port` → DB pool connects to `localhost:tunnel_port`

**Web Mode (Plan 007):**
1. Same wizard flow, commands go through API adapter
2. SSH tunnel and DB pool are created on the server side
3. Query results stream over WebSocket (existing Plan 007 mechanism)

## Technical Approach

### Phase 0: Prerequisites (From Plan 002)

**This plan assumes Plan 002 is implemented first:**
- SshProfile model with full fields (host, port, username, auth_method, key_path, proxy_jump, keepalive)
- SshTunnel implementation using russh
- TunnelManager with port allocation
- Credential storage (keyring or vault)
- SSH profile CRUD commands and UI

**No duplicated SSH infrastructure in this plan.**

### Phase 1: Docker Container Inspection

**Goal:** Add Docker container discovery capability.

**Implementation:**

```rust
// core/src/docker/mod.rs
pub mod inspector;

// core/src/docker/inspector.rs
use crate::ssh::SshSession;  // From Plan 002

pub struct ContainerInfo {
    pub ip_address: String,
    pub status: ContainerStatus,
    pub ports: Vec<ContainerPort>,
    pub labels: HashMap<String, String>,
    pub database_type_hint: Option<String>,  // Detected from labels/image
}

pub enum ContainerStatus {
    Running,
    Stopped,
    NotFound,
}

pub struct ContainerPort {
    pub container_port: u16,
    pub protocol: String,
}

impl ContainerInspector {
    /// Execute `docker inspect` over SSH session
    pub async fn inspect(
        ssh_session: &SshSession,
        container_name: &str,
    ) -> Result<ContainerInfo, DockerError>;
    
    /// Execute `docker ps --format json` to list running containers
    pub async fn list_running(
        ssh_session: &SshSession,
    ) -> Result<Vec<ContainerSummary>, DockerError>;
}

pub enum DockerError {
    ContainerNotFound(String),
    ContainerStopped(String),
    PermissionDenied,
    DaemonUnreachable,
    SshError(crate::ssh::SshError),
    ParseError(String),
}
```

**Tasks:**
- [ ] Create `core/src/docker/mod.rs` and `inspector.rs`
- [ ] Implement `docker inspect` command execution over SSH
- [ ] Parse JSON output to extract IP address and status
- [ ] Implement container status detection
- [ ] Add error handling for all failure modes
- [ ] Sanitize container name input (prevent command injection)
- [ ] Add optional `docker ps` listing for future auto-discovery

### Phase 2: Connection Model Extension

**Goal:** Extend SavedConnection to support Docker containers.

**Data model changes (extend Plan 002's SavedConnection):**

```rust
// core/src/models.rs

/// Connection type discriminator
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ConnectionType {
    Direct,           // Standard URL connection (existing)
    SshTunnel,        // SSH tunnel to host:port (from Plan 002)
    DockerContainer,  // SSH tunnel to container IP (NEW)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedConnection {
    // ... existing fields from Plan 002 ...
    pub id: String,
    pub name: String,
    pub group_id: Option<String>,
    pub color_id: String,
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub ssh_profile_id: Option<String>,  // From Plan 002
    
    // NEW fields for Docker support
    pub connection_type: ConnectionType,
    pub container_name: Option<String>,   // For DockerContainer type
    pub container_port: Option<u16>,      // Port inside container (defaults to db_type standard)
}

impl Default for ConnectionType {
    fn default() -> Self {
        ConnectionType::Direct
    }
}
```

**Migration for existing connections:**
- All existing connections get `connection_type: Direct`
- Connections with `ssh_profile_id` but no `container_name` are `SshTunnel` type
- Connections with both `ssh_profile_id` and `container_name` are `DockerContainer` type

**Tasks:**
- [ ] Add `ConnectionType` enum to `models.rs`
- [ ] Add `container_name` and `container_port` fields to `SavedConnection`
- [ ] Implement migration logic in config loader
- [ ] Update TypeScript types in `src/lib/types.ts`

### Phase 3: Connect/Disconnect Logic

**Goal:** Extend connection logic to handle DockerContainer type.

**Implementation (extend Plan 002's connect logic):**

```rust
// core/src/db.rs

impl DbManager {
    /// Connect to a database, optionally via SSH tunnel to container
    pub async fn connect_docker_container(
        &self,
        connection_id: &str,
        connection: &SavedConnection,
        ssh_profile: &SshProfile,
        tunnel_manager: &TunnelManager,
    ) -> Result<(), CoreError> {
        // 1. Create SSH session using profile (from Plan 002)
        let session = SshSession::connect(ssh_profile).await?;
        
        // 2. Discover container IP
        let container_name = connection.container_name.as_ref()
            .ok_or(CoreError::MissingContainerName)?;
        let info = ContainerInspector::inspect(&session, container_name).await?;
        
        // 3. Check container is running
        if info.status != ContainerStatus::Running {
            return Err(CoreError::ContainerNotRunning(container_name.clone()));
        }
        
        // 4. Determine container port
        let container_port = connection.container_port
            .unwrap_or_else(|| default_port_for_db_type(&connection.db_type));
        
        // 5. Create SSH tunnel to container_ip:container_port
        let tunnel = tunnel_manager.create_tunnel(
            connection_id,
            ssh_profile,
            &info.ip_address,
            container_port,
        ).await?;
        
        // 6. Connect database to tunnel endpoint
        let local_port = tunnel.local_port;
        let url = build_connection_url(
            &connection.db_type,
            "127.0.0.1",
            local_port,
            &connection.database,
            &connection.username,
            // password from credential manager
        );
        
        self.connect(connection_id, &url).await
    }
}
```

**Tasks:**
- [ ] Add `connect_docker_container` method to `DbManager`
- [ ] Extend `connect_database` command to handle `DockerContainer` type
- [ ] Ensure `disconnect_database` closes tunnels (already handled by Plan 002)
- [ ] Handle container IP re-discovery on reconnect

### Phase 4: Tauri/Web Commands

**Goal:** Add Docker discovery command, extend existing commands.

**New command:**

```rust
// src-tauri/src/commands/docker.rs (or in web-server)

#[tauri::command]
pub async fn discover_container(
    state: State<'_, AppState>,
    ssh_profile_id: String,
    container_name: String,
) -> Result<ContainerInfo, CommandError> {
    let profile = state.config.get_ssh_profile(&ssh_profile_id)?;
    let session = SshSession::connect(&profile).await?;
    ContainerInspector::inspect(&session, &container_name)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn list_running_containers(
    state: State<'_, AppState>,
    ssh_profile_id: String,
) -> Result<Vec<ContainerSummary>, CommandError> {
    let profile = state.config.get_ssh_profile(&ssh_profile_id)?;
    let session = SshSession::connect(&profile).await?;
    ContainerInspector::list_running(&session)
        .await
        .map_err(CommandError::from)
}
```

**Web API endpoints (Plan 007 compatibility):**

```rust
// web-server/src/routes/docker.rs

pub fn docker_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/discover", post(discover_container))
        .route("/list", post(list_containers))
}

async fn discover_container(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DiscoverRequest>,
) -> Result<Json<ContainerInfo>, ApiError> {
    // Same logic as Tauri command
}
```

**Tasks:**
- [ ] Create `src-tauri/src/commands/docker.rs`
- [ ] Implement `discover_container` command
- [ ] Implement `list_running_containers` command (for future use)
- [ ] Register commands in `lib.rs`
- [ ] Add equivalent REST endpoints in web-server (Plan 007)
- [ ] Update API adapter layer for web mode

### Phase 5: Frontend Wizard

**Goal:** Create guided setup flow for Docker container connections.

**Integration with Plan 002's connection form:**

The wizard extends the existing connection form with a Docker-specific flow. It reuses:
- `SshProfileSelector.svelte` from Plan 002
- Credential forms from Plan 002
- Connection testing from Plan 002

```svelte
<!-- src/lib/components/DockerConnectionWizard.svelte -->
<script lang="ts">
  import { discoverContainer } from '$lib/api';
  import SshProfileSelect from './SshProfileSelect.svelte';  // From Plan 002
  
  let step = $state<'ssh' | 'container' | 'credentials' | 'test'>('ssh');
  let sshProfileId = $state<string>('');
  let containerName = $state('');
  let containerInfo = $state<ContainerInfo | null>(null);
  let containerPort = $state<number>(5432);
  let dbCredentials = $state({ username: '', password: '', database: '' });
  
  async function handleDiscover() {
    containerInfo = await discoverContainer(sshProfileId, containerName);
    // Auto-detect port from container labels if available
    if (containerInfo?.database_type_hint === 'postgres') {
      containerPort = 5432;
    } else if (containerInfo?.database_type_hint === 'mysql') {
      containerPort = 3306;
    }
    step = 'credentials';
  }
  
  async function handleSave() {
    // Save as SavedConnection with type: DockerContainer
    await saveConnection({
      name: `${containerName} (${containerInfo.ip_address})`,
      connection_type: 'DockerContainer',
      ssh_profile_id: sshProfileId,
      container_name: containerName,
      container_port: containerPort,
      ...dbCredentials,
    });
  }
</script>

<div class="wizard">
  {#if step === 'ssh'}
    <SshProfileSelect bind:profileId={sshProfileId} onNext={() => step = 'container'} />
  {:else if step === 'container'}
    <div class="form-group">
      <label>Container Name</label>
      <input bind:value={containerName} placeholder="my-database-container" />
      <button onclick={handleDiscover}>Discover</button>
    </div>
  {:else if step === 'credentials'}
    <ContainerStatus info={containerInfo} />
    <div class="form-group">
      <label>Container Port</label>
      <input type="number" bind:value={containerPort} />
    </div>
    <DbCredentialsForm bind:credentials={dbCredentials} />
    <button onclick={() => step = 'test'}>Test Connection</button>
  {:else if step === 'test'}
    <ConnectionTest ... onsuccess={handleSave} />
  {/if}
</div>
```

**Tasks:**
- [ ] Create `DockerConnectionWizard.svelte`
- [ ] Create `ContainerInput.svelte` with discover button
- [ ] Create `ContainerStatus.svelte` to show discovered info
- [ ] Add "New Container Connection" option in connections sidebar
- [ ] Extend `saveConnection` to handle DockerContainer type
- [ ] Update API client for new Docker commands

### Phase 6: Error Handling

**Goal:** Comprehensive error handling with actionable messages.

**Error taxonomy (extends Plan 002):**

| Category | Error | User Message |
|----------|-------|--------------|
| SSH | (handled by Plan 002) | — |
| Docker | ContainerNotFound | "Container '{name}' not found. Check the name and try again." |
| Docker | ContainerStopped | "Container '{name}' is not running. Start it and try again." |
| Docker | PermissionDenied | "Permission denied. User may not have Docker access on the server." |
| Docker | DaemonUnreachable | "Docker daemon not responding on server." |
| Docker | NetworkIsolated | "Container is on an isolated network. Check Docker network configuration." |
| DB | (handled by existing logic) | — |

**Tasks:**
- [ ] Create `DockerError` enum with all variants
- [ ] Map to user-friendly messages
- [ ] Add error display in wizard
- [ ] Add retry options for transient errors

### Phase 7: Same-Server Mode

**Goal:** Support Docker socket access when Sqlator runs on the same server.

**Implementation:**

```rust
// core/src/docker/local.rs

pub struct LocalDockerAccess {
    socket_path: PathBuf,
}

impl LocalDockerAccess {
    pub fn new() -> Result<Self, DockerError> {
        let socket = PathBuf::from("/var/run/docker.sock");
        if socket.exists() {
            Ok(Self { socket_path: socket })
        } else {
            Err(DockerError::SocketNotFound)
        }
    }
    
    /// Inspect container using local Docker socket (no SSH)
    pub async fn inspect(&self, container_name: &str) -> Result<ContainerInfo, DockerError>;
    
    /// Connect directly to container IP (no tunnel)
    pub async fn connect_direct(
        &self,
        info: &ContainerInfo,
        port: u16,
        credentials: &DbCredentials,
    ) -> Result<AnyPool, Error>;
}
```

**Connection type for local Docker:**

```rust
pub enum ConnectionType {
    Direct,
    SshTunnel,
    DockerContainer,       // SSH tunnel to container
    LocalDockerContainer,  // Direct connection to local container (NEW)
}
```

**Tasks:**
- [ ] Implement `LocalDockerAccess`
- [ ] Add socket permission detection
- [ ] Add "Local Docker" option in wizard
- [ ] Handle permission errors gracefully

## System-Wide Impact

### Interaction with Plan 002 (Enhanced Connection Manager)

- **SshProfile:** Reused unchanged. Docker connections reference existing profiles.
- **TunnelManager:** Extended to support container IP as tunnel target.
- **CredentialManager:** Reused unchanged for DB credentials.
- **Connection Groups:** Docker connections are saved as regular connections, support grouping.

### Interaction with Plan 007 (Web Version)

- **Tunnel Location:** SSH tunnels run on the server side in web mode.
- **API Adapter:** Docker commands use the same abstraction as other commands.
- **Streaming:** Query results stream over WebSocket regardless of connection type.
- **Single-DB Mode:** Not applicable to Docker connections (Docker implies multi-container scenarios).

### State Lifecycle

| Event | Action |
|-------|--------|
| Connect to Docker connection | SSH session → docker inspect → tunnel → DB pool |
| Disconnect | Close DB pool → close tunnel (Plan 002 handles this) |
| App/Server exit | TunnelManager closes all tunnels (Plan 002) |
| Container restarts mid-session | Connection fails; user reconnects; IP re-discovered |

## Acceptance Criteria

### Functional Requirements

- [ ] AC-01 User can create a connection targeting a Docker container by name
- [ ] AC-02 Sqlator discovers container's internal IP via SSH + `docker inspect`
- [ ] AC-03 SSH tunnel establishes to container's internal IP (using Plan 002 infrastructure)
- [ ] AC-04 Database connection through tunnel provides full SQL client features
- [ ] AC-05 Connection works on read-only servers (no server modifications)
- [ ] AC-06 User selects existing SSH profile (from Plan 002) or creates new one
- [ ] AC-07 Container connections are saved as regular connections
- [ ] AC-08 Setup uses a guided wizard flow
- [ ] AC-09 Container IP is re-discovered on each connection
- [ ] AC-10 User can specify non-standard database ports inside containers
- [ ] AC-11 Clear error messages for all failure modes
- [ ] AC-12 Works identically in desktop (Tauri) and web (Plan 007) modes

### Non-Functional Requirements

- [ ] SSH operations timeout after 30 seconds
- [ ] Container discovery completes within 5 seconds on normal servers
- [ ] Container names are sanitized to prevent command injection
- [ ] No duplicate SSH infrastructure (reuse Plan 002)

### Security Requirements

- [ ] Container names sanitized before passing to shell commands
- [ ] SSH tunnels bind to localhost only
- [ ] Credentials stored via CredentialManager (Plan 002)
- [ ] No new credential storage mechanisms

## Dependencies

### Prerequisites (Must be implemented first)

- **Plan 002: Enhanced Connection Manager** — SSH profiles, tunneling, credential storage
- **Plan 007: Web Version** (optional) — API adapter layer

### New Dependencies

No new Rust dependencies beyond what Plan 002 already adds (russh, russh-keys).

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Container IP changes mid-session | Medium | Medium | Re-discover on each connect; document limitation |
| Container on isolated Docker network | Medium | Medium | Detect and report; suggest network config |
| Docker inspect hangs | Low | Medium | 30-second timeout on SSH commands |
| Plan 002 not implemented | N/A | Critical | This plan requires Plan 002 as prerequisite |

## Future Considerations

- Auto-discover available containers (`docker ps` integration)
- Detect database type from container image/labels
- Connection recovery after container restart (auto-reconnect)
- Docker Compose service discovery (connect by service name)

## Sources & References

### Origin

- **Prerequisite:** [docs/plans/2026-04-04-002-feat-enhanced-connection-manager-plan.md](docs/plans/2026-04-04-002-feat-enhanced-connection-manager-plan.md)
  - SSH infrastructure reused: russh, SshProfile, SshTunnel, TunnelManager
  - Credential storage reused: CredentialManager, keyring, vault
  - Connection model extended: SavedConnection with container fields
- **Compatible with:** [docs/plans/2026-04-04-007-feat-web-version-server-mode-plan.md](docs/plans/2026-04-04-007-feat-web-version-server-mode-plan.md)
  - API adapter layer for web mode
  - Server-side tunnel management
- **Requirements:** [docs/brainstorms/2026-04-04-docker-container-database-connections-requirements.md](docs/brainstorms/2026-04-04-docker-container-database-connections-requirements.md)

### Internal References

- SSH tunnel infrastructure: Plan 002, Phase 1
- SshProfile model: Plan 002, lines 260-282
- TunnelManager: Plan 002, architecture diagram
- API adapter: Plan 007, Phase 1
- SavedConnection model: `core/src/models.rs`

### External References

- [Docker inspect format](https://docs.docker.com/engine/api/v1.45/#tag/Container/operation/ContainerInspect)
- [Docker container networking](https://docs.docker.com/network/)
