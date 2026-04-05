---
title: "feat: Enhanced Connection Manager with SSH Tunneling"
type: feat
status: active
date: 2026-04-04
origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md
---

# 🔐 Enhanced Connection Manager with SSH Tunneling

A comprehensive upgrade to the MVP connection manager, adding SSH tunnel support, manual field configuration, reusable SSH profiles, configurable credential storage, and organizational features.

---

## Overview

This enhancement transforms the basic URL-based connection manager into a production-grade system supporting:
- **SSH tunneling** through jump hosts with multiple auth methods
- **Flexible input modes** via tabbed URL/Manual entry
- **Reusable SSH profiles** for shared tunnel configurations
- **Configurable credential storage** with OS keyring or master password encryption
- **Connection organization** via groups, import/export, cloning, and status badges

---

## Problem Statement

The MVP connection manager (origin: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:174-257`) supports only direct database connections via URL entry. Real-world database access often requires:
- SSH tunnels through bastion hosts for security
- Separate credential management for SSH vs database auth
- Flexible configuration when URLs aren't provided
- Organization for users managing many connections

---

## Proposed Solution

### Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Svelte 5 Frontend                               │
│  ┌────────────────┐  ┌──────────────────┐  ┌────────────────────┐  │
│  │  Sidebar       │  │  Connection      │  │  SSH Profile       │  │
│  │  Groups/       │  │  Form            │  │  Manager           │  │
│  │  Connections   │  │  (Tabbed UI)     │  │  (Reusable)        │  │
│  └───────┬────────┘  └────────┬─────────┘  └─────────┬──────────┘  │
│          │                    │                      │              │
│          └────────────────────┼──────────────────────┘              │
│                               │ invoke()                            │
└───────────────────────────────┼─────────────────────────────────────┘
                                │
┌───────────────────────────────┼─────────────────────────────────────┐
│           Tauri 2 Rust Backend                                       │
│                               │                                       │
│  ┌────────────────────────────▼──────────────────────────────────┐  │
│  │  Commands: save_connection, get_connections, test_connection, │  │
│  │  create_ssh_tunnel, manage_ssh_profiles, import/export,       │  │
│  │  validate_ssh_config                                          │  │
│  └───────┬────────────────────────────────────────────────────────┘  │
│          │                                                            │
│  ┌───────▼──────────┐  ┌─────────────────┐  ┌────────────────────┐  │
│  │  russh           │  │  Credential     │  │  russh-config      │  │
│  │  (SSH Tunnels)   │  │  Manager        │  │  (SSH Config)      │  │
│  │  - Key auth      │  │  - keyring      │  │  - Parse ~/.ssh/   │  │
│  │  - Password      │  │  - Encrypted    │  │  - List hosts      │  │
│  │  - Agent fwd     │  │    vault        │  │  - ProxyJump       │  │
│  └──────────────────┘  └─────────────────┘  └────────────────────┘  │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │  tauri-plugin-store (connection metadata + SSH profiles)       │  │
│  │  + groups + status + import history                            │  │
│  └────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────────────┘
```

### Credential Storage Architecture

```
User Preference ──────┬────── OS Keyring Available?
                      │
          ┌───────────┴───────────┐
          │ YES                   │ NO
          ▼                       ▼
┌─────────────────┐    ┌─────────────────────────┐
│ keyring 3.3     │    │ Encrypted Vault         │
│ - macOS Keychain│    │ - Argon2id KDF          │
│ - Win DPAPI     │    │ - AES-256-GCM           │
│ - Linux libsecret│   │ - Master password       │
└─────────────────┘    │ - Portable file         │
                       └─────────────────────────┘
```

---

## Technical Approach

### SSH Tunneling with russh

**Why russh over alternatives:**
- Pure Rust (no C dependencies, easier cross-platform builds)
- Native async/await with tokio
- Built-in port forwarding support
- SSH agent integration via `russh-keys`

**Dependencies (Cargo.toml):**

```toml
[dependencies]
russh = { version = "0.50", features = ["aws-lc-rs"] }
russh-keys = "0.50"
russh-config = "0.50"
ssh2-config = "0.5"           # Alternative config parser
keyring = "3.3"
argon2 = "0.5"
aes-gcm = "0.10"
zeroize = "1"
rand = "0.8"
```

**Tunnel Lifecycle:**

```rust
// src-tauri/src/ssh/tunnel.rs
pub struct SshTunnel {
    pub profile_id: String,
    pub local_port: u16,
    pub session: Arc<Mutex<client::Handle<Client>>>,
    pub cancel_token: CancellationToken,
}

impl SshTunnel {
    pub async fn create(profile: &SshProfile, target: &TargetHost) -> Result<Self, TunnelError> {
        // 1. Load credentials from storage
        // 2. Connect to SSH host (via jump chain if configured)
        // 3. Bind local port
        // 4. Forward traffic through tunnel
        // 5. Return handle for cleanup
    }
}
```

### SSH Config Parsing

```rust
// src-tauri/src/ssh/config_parser.rs
use russh_config::SshConfig;

pub fn load_ssh_config() -> Result<SshConfigEntries, ConfigError> {
    let config_path = dirs::home_dir()
        .map(|h| h.join(".ssh/config"))
        .ok_or(ConfigError::HomeNotFound)?;
    
    let config = SshConfig::parse(&config_path)?;
    
    Ok(SshConfigEntries {
        hosts: config.hosts().into_iter().map(|h| HostEntry {
            alias: h.alias,
            hostname: h.hostname,
            port: h.port.unwrap_or(22),
            user: h.user,
            identity_file: h.identity_file,
            proxy_jump: h.proxy_jump,
        }).collect()
    })
}
```

### Connection Form: Tabbed Interface

**Frontend structure:**

```svelte
<!-- src/lib/components/ConnectionForm.svelte -->
<script lang="ts">
  let activeTab = $state<'url' | 'manual'>('url');
  let formData = $state<ConnectionFormData>({ ... });
</script>

<div class="connection-form">
  <div class="tabs">
    <button class:active={activeTab === 'url'} onclick={() => activeTab = 'url'}>
      Quick (URL)
    </button>
    <button class:active={activeTab === 'manual'} onclick={() => activeTab = 'manual'}>
      Advanced (Fields)
    </button>
  </div>
  
  {#if activeTab === 'url'}
    <UrlInput bind:value={formData} onparse={handleUrlParse} />
  {:else}
    <ManualFields bind:value={formData} />
  {/if}
  
  <SshProfileSelector bind:profileId={formData.sshProfileId} />
</div>
```

**URL parsing (Rust):**

```rust
// src-tauri/src/commands/connections.rs
#[tauri::command]
pub async fn parse_connection_url(url: String) -> Result<ParsedConnection, CommandError> {
    let parsed = url::Url::parse(&url)?;
    
    Ok(ParsedConnection {
        db_type: match parsed.scheme() {
            "postgres" | "postgresql" => DbType::Postgres,
            "mysql" => DbType::Mysql,
            "sqlite" => DbType::Sqlite,
            _ => return Err(CommandError::UnsupportedDatabase),
        },
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed.port().unwrap_or_default_port(&db_type),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        password: parsed.password().map(|p| p.to_string()),
    })
}
```

### Reusable SSH Profiles

**Data model:**

```typescript
// src/lib/types.ts
export interface SshProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: 'key' | 'password' | 'agent';
  keyPath?: string;           // Custom key file
  keyPassphrase?: string;     // For encrypted keys (stored in keyring)
  password?: string;          // Stored in keyring
  proxyJump?: SshJumpHost[];  // Jump host chain
  localPortBinding?: number;  // Custom local port (0 = auto)
  keepAliveInterval?: number; // Seconds, 0 = disabled
  keepAliveCountMax?: number; // Max missed keepalives
}

export interface SshJumpHost {
  host: string;
  port: number;
  username: string;
  authMethod: 'key' | 'password' | 'agent';
  keyPath?: string;
}
```

**Rust persistence:**

```rust
// src-tauri/src/models.rs
#[derive(Serialize, Deserialize, Clone)]
pub struct SshProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub key_path: Option<String>,
    pub proxy_jump: Vec<JumpHost>,
    pub local_port_binding: Option<u16>,
    pub keepalive_interval: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum AuthMethod {
    Key,
    Password,
    Agent,
}
```

### Connection Groups

**Data model:**

```typescript
// src/lib/types.ts
export interface ConnectionGroup {
  id: string;
  name: string;
  color?: string;
  parentGroupId?: string;  // For nesting (max depth: 3)
  order: number;
  collapsed: boolean;
}

export interface SavedConnection {
  id: string;
  name: string;
  groupId?: string;
  colorId: ConnectionColorId;
  dbType: 'postgres' | 'mysql' | 'sqlite';
  host: string;
  port: number;
  database: string;
  username: string;
  maskedUrl: string;
  sshProfileId?: string;  // Link to SSH profile
  status: 'idle' | 'connecting' | 'connected' | 'error';
}
```

### Import/Export

**Export format:**

```json
{
  "version": "1.0",
  "exportedAt": "2026-04-04T12:00:00Z",
  "connections": [
    {
      "name": "Production DB",
      "group": "Production",
      "dbType": "postgres",
      "host": "db.example.com",
      "port": 5432,
      "database": "myapp",
      "username": "admin",
      "sshProfile": "bastion-prod",
      "colorId": "blue"
    }
  ],
  "sshProfiles": [
    {
      "name": "bastion-prod",
      "host": "bastion.example.com",
      "port": 22,
      "username": "deploy",
      "authMethod": "key",
      "keyPath": "~/.ssh/id_rsa"
    }
  ],
  "groups": [
    { "name": "Production", "color": "#ef4444" },
    { "name": "Development", "color": "#22c55e" }
  ]
}
```

**Security note:** Passwords and key passphrases are NEVER exported. Users must re-enter credentials after import.

---

## Implementation Phases

### Phase 1: SSH Foundation

**Goal:** Add SSH tunneling infrastructure with russh.

**Tasks:**
- [ ] Add russh dependencies to `Cargo.toml`
- [ ] Create `src-tauri/src/ssh/` module structure
- [ ] Implement `SshTunnel::create()` with basic key auth
- [ ] Implement `SshTunnel::connect_via_jump()` for ProxyJump
- [ ] Add SSH agent support (`russh_keys::agent`)
- [ ] Add password auth method
- [ ] Implement tunnel cleanup on disconnect
- [ ] Add `create_ssh_tunnel` Tauri command
- [ ] Add `close_ssh_tunnel` Tauri command
- [ ] Write unit tests for tunnel creation

**Success criteria:**
- Can create SSH tunnel to a host using key file
- Can chain through jump hosts
- Tunnels close cleanly on command

**Key files:**
- `src-tauri/Cargo.toml`
- `src-tauri/src/ssh/mod.rs`
- `src-tauri/src/ssh/tunnel.rs`
- `src-tauri/src/ssh/auth.rs`
- `src-tauri/src/commands/ssh.rs`

---

### Phase 2: SSH Config Integration

**Goal:** Parse `~/.ssh/config` and list available hosts.

**Tasks:**
- [ ] Add `russh-config` dependency
- [ ] Create `src-tauri/src/ssh/config_parser.rs`
- [ ] Implement `load_ssh_config()` function
- [ ] Implement `list_ssh_hosts()` Tauri command
- [ ] Handle malformed config gracefully (skip bad entries, log warning)
- [ ] Add SSH host dropdown in frontend
- [ ] Auto-populate SSH form from selected config host

**Success criteria:**
- Can read and parse `~/.ssh/config`
- Hosts appear in dropdown with correct settings
- ProxyJump hosts are parsed correctly
- Graceful handling of missing/invalid config

**Key files:**
- `src-tauri/src/ssh/config_parser.rs`
- `src-tauri/src/commands/ssh.rs`
- `src/lib/components/SshHostDropdown.svelte`

---

### Phase 3: SSH Profiles Management

**Goal:** Create, edit, delete, and reuse SSH profiles.

**Tasks:**
- [ ] Add SSH profile data types to `models.rs`
- [ ] Implement `save_ssh_profile` command
- [ ] Implement `get_ssh_profiles` command
- [ ] Implement `update_ssh_profile` command
- [ ] Implement `delete_ssh_profile` command (check for connections using it)
- [ ] Create `src/lib/components/SshProfileForm.svelte`
- [ ] Create `src/lib/components/SshProfileList.svelte`
- [ ] Create `src/lib/stores/ssh-profiles.svelte.ts`
- [ ] Add SSH profile selection in connection form
- [ ] Store credentials (keys, passwords) in keyring

**Success criteria:**
- Can create SSH profile with key auth
- Profile appears in connection form dropdown
- Deleting profile warns if connections are using it
- Credentials stored securely in keyring

**Key files:**
- `src-tauri/src/models.rs`
- `src-tauri/src/commands/ssh_profiles.rs`
- `src/lib/components/SshProfileForm.svelte`
- `src/lib/components/SshProfileSelector.svelte`
- `src/lib/stores/ssh-profiles.svelte.ts`

---

### Phase 4: Connection Form Redesign

**Goal:** Implement tabbed URL/Manual entry interface.

**Tasks:**
- [ ] Create `src/lib/components/ConnectionForm.svelte` with tabs
- [ ] Create `src/lib/components/UrlInput.svelte` tab
- [ ] Create `src/lib/components/ManualFields.svelte` tab
- [ ] Implement URL parsing on paste/blur
- [ ] Auto-switch to Manual tab after URL parse
- [ ] Keep fields synchronized between tabs
- [ ] Add validation for all fields
- [ ] Add "Test Connection" with tunnel support
- [ ] Handle test for connections with SSH profiles

**Success criteria:**
- URL tab accepts connection strings
- URL parses and populates Manual tab fields
- Manual tab allows field-by-field entry
- Both tabs show same connection (synchronized)
- Test works for direct and SSH-tunneled connections

**Key files:**
- `src/lib/components/ConnectionForm.svelte`
- `src/lib/components/UrlInput.svelte`
- `src/lib/components/ManualFields.svelte`
- `src-tauri/src/commands/connections.rs`

---

### Phase 5: Configurable Credential Storage

**Goal:** Support OS keyring OR master password encryption.

**Tasks:**
- [ ] Create `src-tauri/src/credentials/` module
- [ ] Implement `CredentialBackend` trait
- [ ] Implement `KeyringBackend` (existing keyring logic)
- [ ] Implement `EncryptedVaultBackend` (Argon2id + AES-256-GCM)
- [ ] Add keyring availability detection
- [ ] Create storage settings UI
- [ ] Implement migration between backends
- [ ] Store master password hash (never the password itself)
- [ ] Add vault unlock prompt on app start
- [ ] Add session timeout for vault (lock after X minutes idle)

**Success criteria:**
- OS keyring works on macOS/Windows/Linux (with DE)
- Master password vault works anywhere
- User can switch between storage modes
- Migration preserves all credentials
- Vault auto-locks after configurable timeout

**Key files:**
- `src-tauri/src/credentials/mod.rs`
- `src-tauri/src/credentials/keyring_backend.rs`
- `src-tauri/src/credentials/vault_backend.rs`
- `src/lib/components/StorageSettings.svelte`
- `src/lib/stores/credentials.svelte.ts`

---

### Phase 6: Connection Groups

**Goal:** Organize connections into groups/folders.

**Tasks:**
- [ ] Add group data types to `models.rs`
- [ ] Update `SavedConnection` to include `groupId`
- [ ] Implement group CRUD commands
- [ ] Add group drag-drop in sidebar
- [ ] Implement nested groups (max depth: 3)
- [ ] Add group collapse/expand state
- [ ] Add group color indicators
- [ ] Handle group deletion (move connections to parent or root)

**Success criteria:**
- Can create groups and sub-groups
- Connections can be assigned to groups
- Drag-drop moves connections between groups
- Deleting group doesn't delete connections

**Key files:**
- `src-tauri/src/models.rs`
- `src-tauri/src/commands/groups.rs`
- `src/lib/components/ConnectionGroups.svelte`
- `src/lib/components/GroupItem.svelte`

---

### Phase 7: Import/Export

**Goal:** Import and export connection configurations.

**Tasks:**
- [ ] Design JSON export format
- [ ] Implement `export_connections` command
- [ ] Implement `import_connections` command
- [ ] Add file picker UI for import/export
- [ ] Validate imported data structure
- [ ] Handle duplicate connection names (append suffix or skip)
- [ ] Link SSH profiles by name during import
- [ ] Show import preview with counts
- [ ] Never export passwords/passphrases (security)
- [ ] Add export format versioning

**Success criteria:**
- Can export all connections + profiles to JSON
- Can import from JSON file
- Duplicate names handled gracefully
- Import preview shows what will be added

**Key files:**
- `src-tauri/src/commands/import_export.rs`
- `src/lib/components/ImportDialog.svelte`
- `src/lib/components/ExportDialog.svelte`

---

### Phase 8: Clone & Status Badges

**Goal:** Clone connections and show status indicators.

**Tasks:**
- [ ] Add `clone_connection` command
- [ ] Clone copies all fields except ID
- [ ] Append " (Copy)" to cloned connection name
- [ ] Add status badge states: idle, connecting, connected, error
- [ ] Update status on connect/disconnect/error
- [ ] Add status indicator in sidebar
- [ ] Add connection health check (optional ping)
- [ ] Show last connected timestamp

**Success criteria:**
- Clone creates copy with new ID
- Status badges reflect current connection state
- Error badge shows tooltip with error message

**Key files:**
- `src-tauri/src/commands/connections.rs`
- `src/lib/components/ConnectionItem.svelte`
- `src/lib/components/StatusBadge.svelte`

---

## Acceptance Criteria

### Functional Requirements

- [ ] **AC-01** User can create SSH profile with key-based authentication
- [ ] **AC-02** User can create SSH profile with password authentication
- [ ] **AC-03** User can create SSH profile using SSH agent
- [ ] **AC-04** SSH profiles can be linked to database connections
- [ ] **AC-05** Connection form has tabbed interface: Quick (URL) and Advanced (Fields)
- [ ] **AC-06** URL tab parses connection string and populates fields
- [ ] **AC-07** Manual tab allows field-by-field entry
- [ ] **AC-08** App reads `~/.ssh/config` and lists hosts in dropdown
- [ ] **AC-09** SSH profiles support jump hosts (ProxyJump)
- [ ] **AC-10** SSH profiles support custom local port binding
- [ ] **AC-11** SSH profiles support keep-alive configuration
- [ ] **AC-12** SSH profiles support custom key file paths
- [ ] **AC-13** User can choose between OS keyring and master password storage
- [ ] **AC-14** Master password uses Argon2id + AES-256-GCM encryption
- [ ] **AC-15** User can migrate credentials between storage backends
- [ ] **AC-16** Connections can be organized into groups (nested, max depth 3)
- [ ] **AC-17** User can import connections from JSON file
- [ ] **AC-18** User can export connections to JSON file
- [ ] **AC-19** Import never includes passwords/passphrases (must re-enter)
- [ ] **AC-20** User can clone existing connection (copy with new ID)
- [ ] **AC-21** Connection list shows status badges (idle/connecting/connected/error)
- [ ] **AC-22** Deleting SSH profile warns if connections are using it
- [ ] **AC-23** Deleting group moves connections to parent or root (doesn't delete connections)
- [ ] **AC-24** SSH tunnel establishes before database connection attempt
- [ ] **AC-25** SSH tunnel closes cleanly when connection is closed
- [ ] **AC-26** Vault auto-locks after configurable idle timeout
- [ ] **AC-27** App detects keyring availability and offers fallback

### Non-Functional Requirements

- [ ] SSH tunnel establishment completes within 5 seconds for local networks
- [ ] SSH tunnel establishment completes within 15 seconds for jump host chains
- [ ] Import of 100 connections completes within 3 seconds
- [ ] Status badge updates within 500ms of state change
- [ ] Credential storage supports minimum 100 connections
- [ ] Encrypted vault file size under 1MB for typical usage

### Security Requirements

- [ ] Passwords are never logged or written to temporary files
- [ ] Master password is never stored (only derived key in memory)
- [ ] SSH key passphrases stored in keyring, not config files
- [ ] Import files are validated before processing
- [ ] SSH host keys are verified (user can accept/reject unknown keys)
- [ ] Session credentials cleared from memory on logout/lock
- [ ] Zeroize sensitive data on drop (using `zeroize` crate)

---

## Dependencies & Prerequisites

### Rust (Cargo.toml additions)

```toml
[dependencies]
# SSH Tunneling
russh = { version = "0.50", features = ["aws-lc-rs"] }
russh-keys = "0.50"
russh-config = "0.50"
ssh2-config = "0.5"

# Credential Encryption
argon2 = "0.5"
aes-gcm = "0.10"
zeroize = "1"
rand = "0.8"

# File paths
dirs = "5"

# Existing from MVP
tauri = { version = "2", features = ["protocol-asset"] }
tauri-plugin-store = "2"
sqlx = { version = "0.8", features = [...] }
keyring = "3.3"
dashmap = "6"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
uuid = { version = "1", features = ["v4"] }
url = "2"
```

### Frontend (package.json additions)

No new dependencies required beyond MVP stack.

### Prerequisites

- All MVP prerequisites (Rust 1.77.2+, Node.js 20+, Tauri CLI v2)
- SSH key files accessible to user
- SSH agent running (for agent auth method)
- OS keyring service (for keyring storage mode)

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| russh API breaking changes | Low | High | Pin to 0.50.x; test with lockfile |
| SSH agent not available on Windows | Medium | Medium | Fall back to key file auth; document setup |
| Jump host chain fails mid-way | Medium | High | Implement partial cleanup; clear error messages |
| Keyring unavailable on headless Linux | High | Medium | Auto-detect and offer vault mode; graceful fallback |
| Master password forgotten | Medium | Critical | Clear warning; no recovery mechanism (by design) |
| Import file malicious data | Low | High | Validate schema; reject unknown fields; no code execution |
| SSH config file changes externally | Medium | Low | Re-parse on each profile creation; don't cache |
| Concurrent tunnel creation race | Low | Medium | Use port allocation mutex; check availability first |
| Encrypted vault corruption | Low | Critical | Atomic writes; backup before write; detect corruption |

---

## Open Questions

### Blocking (resolve before implementation)

1. **Tunnel state on app restart:** Should tunnels auto-reconnect, or require manual reconnect?
   - **Recommendation:** Persist last active connection, show "Reconnect?" prompt on app start

2. **Profile edit propagation:** When SSH profile is edited, update active connections using it?
   - **Recommendation:** Prevent edit if connections are active; require disconnect first

3. **Import duplicate handling:** If connection with same name exists, skip or rename?
   - **Recommendation:** Show preview with options: skip, rename, replace

4. **SSH host key verification:** Auto-accept first connection or always prompt?
   - **Recommendation:** Always prompt for unknown hosts; option to "Trust all from this profile"

### Non-Blocking (can be decided during implementation)

5. Maximum nested group depth? (Recommend: 3)
6. Vault session timeout default? (Recommend: 15 minutes, configurable)
7. Keep-alive interval default? (Recommend: 30 seconds)
8. Import file size limit? (Recommend: 1MB / 1000 connections)

---

## Future Considerations

- **SSH certificate authentication** — for enterprise environments
- **Connection templates** — pre-configured settings for common setups
- **Audit logging** — track credential access and connection events
- **Connection scheduling** — automated queries at scheduled times
- **Team sharing** — encrypted sync between team members
- **SSH config write-back** — update `~/.ssh/config` from app

---

## File Structure

```
sqlator/
├── src-tauri/
│   ├── src/
│   │   ├── ssh/
│   │   │   ├── mod.rs
│   │   │   ├── tunnel.rs          # SshTunnel implementation
│   │   │   ├── auth.rs            # Key, password, agent auth
│   │   │   └── config_parser.rs   # ~/.ssh/config parsing
│   │   ├── credentials/
│   │   │   ├── mod.rs
│   │   │   ├── backend.rs         # CredentialBackend trait
│   │   │   ├── keyring_backend.rs # OS keyring implementation
│   │   │   └── vault_backend.rs   # Encrypted vault implementation
│   │   ├── commands/
│   │   │   ├── mod.rs
│   │   │   ├── connections.rs     # Enhanced with SSH support
│   │   │   ├── ssh_profiles.rs    # SSH profile CRUD
│   │   │   ├── groups.rs          # Connection groups
│   │   │   └── import_export.rs   # Import/export operations
│   │   ├── models.rs              # Add SshProfile, ConnectionGroup
│   │   └── state.rs               # Track active tunnels
│   └── Cargo.toml
├── src/
│   ├── lib/
│   │   ├── components/
│   │   │   ├── ConnectionForm.svelte      # Tabbed interface
│   │   │   ├── UrlInput.svelte            # URL tab
│   │   │   ├── ManualFields.svelte        # Manual tab
│   │   │   ├── SshProfileForm.svelte      # SSH profile editor
│   │   │   ├── SshProfileSelector.svelte  # Dropdown in connection form
│   │   │   ├── SshHostDropdown.svelte     # Hosts from ~/.ssh/config
│   │   │   ├── ConnectionGroups.svelte    # Group management
│   │   │   ├── GroupItem.svelte           # Group in sidebar
│   │   │   ├── StatusBadge.svelte         # Connection status indicator
│   │   │   ├── ImportDialog.svelte        # Import UI
│   │   │   ├── ExportDialog.svelte        # Export UI
│   │   │   └── StorageSettings.svelte     # Keyring/vault toggle
│   │   ├── stores/
│   │   │   ├── connections.svelte.ts      # Enhanced with groups
│   │   │   ├── ssh-profiles.svelte.ts     # SSH profile state
│   │   │   ├── groups.svelte.ts           # Group state
│   │   │   └── credentials.svelte.ts      # Storage mode state
│   │   └── types.ts                       # Add SshProfile, ConnectionGroup
│   └── ...
└── ...
```

---

## Sources & References

### Origin Document

- **Origin:** [docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md](docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md)
- Key decisions carried forward:
  - Tauri 2 + Svelte 5 + Rust backend architecture
  - keyring 3.3 for OS keychain (now configurable with vault fallback)
  - tauri-plugin-store for metadata (now includes SSH profiles, groups)
  - DashMap for concurrent state (now includes tunnel tracking)

### External References

- [russh documentation](https://docs.rs/russh/) — Pure Rust SSH client
- [russh-keys](https://docs.rs/russh-keys/) — SSH key and agent handling
- [russh-config](https://docs.rs/russh-config/) — SSH config file parsing
- [keyring crate](https://docs.rs/keyring/) — OS keychain integration
- [Argon2 RFC 9106](https://www.rfc-editor.org/rfc/rfc9106) — Password hashing standard
- [AES-GCM](https://docs.rs/aes-gcm/) — Authenticated encryption
- [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html) — Security best practices

### Key Gotchas

1. **russh requires crypto backend feature** — Use `features = ["aws-lc-rs"]` or `["ring"]`
2. **SSH agent only works on Unix** — Windows requires key file auth
3. **Keyring may fail silently** — Always wrap in detection logic
4. **Zeroize credentials on drop** — Use `zeroize` crate for all sensitive types
5. **SSH config is user-writable** — Expect malformed entries, handle gracefully
6. **Port allocation is racy** — Check availability, use mutex for allocation
7. **Vault file must be atomic** — Write to temp file, then rename
8. **Master password is unrecoverable** — Clear UX warning, no backdoor
