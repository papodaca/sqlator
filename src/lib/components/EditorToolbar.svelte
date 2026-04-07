<script lang="ts">
  import type { ConnectionTab, QueryTab } from "$lib/types";

  let {
    connectionTab,
    queryTab,
  }: {
    connectionTab: ConnectionTab;
    queryTab: QueryTab;
  } = $props();

  const statusColor = $derived(
    connectionTab.status === "connected"
      ? "var(--color-success)"
      : connectionTab.status === "error"
        ? "var(--color-error)"
        : "var(--color-text-muted)",
  );

  const statusLabel = $derived(
    connectionTab.status === "connected"
      ? "Connected"
      : connectionTab.status === "connecting"
        ? "Connecting..."
        : connectionTab.status === "error"
          ? "Error"
          : "Disconnected",
  );

  const durationText = $derived(() => {
    const r = queryTab.result;
    if (r.kind === "results" || r.kind === "empty") return `${r.durationMs}ms`;
    if (r.kind === "rowsAffected") return `${r.durationMs}ms`;
    return "";
  });
</script>

<div class="toolbar">
  <div class="toolbar-left">
    <div class="status-badge" title={statusLabel}>
      <span class="status-dot" style="background-color: {statusColor}"></span>
      <span class="status-text">{statusLabel}</span>
    </div>
  </div>

  <div class="toolbar-right">
    {#if durationText()}
      <span class="duration">{durationText()}</span>
    {/if}
    {#if queryTab.isExecuting}
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
