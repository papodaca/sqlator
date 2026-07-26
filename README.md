# SQLator

A multi-database SQL client with a desktop GUI, terminal UI, and web interface. Connect to PostgreSQL, MySQL, MariaDB, SQLite, MSSQL, Oracle, and ClickHouse — with SSH tunnel support.

## Features

- **Multi-database support** — PostgreSQL, MySQL, MariaDB, SQLite, Microsoft SQL Server, Oracle, and ClickHouse
- **SSH tunnels** — Connect through SSH with key, password, or agent auth; supports ProxyJump chains
- **Schema browser** — Tree view of schemas, tables, columns, keys, and DDL
- **SQL editor** — CodeMirror-powered editor with SQL highlighting and one-dark theme
- **Result grid** — Virtualized grid for large result sets with sorting and filtering
- **Connection groups** — Organize connections into color-coded folders (up to 3 levels deep)
- **Docker integration** — Auto-detect and connect to local Docker containers
- **Credential vault** — Encrypted local vault or OS keyring for password storage
- **Multiple frontends** — Desktop app (Tauri), terminal UI (ratatui), and web server (Axum)

## Architecture

```
core/           Rust core library — connection management, query execution, SSH tunnels, credential storage
src-tauri/      Tauri desktop app — SvelteKit frontend + Rust backend
src/            SvelteKit frontend — components, stores, API adapters
tui-app/        Terminal UI — ratatui-based TUI client
web-server/     Web server — Axum + WebSocket for browser access
```

The `sqlator-core` crate is shared across all three frontends. Each frontend has an adapter (`src/lib/api/`) that abstracts the Tauri command layer or WebSocket protocol into a common API.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Frontend | SvelteKit 5, TypeScript, Tailwind CSS 4, CodeMirror 6, xterm.js |
| Desktop | Tauri 2 |
| TUI | ratatui, crossterm, tui-textarea |
| Web Server | Axum, Tower, WebSocket |
| Core | Rust, SQLx, russh, tiberius (MSSQL), oracle-rs, argon2/aes-gcm (vault) |
| Databases | PostgreSQL, MySQL, MariaDB, SQLite, MSSQL, Oracle, ClickHouse |

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/)
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for desktop app

### Install dependencies

```bash
pnpm install
```

### Run the desktop app

```bash
pnpm tauri dev
```

### Run the TUI

```bash
cargo run -p sqlator-tui
```

### Run the web server

```bash
cargo run -p sqlator-web
```

### Development databases

A `docker-compose.yml` is included with pre-configured instances for local development:

```bash
docker compose up -d
```

| Database | Connection String |
|-----------|-----------------|
| PostgreSQL | `postgresql://sqlator:sqlator@localhost:5454/sqlator` |
| MySQL | `mysql://sqlator:sqlator@localhost:3336/sqlator` |
| MariaDB | `mysql://sqlator:sqlator@localhost:3337/sqlator` |
| MSSQL | `mssql://sa:Sqlator123!@localhost:1444/master` |
| Oracle | `oracle://system:Sqlator123!@localhost:1522/FREEPDB1` |
| ClickHouse | `clickhouse://sqlator:sqlator@localhost:8123/sqlator` |

## License

[MIT](LICENSE.txt)
