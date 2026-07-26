# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:ca08a54f -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->

## Conventions & Patterns

### Temporary scratch folder

If you need temp scratch folder for testing or gounding work/plans in how tools or programming packages actually work, use the tmp folder in this projects root directory.

### Async Rust: Never hold a mutex across `.await`

`Arc<Mutex<T>>` shared between async tasks is a deadlock risk. If one task holds the lock across a long-running `.await` (e.g., `ch.wait()`, I/O reads), other tasks needing the same lock will block forever.

**Use instead:** Single-owner + `tokio::select!` + `mpsc` channels. One task owns the resource exclusively; others communicate via message passing.

```rust
// AVOID: shared mutex across async tasks
let channel = Arc::new(Mutex::new(channel));

// PREFER: single owner with select! + mpsc
let (to_ssh_tx, mut to_ssh_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
let (from_ssh_tx, mut from_ssh_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);

tokio::spawn(async move {
    let mut channel = channel;
    loop {
        tokio::select! {
            msg = to_ssh_rx.recv() => { /* send to channel */ }
            msg = channel.wait() => { /* receive from channel */ }
        }
    }
});
```

**Enforce:** Enable `clippy::await_holding_lock` lint in CI. See `docs/solutions/runtime-errors/ssh-tunnel-mutex-deadlock.md` for full analysis.

### Bidirectional I/O: Use `tokio::select!` as the default

When bridging two async streams (SSH tunnel, TCP proxy, WebSocket bridge), `select!` with exclusive ownership is the canonical pattern. Avoid `Arc<Mutex<...Channel/Stream/Socket...>>` — it's a design error for bidirectional I/O.

### SSH Tunneling

SSH tunnels are managed via `core/src/ssh/tunnel.rs`. Key points:
- `forward_stream` uses the `select!` + mpsc pattern (see above)
- Tunnels auto-create on `connect_database` when `ssh_profile_id` is set
- `proxy_jump` is wired up for multi-hop SSH
- Enable debug tracing: `RUST_LOG=sqlator_core::ssh=debug`

## Solved Problems

Documented solutions live in `docs/solutions/`. Check these before investigating similar issues:

- **SSH tunnel mutex deadlock** → `docs/solutions/runtime-errors/ssh-tunnel-mutex-deadlock.md`
  - `Arc<Mutex<Channel>>` caused deadlock; fixed with `select!` + mpsc
