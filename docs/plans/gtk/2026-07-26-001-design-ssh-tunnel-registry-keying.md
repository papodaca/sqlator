---
title: "design: SSH tunnel registry keying (profile + target)"
type: design
status: active
date: 2026-07-26
related:
  - 2026-07-25-002-refactor-sqlator-service-extraction-plan.md
  - sqlator-9g1.3.4
---

# design: SSH tunnel registry keying (profile + target)

## Problem

Plan 002 resolved the tunnel registry as **keyed by SSH profile id with
refcounting**: multiple DB connections that share an SSH profile should share
one tunnel, created on first user and torn down when the last user disconnects.

That resolution assumed “one tunnel” means one local listener / one SSH session
as a single unit. The current `TunnelHandle` shape in `sqlator-core` makes that
assumption concrete:

```rust
pub struct TunnelHandle {
    pub profile_id: String,
    pub local_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub session: SessionHandle,
    pub cancel_token: CancellationToken,
}
```

One handle = **one** local port forwarding to **one** `target_host:target_port`,
backed by **one** SSH session. There is no API to open a second local forward on
an existing session.

### Why profile-id-only sharing breaks

A single SSH profile is routinely reused for different remote targets:

| Connection | SSH profile | Remote target |
|---|---|---|
| `prod-pg` | `bastion` | `pg.internal:5432` |
| `prod-mysql` | `bastion` | `mysql.internal:3306` |
| `staging-pg` | `bastion` | `pg.staging:5432` |

Under a pure `profile_id` registry key, the second connect either:

1. **Reuses** the first tunnel and connects to the wrong target, or
2. **Replaces** the first tunnel and breaks the first connection’s pool.

Neither is acceptable. Today’s frontends dodge this by keying connect-path
tunnels by **connection id** (and standalone `create_ssh_tunnel` by profile id) —
which “works” for different targets but never shares, and collides when both
paths touch the same map.

## Decision for Phase 1d (interim)

**Registry key = `(ssh_profile_id, target_host, target_port)`**, with refcounting
of connection ids (and a distinct claim for standalone/explicit tunnels) per
entry.

- Same profile **and** same target → share one `TunnelHandle` / one listener.
- Same profile, **different** target → separate tunnels (separate sessions too,
  for now).
- Do **not** key the shared registry by connection id.
- Public lookup for terminal/VTE: `connection_id` → saved connection’s profile +
  resolved target → registry entry → `local_port`.

This preserves correct multi-target behavior while still sharing when two DB
connections truly punch through to the same remote endpoint (the common
“two schemas / two saved URLs, one host:port” case).

Standalone `create_ssh_tunnel` / `close_ssh_tunnel` use the same composite key
(`profile_id` + request target). Closing by profile alone, if retained for API
compat, means “close every registry entry for that profile” (and only when no
refcount users remain, or force-close with clear semantics — implementers must
document which).

## Follow-up: shared session, multiple local forwards

The profile-id-only ideal from plan 002 is still the right end state for
resource use (one SSH handshake / keepalive / jump chain per bastion). Getting
there requires lifting the 1:1 session↔forward coupling in core.

### Sketch (option 3)

1. **Split types in `sqlator-core`:**
   - `SshSession` — authenticated russh `Handle`, cancel/lifecycle, jump-chain
     setup. Registry of sessions keyed by **SSH profile id** (refcount users =
     forwards or higher-level claims).
   - `LocalForward` — `local_port`, `target_host`, `target_port`, listener task
     using `session.channel_open_direct_tcpip(...)`. Many forwards per session.

2. **Service registry:**
   - Sessions: `profile_id` → `ManagedSession { session, forwards, users }`
   - Forwards: `(profile_id, target_host, target_port)` → `local_port` (+ refcount)
   - Connect path acquires session then forward; disconnect drops forward claim,
     then session when no forwards remain.

3. **`SshTunnel::create` compatibility:** keep a façade that creates session +
   single forward for callers not yet migrated, or replace call sites in
   `sqlator-service` only.

4. **Teardown / cancel:** today one `CancellationToken` stops the single
   listener and `close` disconnects the session. Multi-forward needs per-forward
   cancel (drop listener only) vs session-level disconnect (drop all forwards).

5. **Tests to add when doing this:**
   - Two targets, one profile → one SSH connect, two local ports.
   - Drop one forward → other still works; session stays up.
   - Drop last forward → session tears down.
   - Ephemeral `test_*` still always closes its forward (and session if it was
     the sole user).

This follow-up is intentionally **not** part of Phase 1d; 1d ships the composite
key so connect/disconnect/test orchestration can move onto `AppService` without
wrong-target bugs. Track implementation under a dedicated beads issue pointing
at this doc.

## Non-goals here

- Changing russh integration beyond what’s needed for the follow-up sketch.
- Migrating frontends to thin adapters (Phase 1e).
- Fixing web `connect_database` divergence (also 1e, once shared `connect` exists).
