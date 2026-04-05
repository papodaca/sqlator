<script lang="ts">
  import { connections } from "$lib/stores/connections.svelte";
  import { query } from "$lib/stores/query.svelte";

  let statusColor = $derived(
    connections.status === "connected"
      ? "var(--color-success)"
      : connections.status === "error"
        ? "var(--color-error)"
        : "var(--color-text-muted)",
  );

  let statusLabel = $derived(
    connections.status === "connected"
      ? "Connected"
      : connections.status === "connecting"
        ? "Connecting..."
        : connections.status === "error"
          ? "Error"
          : "Disconnected",
  );

  let durationText = $derived(() => {
    const r = query.result;
    if (r.kind === "results" || r.kind === "empty")
      return `${r.durationMs}ms`;
    if (r.kind === "rowsAffected") return `${r.durationMs}ms`;
    return "";
  });
</script>

<div class="toolbar">
  <div class="toolbar-left">
    <div class="status-badge" title={statusLabel}>
      <span class="status-dot" style="background-color: {statusColor}"></span>
      <span class="status-text">{connections.active?.name ?? "—"}</span>
    </div>

    {#if connections.status === "error" && connections.error}
      <button class="retry-btn" onclick={() => connections.retry()}>
        Retry
      </button>
    {/if}
  </div>

  <div class="toolbar-right">
    {#if durationText()}
      <span class="duration">{durationText()}</span>
    {/if}
    {#if query.isExecuting}
      <span class="executing-label">Running...</span>
    {:else}
      <span class="shortcut-hint">Ctrl+Enter to run</span>
    {/if}
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    font-size: 12px;
    min-height: 32px;
  }

  .toolbar-left,
  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status-badge {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .status-text {
    font-weight: 500;
    color: var(--color-text);
  }

  .retry-btn {
    font-size: 11px;
    padding: 2px 8px;
    border: 1px solid var(--color-error);
    border-radius: 4px;
    background: transparent;
    color: var(--color-error);
    cursor: pointer;
  }

  .retry-btn:hover {
    background: var(--color-error);
    color: white;
  }

  .duration {
    color: var(--color-text-muted);
    font-variant-numeric: tabular-nums;
  }

  .executing-label {
    color: var(--color-accent);
    font-weight: 500;
  }

  .shortcut-hint {
    color: var(--color-text-muted);
    opacity: 0.7;
  }
</style>
