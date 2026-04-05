---
date: 2026-04-04
topic: docker-container-database-connections
---

# Docker Container Database Connections

## Problem Frame

Users with databases running inside Docker containers on remote servers face a connectivity gap. Many containers don't expose database ports to the host, making direct connections impossible. Today, users manually SSH into servers and run `docker exec` to interact with databases, losing the benefits of a SQL client (schema browsing, query history, result visualization, connection pooling).

This feature enables Sqlator to connect to databases inside Docker containers via SSH tunneling to the container's internal IP, providing full SQL client functionality without requiring server modifications.

## Requirements

- R1. User can create a connection that targets a Docker container by name on a remote server
- R2. Sqlator discovers the container's internal IP address via SSH + Docker CLI
- R3. Sqlator establishes an SSH tunnel from local machine to the container's internal IP
- R4. Database connection flows through the SSH tunnel, providing full SQL client features
- R5. Connection works on read-only servers (no modifications required)
- R6. User manually specifies container name initially; auto-discovery is a future enhancement
- R7. When Sqlator runs on the same server as Docker, user can choose SSH tunnel OR direct Docker socket access
- R8. Container connections are saved as regular connections (same as direct database connections)
- R9. Setup uses a guided flow/wizard to discover container details and configure the connection

## Success Criteria

- Can connect to a PostgreSQL container that has no exposed ports on the host
- Can connect to a MySQL container that has no exposed ports on the host
- Connection provides full SQL client functionality (schema browser, query execution, data grid)
- Works without modifying any docker-compose.yml or server configuration
- Connection establishes within 10 seconds for typical setups

## Scope Boundaries

- NOT auto-discovering all database containers on a server (manual selection first)
- NOT mounting sockets from containers to host (requires docker-compose changes)
- NOT using `docker exec` for query execution (need real connection for full features)
- NOT supporting Windows containers (Linux containers only)
- NOT managing Docker containers (start/stop/create) - only connecting to existing ones

## Key Decisions

- **Primary approach: SSH tunnel to container internal IP.** Docker containers have internal IPs accessible from the host. SSH tunneling to these IPs requires no server modifications.
- **Manual container selection first.** User provides container name; Sqlator handles IP discovery and tunneling. Auto-discovery of available containers is a follow-up feature.
- **Support both remote and same-server scenarios.** Remote uses SSH tunnel. Same-server can use SSH tunnel OR direct access via Docker socket.
- **Read-only server support is required.** The solution must work without installing any software on the server.
- **Wizard-based setup, normal connection storage.** A guided flow helps discover and configure container connections. Once configured, they're stored as regular connections alongside direct database connections.

## Dependencies / Assumptions

- User has SSH access to the server running Docker
- Docker CLI is available on the server (for `docker inspect`)
- User knows the container name or can discover it
- Database credentials are known (username, password, database name)
- Containers use default database ports internally (5432 for Postgres, 3306 for MySQL) OR user specifies custom port

## Outstanding Questions

### Deferred to Planning

- [Affects R2][Technical] How to handle container IP changes? Containers get new IPs on restart. Need to either: (a) re-discover on each connection, (b) cache and verify, or (c) let connection fail and prompt user.
- [Affects R3][Technical] Should we use russh for tunneling (consistent with existing SSH plan) or shell out to system SSH? russh gives more control but system SSH is battle-tested.
- [Affects R7][Technical] For direct Docker socket access on same-server, should we use the Docker API to inspect containers, or shell out to `docker` CLI?
- [Affects R4][Technical] How to handle container databases with non-standard ports? Need to discover or let user specify.
- [Affects R1][Needs research] Can we detect database type (Postgres vs MySQL) from container inspection, or does user need to specify?

## Next Steps

→ `/ce:plan` for structured implementation planning
