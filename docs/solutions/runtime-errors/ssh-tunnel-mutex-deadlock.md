---
title: SSH tunnel mutex deadlock causing indefinite connection hang
category: runtime-errors
date: 2026-04-09
tags: [ssh, deadlock, mutex, tokio, concurrency, mpsc]
component: core/src/ssh/tunnel.rs
symptoms: SSH tunnel connections hang indefinitely; the writer task holding a Mutex lock across ch.wait() permanently blocks the reader task from acquiring the lock to send the client startup message.
problem_type: concurrency
severity: critical
---

## Problem

SSH tunnel connections hung permanently with no data flowing in either direction. Any SQL client connecting through the tunnel would stall indefinitely on startup — the connection was established at the TCP level but no SQL handshake ever completed.

## Investigation

The tunnel connected successfully (SSH authentication passed, channel opened), so the issue wasn't with SSH setup. The hang occurred in `forward_stream`, which bridges a TCP socket to an SSH channel. Tracing the two spawned tasks revealed:

1. The writer task called `ch.wait().await` while holding a `Mutex<Channel>` lock — this is a potentially infinite await
2. The reader task needed the same mutex to call `ch.data()` to send the SQL startup message
3. The SSH server was waiting for the startup message before sending any data back
4. So the writer would never return from `ch.wait()`, and the reader could never acquire the lock — a classic deadlock

## Root Cause

A single `Arc<Mutex<Channel>>` was shared between two concurrent tasks. The writer task held the mutex lock across `ch.wait()`, a long-running async operation that blocks until the SSH server sends data. But the SSH protocol requires the client to transmit first, so the server will never send data until the reader task sends the startup message — which it can't do because the mutex is locked by the waiting writer. The lock is only released *after* `ch.wait()` returns, which requires the very data that the lock prevents from being sent. This creates an unbreakable circular dependency.

## Solution

Replace the shared `Arc<Mutex<Channel>>` with a single task that owns the SSH channel exclusively, using `tokio::select!` to multiplex both directions, bridged by `tokio::sync::mpsc` channels so the reader and writer tasks never contend on the same lock.

**Before (deadlocked):**
```rust
let channel = Arc::new(Mutex::new(channel));
let channel_clone = channel.clone();

// Reader task - needs mutex to call ch.data()
tokio::spawn(async move {
    let mut buf = vec![0u8; 8192];
    loop {
        match stream_reader.read(&mut buf).await {
            Ok(n) => {
                let ch = channel_clone.lock().await; // BLOCKED by writer!
                ch.data(&buf[..n]).await;
            }
            // ...
        }
    }
});

// Writer task - holds mutex across ch.wait()
tokio::spawn(async move {
    loop {
        let mut ch = channel.lock().await; // Acquires lock
        let Some(msg) = ch.wait().await else { break }; // Holds lock during wait!
        drop(ch); // Only releases after data arrives
        // ...
    }
});
```

**After (fixed):**
```rust
let (to_ssh_tx, mut to_ssh_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
let (from_ssh_tx, mut from_ssh_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);

// Single task owns the channel, uses select! for both directions
tokio::spawn(async move {
    let mut channel = channel; // Channel moved here, no mutex needed
    loop {
        tokio::select! {
            msg = to_ssh_rx.recv() => {
                match msg {
                    Some(data) => {
                        if channel.data(data.as_ref()).await.is_err() { break; }
                    }
                    None => break,
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if from_ssh_tx.send(data.to_vec()).await.is_err() { break; }
                    }
                    Some(ChannelMsg::Eof) | None => break,
                    _ => {}
                }
            }
        }
    }
});

// Reader task - sends to mpsc channel instead of mutex
tokio::spawn(async move {
    let mut buf = vec![0u8; 8192];
    loop {
        match stream_reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to_ssh_tx.send(buf[..n].to_vec()).await.is_err() { break; }
            }
        }
    }
});

// Writer task - reads from mpsc channel instead of mutex
tokio::spawn(async move {
    loop {
        match from_ssh_rx.recv().await {
            Some(data) => {
                if stream_writer.write_all(&data).await.is_err() { break; }
            }
            None => break,
        }
    }
});
```

