<script lang="ts">
  import { getColorHex } from "$lib/constants/colors";
  import { connections } from "$lib/stores/connections.svelte";
  import type { ConnectionTab } from "$lib/types";

  let {
    connectionTabs,
    activeConnectionId,
    onselect,
    onclose,
    onnew,
  }: {
    connectionTabs: ConnectionTab[];
    activeConnectionId: string | null;
    onselect: (connectionId: string) => void;
    onclose: (connectionId: string) => void;
    onnew: () => void;
  } = $props();

  function getConnectionInfo(connectionId: string) {
    return connections.list.find((c) => c.id === connectionId) ?? null;
  }

  function statusColor(tab: ConnectionTab): string {
    if (tab.status === "connected") return "#22c55e";
    if (tab.status === "connecting") return "#f59e0b";
    if (tab.status === "error") return "var(--color-error)";
    return "var(--color-text-muted)";
  }

  function handleMiddleClick(e: MouseEvent, connectionId: string) {
    if (e.button === 1) {
      e.preventDefault();
      onclose(connectionId);
    }
  }
</script>

<div class="connection-tab-bar" role="tablist" aria-label="Connection tabs">
  {#each connectionTabs as tab (tab.connectionId)}
    {@const info = getConnectionInfo(tab.connectionId)}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      class="tab"
      class:active={tab.connectionId === activeConnectionId}
      role="tab"
      aria-selected={tab.connectionId === activeConnectionId}
      title={info?.name ?? tab.connectionId}
      onclick={() => onselect(tab.connectionId)}
      onmousedown={(e) => handleMiddleClick(e, tab.connectionId)}
      tabindex={tab.connectionId === activeConnectionId ? 0 : -1}
    >
      {#if info}
        <span
          class="color-dot"
          style="background-color: {getColorHex(info.color_id)}"
        ></span>
      {/if}
      <span
        class="status-dot"
        style="background-color: {statusColor(tab)}"
        class:pulse={tab.status === "connecting"}
        title={tab.status}
      ></span>
      <span class="tab-label">{info?.name ?? tab.connectionId}</span>
      <button
        class="close-btn"
        tabindex="-1"
        aria-label="Close {info?.name ?? tab.connectionId}"
        onclick={(e) => { e.stopPropagation(); onclose(tab.connectionId); }}
        onmousedown={(e) => e.stopPropagation()}
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <line x1="18" y1="6" x2="6" y2="18"></line>
          <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
      </button>
    </div>
  {/each}

  <button class="new-tab-btn" onclick={onnew} title="Open connection" aria-label="Open new connection">
    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
      <line x1="12" y1="5" x2="12" y2="19"></line>
      <line x1="5" y1="12" x2="19" y2="12"></line>
    </svg>
  </button>
</div>

<style>
  .connection-tab-bar {
    display: flex;
    align-items: stretch;
    overflow-x: auto;
    overflow-y: hidden;
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    scrollbar-width: none;
    min-height: 38px;
  }

  .connection-tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 12px;
    min-width: 120px;
    max-width: 220px;
    height: 38px;
    border-right: 1px solid var(--color-border);
    cursor: pointer;
    flex-shrink: 0;
    color: var(--color-text-muted);
    font-size: 13px;
    user-select: none;
    transition: background-color 0.1s;
  }

  .tab:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .tab:hover .close-btn {
    opacity: 1;
  }

  .tab.active {
    background: var(--color-bg);
    color: var(--color-text);
    font-weight: 500;
    border-bottom: 2px solid var(--color-accent);
  }

  .tab.active .close-btn {
    opacity: 1;
  }

  .tab-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .color-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot.pulse {
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--color-text-muted);
    border-radius: 3px;
    padding: 0;
    opacity: 0;
    transition: opacity 0.1s, background-color 0.1s;
  }

  .close-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .tab.active .close-btn {
    opacity: 1;
  }

  .new-tab-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 12px;
    height: 38px;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--color-text-muted);
    flex-shrink: 0;
    transition: background-color 0.1s, color 0.1s;
  }

  .new-tab-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }
</style>
