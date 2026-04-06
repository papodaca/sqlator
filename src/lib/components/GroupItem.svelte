<script lang="ts">
  import GroupItem from "./GroupItem.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import ConnectionItem from "./ConnectionItem.svelte";
  import type { ConnectionGroup, ConnectionInfo } from "$lib/types";

  let {
    group,
    depth = 0,
    draggedId,
    onedit,
    ondragstart,
  }: {
    group: ConnectionGroup;
    depth?: number;
    draggedId: string | null;
    onedit: (conn: ConnectionInfo) => void;
    ondragstart: (id: string) => void;
  } = $props();

  let showMenu = $state(false);
  let renaming = $state(false);
  let renamingValue = $state(group.name);
  let dragOver = $state(false);
  let showColorPicker = $state(false);
  let confirmDelete = $state(false);

  const GROUP_COLORS = [
    "#ef4444", "#f97316", "#eab308", "#22c55e",
    "#14b8a6", "#3b82f6", "#8b5cf6", "#ec4899",
    "#64748b",
  ];

  const childGroups = $derived(groups.childrenOf(group.id));
  const groupConnections = $derived(
    connections.list.filter((c) => c.group_id === group.id),
  );

  function toggleCollapse() {
    groups.toggleCollapsed(group.id);
  }

  function handleDragOver(e: DragEvent) {
    if (!draggedId) return;
    e.preventDefault();
    dragOver = true;
  }

  function handleDragLeave() {
    dragOver = false;
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    if (!draggedId) return;
    const info = await groups.moveConnection(draggedId, group.id);
    // Update the connections store in place
    connections.applyMove(info);
  }

  async function handleRename() {
    if (!renamingValue.trim() || renamingValue === group.name) {
      renaming = false;
      return;
    }
    await groups.update({ ...group, name: renamingValue.trim() });
    renaming = false;
  }

  function handleRenameKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleRename();
    if (e.key === "Escape") {
      renamingValue = group.name;
      renaming = false;
    }
  }

  async function handleColorSelect(color: string) {
    await groups.update({ ...group, color });
    showColorPicker = false;
    showMenu = false;
  }

  async function handleDelete() {
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => { confirmDelete = false; }, 3000);
      return;
    }
    await groups.remove(group.id);
    // Reload connections so moved connections reflect new group_id
    await connections.load();
    showMenu = false;
  }

  function closeMenu() {
    showMenu = false;
    showColorPicker = false;
    confirmDelete = false;
  }
</script>

<svelte:window onclick={closeMenu} />

