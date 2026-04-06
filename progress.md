Here is a concise summary of everything completed so far on the feat/enhanced-connection-manager branch.
---
Progress Summary — Enhanced Connection Manager
Branch: feat/enhanced-connection-manager
Commits: 4 feature commits on top of main
Scope: 21 files changed, ~3,700 lines added
---
Phase 1 — SSH Tunneling Foundation ✅
Dependency: russh = "0.60" (latest, up from initial 0.50.4), russh-config = "0.58", keyring = "3", tokio-util, zeroize, thiserror, tracing added to core/Cargo.toml.
core/src/ssh/ (new module)
File	What it does
auth.rs	SshAuthConfig (with zeroize on drop), SshHostConfig, AuthMethod enum (Key / Password / Agent)
error.rs	SshError / SshResult — typed SSH error variants
tunnel.rs	SshTunnel::create() — connects direct or through ProxyJump chain; start_forwarding() — spawns async TCP listener loop forwarding connections via channel_open_direct_tcpip; close() — cancels token + disconnects session
mod.rs	Public re-exports
Tauri commands added: create_ssh_tunnel, close_ssh_tunnel, get_active_tunnels
src-tauri/src/state.rs: AppState gains tunnels: DashMap<String, TunnelHandle> for in-process tunnel tracking.
> Note: SSH agent auth is stubbed — the russh 0.60 agent API requires further adaptation, logged as a TODO.
Phase 2 — SSH Config Integration ✅
**`core/src/ssh/config_parser.rs`** (new)
- `extract_host_aliases()` — parses raw `~/.ssh/config` text, skips wildcards (`*`/`?`) and negation patterns
- `load_ssh_config()` — resolves each alias through `russh_config::parse()`, returns `Vec<HostEntry>` sorted alphabetically
- `HostEntry` — serialisable struct: `alias`, `hostname`, `port`, `user`, `identity_file`, `proxy_jump`
- **2 unit tests** covering alias extraction and full parse round-trip
**Tauri command:** `list_ssh_hosts → CmdResult<Vec<HostEntry>>`
**Frontend**
- `SshHostEntry` type added to `src/lib/types.ts`
- `src/lib/stores/ssh-config.svelte.ts` — reactive store (`load`, `loading`, `error`, `hosts`)
- `src/lib/components/SshHostDropdown.svelte` — searchable dropdown; filters by alias or hostname; shows ProxyJump badge, user, port; lazy-loads on first open; Escape to close
---
src/lib/stores/ssh-profiles.svelte.ts�
; advanced options — new types:
- SshAuthMethodSshProfileSelector.sveltegent)
- SshJumpHost) for local port binding and keep-al
- SshProfile — compact inline list for use insideSavedConJ�;���� extended with optional ssh_profڎ;�@� (#[serde(default)] for backward compat)
core/src/credentials.rs (new) ��k�Rvx�yring wrapper:
- store_credential(profile_id, kind, secret) — writes to "sqlator" service, key "ssh-profile:<id>:<kind>"
- get_cred*�;��� — returns None cleanly on missing entry
- delete_credential / delete_all_credentials — idempotent cleanup
- PasswordBj�R�x�ssphrases never touch disk
core/src/config.rs — ConfigManager extended:
- get/save/update/delete_ssh_profile, get_ssh_profile, connections_using_profile
- delete_s�j�R�x�le returns PROFILE_IN_USE error if any connection references it
Tauri commands: get_ssh_profiles, save_ssh_profile, update_sBj�R�x�le, delete_ssh_profile, connections_using_ssh_profile — secrets route to keyring only, never returned to frontend.
Frontend
- SshProfile, SshProfileConfig, SshAuthMethod, SshJumpHost types in types.ts
- src/lib/stores/ssh-profiles.svelte.ts — reactive CRUD store with byId lookup
- SshProfileForm.svelte — create/edit modal; auth-method tab switcher; imports from ~/.ssh/config via SshHostDropdown; advanced options (<details>) for local port binding and keep-alive
- SshProfileSelector.svelte — compact inline list for use inside ConnectionForm; inline edit button; "+ New profile" shortcut; auto-selects newly created profile
---
Remaining Phases
Phase	Description	Status
4	Connection Form redesign — tabbed URL / Manual entry, Test Connection with SSH	Pending
5	Configurable credential storage — CredentialBackend trait, encrypted vault fallback	Pending
6	Connection groups — nested folders, drag-drop, collapse	Pending
7	Import / Export — JSON round-trip, duplicate handling, import preview	Pending
8	Clone connection, status badges (idle / connecting / connected / error)	Pending