<script lang="ts">
  import type { TableInfo, SchemaColumnInfo } from "$lib/types";

  let {
    table,
    columns,
    isExpanded = false,
    isLoadingColumns = false,
    onexpand,
    onopen,
  }: {
    table: TableInfo;
    columns: SchemaColumnInfo[] | null;
    isExpanded?: boolean;
    isLoadingColumns?: boolean;
    onexpand: (table: TableInfo) => void;
    onopen: (table: TableInfo) => void;
  } = $props();

  function handleToggle() {
    onexpand(table);
  }

  function handleDblClick(e: MouseEvent) {
    e.preventDefault();
    onopen(table);
  }

  function typeLabel(t: SchemaColumnInfo): string {
    return t.dataType;
  }
</script>

<div class="schema-node">
  <button
    class="table-row"
    class:expanded={isExpanded}
    onclick={handleToggle}
    ondblclick={handleDblClick}
    title="Single-click to expand, double-click to open table"
  >
    <span class="toggle-icon">
      {#if isLoadingColumns}
        <span class="spinner-sm"></span>
      {:else}
        <span class="chevron" class:open={isExpanded}>▶</span>
      {/if}
    </span>
    <span class="table-icon">{table.tableType === "view" ? "◫" : "⊞"}</span>
    <span class="table-name">{table.name}</span>
  </button>

  {#if isExpanded && columns}
    <ul class="column-list" role="list">
      {#each columns as col (col.name)}
        <li class="column-row" title={col.isForeignKey ? `→ ${col.foreignTable}.${col.foreignColumn}` : ""}>
          <span class="col-icons">
            {#if col.isPrimaryKey}<span class="icon-pk" title="Primary key">🔑</span>{/if}
            {#if col.isForeignKey}<span class="icon-fk" title="Foreign key → {col.foreignTable}">🔗</span>{/if}
            {#if !col.isPrimaryKey && !col.isForeignKey}<span class="icon-col">○</span>{/if}
          </span>
          <span class="col-name">{col.name}</span>
          <span class="col-type">{typeLabel(col)}</span>
          {#if col.nullable}<span class="col-null" title="nullable">?</span>{/if}
        </li>
      {/each}
    </ul>
  {:else if isExpanded && !columns && !isLoadingColumns}
    <p class="no-columns">No columns found</p>
  {/if}
</div>

<style>
  .schema-node {
    width: 100%;
  }

  .table-row {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 3px 8px 3px 4px;
    background: none;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--color-text);
    font-size: 12px;
    border-radius: 4px;
    user-select: none;
  }

  .table-row:hover {
    background: var(--color-surface-2);
  }

  .table-row.expanded {
    color: var(--color-accent);
  }

  .toggle-icon {
    width: 14px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .chevron {
    font-size: 8px;
    color: var(--color-text-muted);
    transition: transform 0.15s;
    display: inline-block;
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  .spinner-sm {
    width: 8px;
    height: 8px;
    border: 1.5px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    display: inline-block;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .table-icon {
    font-size: 11px;
    flex-shrink: 0;
    color: var(--color-text-muted);
  }

  .table-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .column-list {
    list-style: none;
    margin: 0;
    padding: 0 0 2px 22px;
  }

  .column-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 1px 4px;
    font-size: 11px;
    color: var(--color-text-muted);
    border-radius: 3px;
  }

  .column-row:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .col-icons {
    display: flex;
    gap: 1px;
    flex-shrink: 0;
    font-size: 9px;
  }

  .icon-col {
    color: var(--color-text-muted);
    font-size: 8px;
  }

  .col-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: monospace;
  }

  .col-type {
    color: var(--color-text-muted);
    font-size: 10px;
    flex-shrink: 0;
    opacity: 0.7;
  }

  .col-null {
    color: var(--color-text-muted);
    font-size: 10px;
    flex-shrink: 0;
    opacity: 0.5;
  }

  .no-columns {
    font-size: 11px;
    color: var(--color-text-muted);
    padding: 2px 8px 2px 26px;
    margin: 0;
  }
</style>
