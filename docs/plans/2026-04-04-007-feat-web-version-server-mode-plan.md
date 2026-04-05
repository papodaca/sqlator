---
title: "feat: Web Version / Server Mode (`sqlator web`)"
type: feat
status: active
date: 2026-04-04
origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md
---

# ✨ Web Version / Server Mode (`sqlator web`)

## Overview

Introduce a new CLI command to SQLator that spins up a lightweight, fully-featured web server (`sqlator web`). This web version exposes the identical Svelte 5 interface used by the Tauri desktop app, allowing users to manage databases entirely from a browser. It supports two primary modes:
1. **Multi-Database Admin (Default):** Functions just like the desktop app, managing multiple connections, SSH tunnels, and profiles, with all connection info securely saved on the server side.
2. **Single-Database Admin (`-c ./path/to/config.yml`):** Bypasses the connection manager to instantly provide an admin interface for a single database specified via a config file—perfect for quick server-side administration.

## Problem Statement

While native desktop apps and TUIs serve local development workflows beautifully, they lack flexibility when users need to remotely manage a database without exposing its port to the public internet, or when setting up a quick, temporary internal admin tool on a remote server. By serving the SQLator frontend over HTTP and proxying requests to the exact same `core/` library, we unlock a powerful, low-latency web admin interface with zero duplicated business logic.

## Proposed Solution

Expand the Cargo workspace to include a `web-server/` crate built with a high-performance framework (like `axum`). 
Abstract the Svelte frontend's data-fetching layer to conditionally use standard `fetch()` and `WebSocket` (or Server-Sent Events) APIs instead of `tauri::ipc::invoke`. 

The `web-server/` will bundle the compiled Svelte SPA as static assets and expose REST/WS endpoints that directly bridge to the existing `core/` API methods.

## Technical Approach

### Architecture

The project will expand to include a web server boundary that proxies HTTP/WS requests into the `core/` library.

```text
┌─────────────────────────────────────────────────────────┐
│                   Svelte 5 Frontend (SPA)               │
│  ┌───────────────────────────────────────────────────┐  │
│  │  API Adapter Layer (Abstracts Tauri vs Web)       │  │
│  │  - invoke_command() -> fetch() or Tauri IPC       │  │
│  │  - stream_query()   -> WebSocket or Tauri Channel │  │
│  └─────────────────────────┬─────────────────────────┘  │
└────────────────────────────┼────────────────────────────┘
                             │ HTTP / WebSocket
┌────────────────────────────▼────────────────────────────┐
│           Axum Web Server (`web-server/`)               │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Route Handlers (REST & WebSocket Endpoints)      │  │
│  │  - Translates JSON to Core library structs        │  │
│  │  - Bridges Core MPSC channels to WebSocket frames │  │
│  └──────────┬─────────────────────────┬──────────────┘  │
└─────────────┼─────────────────────────┼─────────────────┘
              │                         │
┌─────────────▼─────────────────────────▼─────────────────┐
│           Core Library (Pure Rust)                       │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Core API (Connection Manager, Query Execution)   │  │
│  └──────────┬─────────────────────────┬──────────────┘  │
│             │                         │                 │
│  ┌──────────▼────────┐    ┌───────────▼─────────────┐  │
│  │  sqlx AnyPool     │    │  Config Manager (fs)    │  │
│  │  (Postgres/MySQL/ │    │  connections.json       │  │
│  │  SQLite)          │    │  (name, host, color)    │  │
│  └───────────────────┘    └─────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Implementation Phases

#### Phase 1: Frontend IPC Abstraction
**Goal:** Decouple the Svelte 5 frontend from `@tauri-apps/api`.
* **Tasks:**
  - Create `src/lib/api/client.ts` defining an `ApiClient` interface.
  - Implement `TauriClient` (using existing `invoke` and `Channel`).
  - Implement `WebClient` (using `fetch` for commands and `WebSocket` or `EventSource` for streaming query results).
  - Use environment variables (e.g., `VITE_TARGET=web`) to conditionally inject the correct client at build time.
* **Success Criteria:** The Svelte app can be compiled independently of Tauri (`pnpm build`) and functional API calls can be routed to a mock or local HTTP server.

#### Phase 2: Rust Web Server (`web-server/`)
**Goal:** Expose the `core/` library via a REST API.
* **Tasks:**
  - Create a new Cargo workspace member `web-server` using `axum` and `tokio`.
  - Create REST endpoints matching the `core/` capabilities: `/api/connections`, `/api/connections/:id`, `/api/schema`, `/api/test`.
  - Serve the built Svelte SPA static assets from `/`.
  - Wire the Axum endpoints to the `core` library functions.
* **Success Criteria:** Users can run `cargo run -p web-server` and manage their saved connections via the browser at `http://localhost:3000`.

#### Phase 3: Query Streaming over WebSockets
**Goal:** Support large, non-blocking result sets in the web context.
* **Tasks:**
  - Create a `/api/query` WebSocket endpoint in Axum.
  - Bridge the `core/` library's `tokio::sync::mpsc::Receiver<QueryEvent>` to the WebSocket connection, sending JSON frames.
  - Update the `WebClient` in the frontend to connect to the WebSocket, buffer row events, and flush them to the `$state` exactly as the Tauri Channel implementation does.