## Key Insight

Never hold a mutex across a long-running async wait — if another task needs the same lock to produce the data that would unblock the wait, you've created an unbreakable deadlock; instead, give exclusive ownership to a single task and use message-passing channels to communicate.

## Prevention Strategies

1. **Never hold a mutex lock across `.await` points.** `Mutex::lock()` creates a guard that spans an `.await`, and if another task needs the same lock to make progress, you have a deadlock. Use `tokio::sync::Mutex` only when you must hold the lock across an `.await` — and even then, design so only one task ever contends for it.

2. **Prefer single-owner + message passing over shared mutable state.** One task owns the resource, others communicate via `mpsc` channels. When two async tasks need to interact with the same I/O resource, a `select!` loop owning the resource with channel-based command/query messages eliminates contention entirely.

3. **Audit lock scopes before introducing `Arc<Mutex<T>>`.** `Arc<Mutex<T>>` is a code smell in async Rust. Before reaching for it, ask: do both sides truly need mutable access? Could one side own the data and receive commands? If you must use it, the lock guard's lifetime must not span an `.await`. Enable `clippy::await_holding_lock` to catch this at compile time.

4. **Model ownership and data flow before coding.** Before implementing concurrent async code, diagram which task owns which resource and how data flows between them. If two arrows point at the same mutable resource, you have a potential deadlock.

5. **Use `tokio::select!` as the default pattern for bidirectional I/O.** When bridging two async streams, `select!` gives one task exclusive ownership of both sides, removing any need for shared state.

## Testing Suggestions

### Unit Tests

- **Timeout-based deadlock detection**: Wrap operations in `tokio::time::timeout(Duration::from_secs(5), tunnel_op)`. If a deadlock exists, the test fails with a timeout rather than hanging forever.
- **Channel backpressure test**: When using the mpsc-bridge pattern, verify that if the consumer stalls, the producer receives `SendError` or backpressure rather than blocking indefinitely.

### Integration Tests

- **SSH tunnel liveness probe**: Open an SSH tunnel, send data bidirectionally, assert both directions complete within a timeout.
- **Concurrent session stress test**: Open N simultaneous SSH tunnels and exercise all concurrently. Mutex contention bugs surface more reliably under concurrency.

### Tools

- **`clippy::await_holding_lock`**: Catches mutex guards held across `.await` at compile time. This alone would have prevented the original bug.
- **`tokio-console`**: Shows per-task state (idle, running, waiting), making deadlocks visually obvious.
- **`tracing` spans**: Add `#[tracing::instrument]` to `forward_stream` and lock acquisition points to trace exactly where each task is stuck.

## Code Review Checklist

- [ ] Is any mutex lock guard held across an `.await`? Search for `let guard = x.lock().await; ...some_async_op().await...` where the guard is still in scope. Enable `clippy::await_holding_lock`.
- [ ] Do two or more async tasks contend for the same `Arc<Mutex<T>>`? If yes, verify no lock-holding task performs I/O while holding the lock. Consider single-owner + message passing.
- [ ] Is there a bidirectional I/O pattern? `tokio::select!` with exclusive ownership should be the default, not shared mutable access. Flag `Arc<Mutex<...Channel/Stream/Socket...>>` as a likely design error.
- [ ] Can the code make progress if any single task stalls? If Task A blocks on Task B, and Task B blocks on Task A (even indirectly through a lock), you have a deadlock.
- [ ] Are there timeout/wrapper guards on all I/O-bound operations? Every network operation should have a timeout or cancellation mechanism.

## Related

- **Commit `ae33517`**: `fix(ssh): auto-create tunnel on connect and fix forward_stream deadlock`
- **`core/src/ssh/tunnel.rs`**: Fixed file with `select!` + mpsc pattern (line 142 comment)
- **`core/src/ssh/command.rs`**: Related SSH module using `channel.wait()` in a simpler single-direction pattern
- **`docs/plans/2026-04-04-002-feat-enhanced-connection-manager-plan.md`**: Original SSH tunneling plan (may contain pre-fix code sketch)
