<script module lang="ts">
  // Shared across all ConnectionItem instances — only one menu open at a time.
  let openMenuId = $state<string | null>(null);
</script>

<script lang="ts">
  import { getColorHex } from "$lib/constants/colors";
  import { connections } from "$lib/stores/connections.svelte";
  import { tabs } from "$lib/stores/tabs.svelte";
  import type { ConnectionInfo } from "$lib/types";

  let {
    connection,
    onedit,
    dragstart = undefined,
  }: {
    connection: ConnectionInfo;
    onedit: (conn: ConnectionInfo) => void;
    /** If provided, the item becomes draggable. Use dataTransfer in the handler. */
    dragstart?: (e: DragEvent) => void;
  } = $props();

  const showMenu = $derived(openMenuId === connection.id);
  const connectionTab = $derived(tabs.connectionTabs.find((t) => t.connectionId === connection.id));
  const isActive = $derived(tabs.activeConnectionId === connection.id);
  const connectionStatus = $derived(connectionTab?.status ?? "disconnected");
  let confirmDelete = $state(false);

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    openMenuId = connection.id;
  }

  async function handleClick() {
    const isNew = tabs.openConnection(connection.id);
    if (isNew) {
      tabs.setConnectionStatus(connection.id, "connecting");
      try {
        await connections.connectRaw(connection.id);
        tabs.setConnectionStatus(connection.id, "connected");
      } catch (e) {
        tabs.setConnectionStatus(connection.id, "error", String(e));
      }
    }
  }

  async function handleClone() {
    closeMenu();
    await connections.clone(connection.id);
  }

  async function handleDelete() {
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => {
        confirmDelete = false;
      }, 3000);
      return;
    }
    await connections.remove(connection.id);
  }

  function closeMenu() {
    openMenuId = null;
  }
</script>

<svelte:window onclick={closeMenu} />

<div class="connection-item-wrapper">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <button
    class="connection-item"
    class:active={connections.activeId === connection.id}
    draggable={dragstart !== undefined}
    onclick={handleClick}
    oncontextmenu={handleContextMenu}
    ondragstart={dragstart}
  >
    <span
      class="color-dot"
      style="background-color: {getColorHex(connection.color_id)}"
    ></span>
    <div class="connection-info">
      <span class="connection-name">{connection.name}</span>
      <span class="connection-detail"
        >{connection.db_type} &middot; {connection.host}</span
      >
    </div>
    {#if isActive && connectionStatus !== "disconnected"}
      <span
        class="status-dot status-{connectionStatus}"
        title={connectionStatus === "error" ? (connectionTab?.error ?? "Connection error") : connectionStatus}
      ></span>
    {/if}
  </button>

  {#if showMenu}
    <div class="context-menu" role="menu">
      <button
        class="menu-item"
        role="menuitem"
        onclick={(e: MouseEvent) => {
          e.stopPropagation();
          closeMenu();
          onedit(connection);
        }}
      >
        Edit
      </button>
      <button
        class="menu-item"
        role="menuitem"
        onclick={(e: MouseEvent) => {
          e.stopPropagation();
          handleClone();
        }}
      >
        Clone
      </button>
      <button
        class="menu-item danger"
        role="menuitem"
        onclick={(e: MouseEvent) => {
          e.stopPropagation();
          handleDelete();
        }}
      >
        {confirmDelete ? "Confirm Delete?" : "Delete"}
      </button>
    </div>
  {/if}
</div>

<style>
  .connection-item-wrapper {
    position: relative;
  }

  .connection-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    border: none;
    background: transparent;
    cursor: pointer;
    border-radius: 6px;
    transition: background-color 0.1s;
    text-align: left;
    color: var(--color-text);
  }

  .connection-item:hover {
    background: var(--color-surface-2);
  }

  .connection-item[draggable="true"] {
    cursor: grab;
  }

  .connection-item[draggable="true"]:active {
    cursor: grabbing;
    opacity: 0.7;
  }

  .connection-item.active {
    background: var(--color-accent);
    color: white;
  }

  .color-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .connection-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: hidden;
    min-width: 0;
  }

  .status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    margin-left: auto;
  }

  .status-dot.status-connecting {
    background: #f59e0b;
    animation: pulse 1s ease-in-out infinite;
  }

  .status-dot.status-connected {
    background: #22c55e;
  }

  .status-dot.status-error {
    background: var(--color-error);
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }

  .connection-name {
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .connection-detail {
    font-size: 11px;
    opacity: 0.65;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .context-menu {
    position: absolute;
    top: 100%;
    left: 12px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 4px;
    z-index: 50;
    min-width: 120px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .menu-item {
    display: block;
    width: 100%;
    padding: 6px 12px;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 13px;
    text-align: left;
    border-radius: 4px;
    color: var(--color-text);
  }

  .menu-item:hover {
    background: var(--color-surface-2);
  }

  .menu-item.danger {
    color: var(--color-error);
  }
</style>
