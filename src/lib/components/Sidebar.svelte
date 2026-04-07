<script lang="ts">
  import { onMount } from "svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import ConnectionItem from "./ConnectionItem.svelte";
  import GroupItem from "./GroupItem.svelte";
  import ConnectionForm from "./ConnectionForm.svelte";
  import ThemeToggle from "./ThemeToggle.svelte";
  import type { ConnectionInfo } from "$lib/types";

  let showForm = $state(false);
  let editingConnection = $state<ConnectionInfo | null>(null);
  let rootDragOver = $state(false);
  let creatingGroup = $state(false);
  let newGroupName = $state("");

  onMount(async () => {
    await theme.init();
    await connections.load();
    await groups.load();
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

  // Connections that have no group (ungrouped)
  const ungrouped = $derived(
    connections.list.filter((c) => !c.group_id),
  );

  // Root groups (no parent)
  const rootGroups = $derived(groups.roots);

  function handleRootDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    rootDragOver = true;
  }

  function handleRootDragLeave(e: DragEvent) {
    if (!(e.currentTarget as Element).contains(e.relatedTarget as Node)) {
      rootDragOver = false;
    }
  }

  async function handleRootDrop(e: DragEvent) {
    e.preventDefault();
    rootDragOver = false;
    const connId = e.dataTransfer?.getData("text/plain");
    if (!connId) return;
    const info = await groups.moveConnection(connId, null);
    connections.applyMove(info);
  }

  async function handleCreateGroup() {
    const name = newGroupName.trim();
    if (!name) {
      creatingGroup = false;
      return;
    }
    await groups.create(name);
    newGroupName = "";
    creatingGroup = false;
  }

  function handleGroupNameKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleCreateGroup();
    if (e.key === "Escape") {
      newGroupName = "";
      creatingGroup = false;
    }
  }
</script>

<aside class="sidebar">
  <div class="sidebar-header">
    <span class="sidebar-title">Connections</span>
    <div class="sidebar-actions">
      <ThemeToggle />
      <button
        class="icon-btn"
        onclick={() => (creatingGroup = true)}
        title="New group"
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
          <line x1="12" y1="11" x2="12" y2="17"></line>
          <line x1="9" y1="14" x2="15" y2="14"></line>
        </svg>
      </button>
      <button class="icon-btn" onclick={openNewForm} title="Add connection">
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

  <!-- New group input -->
  {#if creatingGroup}
    <div class="new-group-row">
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="new-group-input"
        bind:value={newGroupName}
        placeholder="Group name…"
        onkeydown={handleGroupNameKeydown}
        onblur={handleCreateGroup}
        autofocus
      />
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="connection-list">
    {#if connections.list.length === 0 && groups.list.length === 0}
      <div class="empty-list">
        <p>No connections yet</p>
        <button class="add-first-btn" onclick={openNewForm}>
          Add your first connection
        </button>
      </div>
    {:else}
      <!-- Ungrouped connections (drop zone to remove from group) -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="ungrouped-zone"
        class:drag-over={rootDragOver}
        ondragover={handleRootDragOver}
        ondragleave={handleRootDragLeave}
        ondrop={handleRootDrop}
      >
        {#each ungrouped as conn (conn.id)}
          <ConnectionItem
            connection={conn}
            onedit={openEditForm}
            dragstart={(e) => {
              e.dataTransfer?.setData("text/plain", conn.id);
              if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
            }}
          />
        {/each}
      </div>

      <!-- Root groups -->
      {#each rootGroups as group (group.id)}
        <GroupItem
          {group}
          depth={0}
          onedit={openEditForm}
        />
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

  .icon-btn {
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

  .icon-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .new-group-row {
    padding: 6px 8px;
    border-bottom: 1px solid var(--color-border);
  }

  .new-group-input {
    width: 100%;
    box-sizing: border-box;
    padding: 5px 8px;
    border: 1px solid var(--color-accent);
    border-radius: 5px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 12px;
    font-weight: 600;
    outline: none;
  }

  .connection-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .ungrouped-zone {
    display: flex;
    flex-direction: column;
    gap: 1px;
    border-radius: 6px;
    transition: background-color 0.1s;
  }

  .ungrouped-zone.drag-over {
    background: color-mix(in oklab, var(--color-accent) 10%, transparent);
    outline: 1px dashed var(--color-accent);
    padding: 4px;
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