* **Success Criteria:** Executing a query in the browser streams results seamlessly to the Enhanced Grid without freezing the UI.

#### Phase 4: Single-DB Config Mode (`-c`)
**Goal:** Support the isolated quick-admin mode.
* **Tasks:**
  - Add CLI argument parsing to `web-server` (e.g., using `clap`).
  - If `-c <path>` is provided, read the database connection URL and bypass the `core/config` manager entirely.
  - Create a virtual "locked" connection ID.
  - Update the frontend: If the server signals it is in "Single-DB Mode", completely hide the Sidebar/Connection Manager and immediately transition the UI into the Connected Workspace for that single database.
* **Success Criteria:** Running `sqlator web -c ./prod-db.yml` instantly drops the user into an admin interface for that specific database, with no options to add or switch to other databases.

## Alternative Approaches Considered

- **Server-Sent Events (SSE) vs WebSockets:** Tauri channels simulate streaming. SSE is unidirectional (Server -> Client) and fits the `QueryEvent` streaming model perfectly, but WebSockets provide a persistent, bidirectional connection which is better suited if we eventually need query cancellation or interactive editable datagrid commits over the same pipe. *Decision: WebSockets for querying, REST for standard state fetching.*
- **Authentication:** Exposing a database admin panel over the web is inherently dangerous. We considered adding a built-in login screen. *Decision: For MVP, the web version binds to `127.0.0.1` by default. If exposed, we strongly recommend users place it behind a reverse proxy (e.g., Nginx with Basic Auth, or Cloudflare Access). We will add a simple `--auth user:pass` basic auth flag to the CLI for minimal protection.*

## System-Wide Impact

### Interaction Graph
1. User starts server: `sqlator web -c db.yml` -> Axum server boots -> Parses `db.yml` -> Binds connection pool in `core`.
2. User loads `http://localhost:3000` -> Svelte SPA loads -> Fetches `/api/config` -> Receives "Single-DB mode".
3. SPA mounts the Tabbed Editor and Schema Browser, hiding the Sidebar.
4. User writes query -> `WebClient` opens WebSocket to `/api/query` -> Axum executes query via `core` -> Streams MPSC results via WS frames -> SPA updates grid.

### Security & State Risks
* **Credential Storage:** In Multi-DB mode, connections are saved on the server side using the `core` credential manager. If the server lacks an OS keyring (headless Linux), it will fall back to the Encrypted Vault backend (see Plan 002).
* **Multi-Tenant Risk:** This is **not** a multi-tenant application. If multiple users connect to the same web instance, they will share the same `DashMap` of connection pools and identical configuration state. The UI will reflect state globally for that server instance.

### API Surface Parity
The `core/` library must remain entirely agnostic to *how* it is called.
- `core::db::execute_query` takes a standard `mpsc::Sender`.
- The `tauri-app` consumes it via `tauri::ipc::Channel`.
- The `web-server` consumes it via `axum::extract::ws::WebSocket`.

## Acceptance Criteria

### Functional Requirements
- [ ] Running `sqlator web` starts an HTTP server serving the SQLator frontend on a specified port (default 3000).
- [ ] The web interface matches the exact capabilities of the Tauri app (Schema Browser, Editable Grid, Tabs).
- [ ] The Svelte frontend abstracts API calls to work over HTTP/WS when built for the web.
- [ ] Running `sqlator web -c <config.yml>` locks the UI to a single database.
- [ ] In Single-DB mode, the connection manager sidebar is hidden or disabled.
- [ ] Query execution streams large result sets via WebSockets (or SSE) without buffering the entire result in server memory.
- [ ] The server accepts an optional `--auth username:password` flag for basic HTTP authentication.

### Non-Functional Requirements
- [ ] Single-page application static assets are bundled directly into the Rust binary to ensure a single-executable deployment.
- [ ] WebSocket streaming performance should handle 1000-row chunks with minimal latency compared to the native Tauri IPC.

## Dependencies & Prerequisites

### Rust (Cargo.toml additions for `web-server/`)
```toml
[dependencies]
core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["ws"] }
tower-http = { version = "0.5", features = ["fs", "cors", "auth"] }
clap = { version = "4.5", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
include_dir = "0.7" # To bundle Svelte dist assets into the Rust binary
```

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Unauthorized Access | High | Critical | Bind to `127.0.0.1` by default. Implement optional Basic Auth via `tower-http`. Heavily document the need for a reverse proxy in production deployments. |
| Streaming Overhead | Medium | Medium | Serializing thousands of rows to JSON for WebSocket frames can be CPU intensive. Ensure chunking is respected and limit maximum rows per frame. |
| Tauri-specific logic in Frontend | High | High | Must rigorously audit `src/` to ensure absolutely no `@tauri-apps/api` imports exist outside of the dedicated `TauriClient` adapter implementation. |

## Sources & References

### Origin
- **Origin document:** [docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md](docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md)
  - Key decision carried forward: *Architecture decoupled into workspaces.* The addition of `web-server/` seamlessly plugs into the `core/` library established by the MVP.
  - Key decision carried forward: *Streaming query results.* Bypassing full memory buffering remains critical, adapted here for WebSockets.