<div class="group-item" style="--depth: {depth}">
  <!-- Group header -->
  <div
    class="group-header"
    class:drag-over={dragOver}
    ondragover={handleDragOver}
    ondragleave={handleDragLeave}
    ondrop={handleDrop}
    role="treeitem"
    aria-expanded={!group.collapsed}
  >
    <button class="collapse-btn" onclick={toggleCollapse} title={group.collapsed ? "Expand" : "Collapse"}>
      <svg
        class="chevron"
        class:collapsed={group.collapsed}
        xmlns="http://www.w3.org/2000/svg"
        width="10"
        height="10"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <polyline points="6 9 12 15 18 9"></polyline>
      </svg>
    </button>

    {#if group.color}
      <span class="group-color-dot" style="background-color: {group.color}"></span>
    {:else}
      <svg class="folder-icon" xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
      </svg>
    {/if}

    {#if renaming}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="rename-input"
        bind:value={renamingValue}
        onblur={handleRename}
        onkeydown={handleRenameKeydown}
        autofocus
        onclick={(e) => e.stopPropagation()}
      />
    {:else}
      <span class="group-name">{group.name}</span>
    {/if}

    <span class="group-count">{groupConnections.length}</span>

    <!-- Context menu trigger -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="menu-trigger"
      onclick={(e) => { e.stopPropagation(); showMenu = !showMenu; }}
      onkeydown={(e) => e.key === "Enter" && (showMenu = !showMenu)}
      role="button"
      tabindex="0"
      title="Group options"
    >
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
        <circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/>
      </svg>
    </div>

    {#if showMenu}
      <div class="context-menu" role="menu">
        <button class="menu-item" role="menuitem" onclick={(e) => {
          e.stopPropagation();
          renamingValue = group.name;
          renaming = true;
          showMenu = false;
        }}>Rename</button>
        <button class="menu-item" role="menuitem" onclick={(e) => {
          e.stopPropagation();
          showColorPicker = !showColorPicker;
        }}>Color</button>
        {#if showColorPicker}
          <div class="color-picker" onclick={(e) => e.stopPropagation()}>
            <button
              class="color-swatch no-color"
              onclick={() => handleColorSelect("")}
              title="No color"
            >✕</button>
            {#each GROUP_COLORS as hex}
              <button
                class="color-swatch"
                class:selected={group.color === hex}
                style="background: {hex}"
                onclick={() => handleColorSelect(hex)}
                title={hex}
              ></button>
            {/each}
          </div>
        {/if}
        <button class="menu-item danger" role="menuitem" onclick={(e) => {
          e.stopPropagation();
          handleDelete();
        }}>
          {confirmDelete ? "Confirm Delete?" : "Delete"}
        </button>
      </div>
    {/if}
  </div>

  <!-- Group content (connections + sub-groups) -->
  {#if !group.collapsed}
    <div class="group-content">
      {#each groupConnections as conn (conn.id)}
        <div
          class="draggable-conn"
          draggable="true"
          ondragstart={(e) => {
            e.stopPropagation();
            ondragstart(conn.id);
          }}
        >
          <ConnectionItem connection={conn} onedit={onedit} />
        </div>
      {/each}

      {#if depth < 2}
        {#each childGroups as child (child.id)}
          <GroupItem
            group={child}
            depth={depth + 1}
            {draggedId}
            {onedit}
            {ondragstart}
          />
        {/each}
      {/if}

      {#if groupConnections.length === 0 && childGroups.length === 0}
        <div class="empty-group">Drop connections here</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .group-item {
    padding-left: calc(var(--depth, 0) * 12px);
  }

  .group-header {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 8px 5px 4px;
    border-radius: 6px;
    cursor: pointer;
    position: relative;
    transition: background-color 0.1s;
    user-select: none;
  }

  .group-header:hover {
    background: var(--color-surface-2);
  }

  .group-header.drag-over {
    background: color-mix(in oklab, var(--color-accent) 15%, transparent);
    outline: 1px dashed var(--color-accent);
  }

  .collapse-btn {
    background: none;
    border: none;
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
    color: var(--color-text-muted);
    border-radius: 3px;
    flex-shrink: 0;
  }

  .collapse-btn:hover {
    background: var(--color-border);
  }

  .chevron {
    transition: transform 0.15s;
  }

  .chevron.collapsed {
    transform: rotate(-90deg);
  }

  .group-color-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .folder-icon {
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .group-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rename-input {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
    padding: 1px 4px;
    border: 1px solid var(--color-accent);
    border-radius: 3px;
    background: var(--color-bg);
    color: var(--color-text);
    outline: none;
    min-width: 0;
  }

  .group-count {
    font-size: 11px;
    color: var(--color-text-muted);
    opacity: 0.6;
    flex-shrink: 0;
  }

  .menu-trigger {
    opacity: 0;
    padding: 2px 4px;
    border-radius: 4px;
    cursor: pointer;
    color: var(--color-text-muted);
    display: flex;
    align-items: center;
    flex-shrink: 0;
    transition: opacity 0.1s, background-color 0.1s;
  }

  .group-header:hover .menu-trigger {
    opacity: 1;
  }

  .menu-trigger:hover {
    background: var(--color-border);
  }

  .context-menu {
    position: absolute;
    top: calc(100% + 2px);
    right: 4px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 4px;
    z-index: 50;
    min-width: 110px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .menu-item {
    display: block;
    width: 100%;
    padding: 5px 10px;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 12px;
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

  .color-picker {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 4px 10px 6px;
  }

  .color-swatch {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition: transform 0.1s;
  }

  .color-swatch:hover {
    transform: scale(1.2);
  }

  .color-swatch.selected {
    border-color: var(--color-text);
  }

  .color-swatch.no-color {
    background: var(--color-surface-2);
    border-color: var(--color-border);
    font-size: 9px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-text-muted);
  }

  .group-content {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .draggable-conn {
    cursor: grab;
  }

  .draggable-conn:active {
    cursor: grabbing;
    opacity: 0.6;
  }

  .empty-group {
    font-size: 11px;
    color: var(--color-text-muted);
    padding: 6px 12px;
    font-style: italic;
    opacity: 0.6;
  }
</style>
