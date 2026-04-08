<script lang="ts">
  import type { TableInfo } from "$lib/types";
  import { schemaStore } from "$lib/stores/schema.svelte";
  import SchemaNode from "./SchemaNode.svelte";

  let {
    connectionId,
    onopen,
  }: {
    connectionId: string;
    onopen: (table: TableInfo) => void;
  } = $props();

  const schemaState = $derived(schemaStore.getState(connectionId));
  let expandedTables = $state<Set<string>>(new Set());
  let searchQuery = $state("");

  const filteredTables = $derived(
    searchQuery.trim()
      ? schemaState.tables.filter((t) =>
          t.name.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : schemaState.tables
  );

  async function handleExpand(table: TableInfo) {
    const key = table.name;
    if (expandedTables.has(key)) {
      // Toggle off (collapse)
      expandedTables.delete(key);
      expandedTables = new Set(expandedTables);
      return;
    }
    expandedTables.add(key);
    expandedTables = new Set(expandedTables);
    if (!(key in schemaState.columns)) {
      await schemaStore.loadColumns(connectionId, table.name, schemaState.activeSchema ?? undefined);
    }
  }
</script>

<div class="schema-tree" role="tree" aria-label="Database tables">
  {#if schemaState.tables.length > 5}
    <div class="search-wrap">
      <input
        type="search"
        class="search-input"
        placeholder="Filter tables..."
        bind:value={searchQuery}
        aria-label="Filter tables"
      />
    </div>
  {/if}

  {#if schemaState.isLoadingTables}
    <div class="loading-state">
      <span class="spinner"></span>
      <span>Loading tables…</span>
    </div>
  {:else if schemaState.error}
    <div class="error-state">
      <span class="error-text">{schemaState.error}</span>
    </div>
  {:else if filteredTables.length === 0}
    <div class="empty-state">
      {searchQuery ? "No tables match your filter" : "No tables found"}
    </div>
  {:else}
    <ul class="tree-list" role="group">
      {#each filteredTables as table (table.name)}
        <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
        <li role="treeitem" aria-expanded={expandedTables.has(table.name)}>
          <SchemaNode
            {table}
            columns={schemaState.columns[table.name] ?? null}
            isExpanded={expandedTables.has(table.name)}
            isLoadingColumns={schemaState.loadingColumns.includes(table.name)}
            onexpand={handleExpand}
            onopen={onopen}
          />
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .schema-tree {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .search-wrap {
    padding: 4px 8px;
    border-bottom: 1px solid var(--color-border);
  }

  .search-input {
    width: 100%;
    font-size: 11px;
    padding: 3px 6px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    color: var(--color-text);
    box-sizing: border-box;
  }

  .search-input::placeholder {
    color: var(--color-text-muted);
  }

  .tree-list {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }

  .loading-state,
  .empty-state,
  .error-state {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 12px;
    font-size: 12px;
    color: var(--color-text-muted);
  }

  .error-text {
    color: var(--color-error);
    font-size: 11px;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
