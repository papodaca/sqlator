<script lang="ts">
  import { onMount } from "svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import ConnectionItem from "./ConnectionItem.svelte";
  import GroupItem from "./GroupItem.svelte";
  import ConnectionForm from "./ConnectionForm.svelte";
  import ImportDialog from "./ImportDialog.svelte";
  import ThemeToggle from "./ThemeToggle.svelte";
  import { api } from "$lib/api";
  import type { ConnectionInfo, TableInfo } from "$lib/types";
  import { tabs } from "$lib/stores/tabs.svelte";
  import SchemaBrowser from "./SchemaBrowser.svelte";

  let showForm = $state(false);
  let editingConnection = $state<ConnectionInfo | null>(null);
  let rootDragOver = $state(false);
  let creatingGroup = $state(false);
  let newGroupName = $state("");
  let showImport = $state(false);
  let exportedPath = $state<string | null>(null);
  let showHeaderMenu = $state(false);

  function closeHeaderMenu() { showHeaderMenu = false; }

  async function handleExport() {
    try {
      exportedPath = await api.invoke<string>("export_connections");
      setTimeout(() => { exportedPath = null; }, 6000);
    } catch (e) {
      console.error("Export failed:", e);
    }
  }

  onMount(() => {
    theme.init();
    connections.load();
    groups.load();

    // Allow TabbedEditor's "+" button to open this form
    function handleOpenForm() { openNewForm(); }
    window.addEventListener("sqlator:open-connection-form", handleOpenForm);
    return () => window.removeEventListener("sqlator:open-connection-form", handleOpenForm);
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

  const activeConnectionId = $derived(tabs.activeConnectionId);
  const activeConnectionTab = $derived(tabs.activeConnectionTab);
  const isConnected = $derived(activeConnectionTab?.status === "connected");

  function handleTableOpen(table: TableInfo) {
    if (!activeConnectionId) return;
    tabs.openTableBrowse(activeConnectionId, table);
  }
</script>

<svelte:window onclick={closeHeaderMenu} />

<aside class="sidebar">
  <div class="sidebar-header">
    <span class="sidebar-title">Connections</span>
    <div class="sidebar-actions">
      <ThemeToggle />
      <button class="icon-btn" onclick={openNewForm} title="Add connection">
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19"></line>
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
      </button>
      <!-- ⋯ overflow menu -->
      <div class="header-menu-wrap">
        <button
          class="icon-btn"
          onclick={(e) => { e.stopPropagation(); showHeaderMenu = !showHeaderMenu; }}
          title="More options"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
            <circle cx="5" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="19" cy="12" r="1.5"/>
          </svg>
        </button>
        {#if showHeaderMenu}
          <div class="header-menu" role="menu">
            <button class="header-menu-item" role="menuitem" onclick={(e) => { e.stopPropagation(); creatingGroup = true; showHeaderMenu = false; }}>
              <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
                <line x1="12" y1="11" x2="12" y2="17"></line><line x1="9" y1="14" x2="15" y2="14"></line>
              </svg>
              New group
            </button>
            <div class="header-menu-divider"></div>
            <button class="header-menu-item" role="menuitem" onclick={(e) => { e.stopPropagation(); handleExport(); showHeaderMenu = false; }}>
              <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
                <polyline points="7 10 12 15 17 10"></polyline><line x1="12" y1="15" x2="12" y2="3"></line>
              </svg>
              Export connections
            </button>
            <button class="header-menu-item" role="menuitem" onclick={(e) => { e.stopPropagation(); showImport = true; showHeaderMenu = false; }}>
              <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
                <polyline points="17 8 12 3 7 8"></polyline><line x1="12" y1="3" x2="12" y2="15"></line>
              </svg>
              Import connections
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>

  <!-- Export toast -->
  {#if exportedPath}
    <div class="export-toast">
      <span class="export-msg">Saved to <code>{exportedPath}</code></span>
      <button class="open-btn" onclick={() => api.openPath(exportedPath!)}>Open</button>
    </div>
  {/if}

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
  <div class="connection-list" class:has-schema={isConnected}>
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

  {#if activeConnectionId}
    <SchemaBrowser
      connectionId={activeConnectionId}
      {isConnected}
      onopen={handleTableOpen}
    />
  {/if}
</aside>

{#if showForm}
  <ConnectionForm editing={editingConnection} onclose={closeForm} />
{/if}

{#if showImport}
  <ImportDialog onclose={() => (showImport = false)} />
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
    min-height: 60px;
  }

  .connection-list.has-schema {
    flex: 0 0 auto;
    max-height: 40%;
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

  .header-menu-wrap {
    position: relative;
  }

  .header-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 4px;
    z-index: 50;
    min-width: 170px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .header-menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 13px;
    text-align: left;
    border-radius: 4px;
    color: var(--color-text);
  }

  .header-menu-item:hover {
    background: var(--color-surface-2);
  }

  .header-menu-divider {
    height: 1px;
    background: var(--color-border);
    margin: 4px 0;
  }

  .export-toast {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: color-mix(in oklab, var(--color-accent) 10%, transparent);
    border-bottom: 1px solid color-mix(in oklab, var(--color-accent) 25%, transparent);
    font-size: 11px;
  }

  .export-msg {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-muted);
  }

  .export-msg code {
    font-family: monospace;
  }

  .open-btn {
    flex-shrink: 0;
    background: none;
    border: 1px solid var(--color-accent);
    color: var(--color-accent);
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 11px;
    cursor: pointer;
  }

  .open-btn:hover {
    background: color-mix(in oklab, var(--color-accent) 15%, transparent);
  }
</style>
