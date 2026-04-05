<script lang="ts">
  import { onMount } from "svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import ConnectionItem from "./ConnectionItem.svelte";
  import ConnectionForm from "./ConnectionForm.svelte";
  import ThemeToggle from "./ThemeToggle.svelte";
  import type { ConnectionInfo } from "$lib/types";

  let showForm = $state(false);
  let editingConnection = $state<ConnectionInfo | null>(null);

  onMount(async () => {
    await theme.init();
    await connections.load();
  });

  function openNewForm() {
    editingConnection = null;
    showForm = true;
  }

  function openEditForm(conn: ConnectionInfo) {
    editingConnection = conn;
    showForm = true;
  }

  function closeForm() {
    showForm = false;
    editingConnection = null;
  }
</script>

<aside class="sidebar">
  <div class="sidebar-header">
    <span class="sidebar-title">Connections</span>
    <div class="sidebar-actions">
      <ThemeToggle />
      <button class="add-btn" onclick={openNewForm} title="Add connection">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="12" y1="5" x2="12" y2="19"></line>
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
      </button>
    </div>
  </div>

  <div class="connection-list">
    {#if connections.list.length === 0}
      <div class="empty-list">
        <p>No connections yet</p>
        <button class="add-first-btn" onclick={openNewForm}>
          Add your first connection
        </button>
      </div>
    {:else}
      {#each connections.list as conn (conn.id)}
        <ConnectionItem connection={conn} onedit={openEditForm} />
      {/each}
    {/if}
  </div>
</aside>

{#if showForm}
  <ConnectionForm editing={editingConnection} onclose={closeForm} />
{/if}

<style>
  .sidebar {
    width: 240px;
    min-width: 240px;
    background: var(--color-surface);
    border-right: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px;
    border-bottom: 1px solid var(--color-border);
  }

  .sidebar-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .sidebar-actions {
    display: flex;
    gap: 4px;
  }

  .add-btn {
    background: none;
    border: 1px solid var(--color-border);
    color: var(--color-text-muted);
    cursor: pointer;
    padding: 6px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.15s, color 0.15s;
  }

  .add-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .connection-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .empty-list {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 24px 12px;
    text-align: center;
  }

  .empty-list p {
    font-size: 13px;
    color: var(--color-text-muted);
    margin: 0;
  }

  .add-first-btn {
    font-size: 12px;
    color: var(--color-accent);
    background: none;
    border: none;
    cursor: pointer;
    padding: 4px 0;
  }

  .add-first-btn:hover {
    text-decoration: underline;
  }
</style>
