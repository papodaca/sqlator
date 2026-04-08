<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type { TableBrowseState, SortSpec, FilterSpec, FilterOperator, TableQueryResult } from "$lib/types";
  import LoadMoreButton from "./LoadMoreButton.svelte";

  const LIMIT = 50;
  const MAX_ROWS = 1000;

  type FilterEntry = { operator: FilterOperator; value: string };

  let {
    browseState,
    onStateChange,
  }: {
    browseState: TableBrowseState;
    onStateChange: (patch: Partial<TableBrowseState>) => void;
  } = $props();

  // Local filter input state (debounced before sending to server)
  let localFilters: Record<string, FilterEntry> = $state({});
  let filterTimers: Record<string, ReturnType<typeof setTimeout>> = {};
  let activeFilterColumn: string | null = $state(null);

  const atLimit = $derived((browseState.result?.totalReturned ?? 0) >= MAX_ROWS);

  // Auto-fetch on first mount (table opened with isLoading=true, no result yet)
  onMount(() => {
    if (browseState.isLoading && !browseState.result) {
      fetchData(0);
    }
  });

  async function fetchData(offset: number = 0) {
    const filters: FilterSpec[] = Object.entries(localFilters)
      .filter(([, f]: [string, FilterEntry]) => f.operator === "isNull" || f.operator === "isNotNull" || f.value.trim() !== "")
      .map(([column, f]: [string, FilterEntry]) => ({
        column,
        operator: f.operator,
        value: f.operator === "isNull" || f.operator === "isNotNull" ? undefined : parseFilterValue(f.value),
      }));

    onStateChange({ isLoading: true, error: null, filters, offset });
    try {
      const result = await invoke<TableQueryResult>("query_table", {
        params: {
          connectionId: browseState.connectionId,
          tableName: browseState.tableName,
          schema: browseState.schema,
          sort: browseState.sort,
          filters,
          limit: LIMIT,
          offset,
        },
      });

      if (offset === 0) {
        onStateChange({ result, isLoading: false });
      } else {
        // Append to existing rows
        const prev = browseState.result;
        const merged: TableQueryResult = {
          ...result,
          rows: [...(prev?.rows ?? []), ...result.rows],
          totalReturned: (prev?.totalReturned ?? 0) + result.totalReturned,
        };
        onStateChange({ result: merged, isLoading: false });
      }
    } catch (e) {
      onStateChange({ error: String(e), isLoading: false });
    }
  }

  function parseFilterValue(v: string): string | number {
    const n = Number(v);
    return isNaN(n) || v.trim() === "" ? v : n;
  }

  function handleSort(column: string) {
    const existing = browseState.sort.find((s) => s.column === column);
    let newSort: SortSpec[];
    if (!existing) {
      newSort = [{ column, desc: false }];
    } else if (!existing.desc) {
      newSort = [{ column, desc: true }];
    } else {
      newSort = [];
    }
    onStateChange({ sort: newSort, offset: 0 });
    fetchData(0);
  }

  function getSortDir(column: string): "asc" | "desc" | null {
    const s = browseState.sort.find((s) => s.column === column);
    if (!s) return null;
    return s.desc ? "desc" : "asc";
  }

  function handleFilterOperatorChange(column: string, op: FilterOperator) {
    localFilters[column] = { ...localFilters[column], operator: op, value: localFilters[column]?.value ?? "" };
    localFilters = { ...localFilters };
    debounceFetch(column);
  }

  function handleFilterValueChange(column: string, value: string) {
    localFilters[column] = { operator: localFilters[column]?.operator ?? "contains", value };
    localFilters = { ...localFilters };
    debounceFetch(column);
  }

  function debounceFetch(column: string) {
    if (filterTimers[column]) clearTimeout(filterTimers[column]);
    filterTimers[column] = setTimeout(() => fetchData(0), 300);
  }

  function clearAllFilters() {
    localFilters = {};
    filterTimers = {};
    onStateChange({ filters: [], offset: 0 });
    fetchData(0);
  }

  function handleLoadMore() {
    const currentOffset = browseState.offset + (browseState.result?.totalReturned ?? 0);
    if (currentOffset >= MAX_ROWS) return;
    fetchData(currentOffset);
  }

  function getFilterOp(col: string): FilterOperator {
    return localFilters[col]?.operator ?? "contains";
  }

  function getFilterValue(col: string): string {
    return localFilters[col]?.value ?? "";
  }

  const hasAnyFilter = $derived(Object.keys(localFilters).some(
    (col) => localFilters[col].operator === "isNull" || localFilters[col].operator === "isNotNull" || localFilters[col].value.trim() !== ""
  ));

  const columns = $derived(browseState.result?.columns ?? []);
  const columnTypes = $derived(browseState.result?.columnTypes ?? []);
  const rows = $derived(browseState.result?.rows ?? []);
  const hasMore = $derived(browseState.result?.hasMore ?? false);
  const totalReturned = $derived(browseState.result?.totalReturned ?? 0);

  function getColTypeCategory(idx: number): "text" | "number" | "date" | "boolean" | "other" {
    const t = columnTypes[idx] ?? "";
    if (["integer", "bigint", "smallint", "decimal", "float", "double"].includes(t)) return "number";
    if (["date", "time", "datetime", "timestamp"].includes(t)) return "date";
    if (t === "boolean") return "boolean";
    if (["varchar", "text", "char", "uuid", "enum"].includes(t)) return "text";
    return "other";
  }

  function formatCellValue(val: unknown): string {
    if (val === null || val === undefined) return "";
    if (typeof val === "boolean") return val ? "true" : "false";
    if (typeof val === "object") return JSON.stringify(val);
    return String(val);
  }

  function isCellNull(val: unknown): boolean {
    return val === null || val === undefined;
  }
</script>

<div class="enhanced-grid">
  <!-- Loading / error state -->
  {#if browseState.isLoading && !browseState.result}
    <div class="grid-loading">
      <span class="spinner"></span>
      <span>Loading data…</span>
    </div>
  {:else if browseState.error}
    <div class="grid-error">
      <span class="error-text">{browseState.error}</span>
      <button class="retry-btn" onclick={() => fetchData(0)}>Retry</button>
    </div>
  {:else if !browseState.result}
    <div class="grid-empty">Double-click a table to browse its data</div>
  {:else}
    <!-- Toolbar -->
    {#if hasAnyFilter}
      <div class="filter-bar">
        <span class="filter-label">Filtered</span>
        <button class="clear-filters-btn" onclick={clearAllFilters}>Clear filters</button>
      </div>
    {/if}

    {#if browseState.isLoading}
      <div class="inline-loading">
        <span class="spinner-sm"></span>
        <span>Updating…</span>
      </div>
    {/if}

    <!-- Table -->
    <div class="table-wrap">
      <table class="data-table">
        <thead>
          <!-- Sort row -->
          <tr class="header-row">
            {#each columns as col, i (col)}
              {@const sortDir = getSortDir(col)}
              <th class="header-cell" aria-sort={sortDir === "asc" ? "ascending" : sortDir === "desc" ? "descending" : "none"}>
                <button
                  class="sort-btn"
                  onclick={() => handleSort(col)}
                  title="Sort by {col}"
                >
                  <span class="col-name">{col}</span>
                  <span class="sort-icon" class:active={sortDir !== null}>
                    {#if sortDir === "asc"}↑{:else if sortDir === "desc"}↓{:else}⇅{/if}
                  </span>
                </button>
              </th>
            {/each}
          </tr>
          <!-- Filter row -->
          <tr class="filter-row">
            {#each columns as col, i (col)}
              {@const cat = getColTypeCategory(i)}
              <th class="filter-cell">
                <div class="filter-wrap">
                  <select
                    class="filter-op-select"
                    value={getFilterOp(col)}
                    onchange={(e) => handleFilterOperatorChange(col, (e.target as HTMLSelectElement).value as FilterOperator)}
                    aria-label="Filter operator for {col}"
                  >
                    {#if cat === "text" || cat === "other"}
                      <option value="contains">contains</option>
                      <option value="equals">equals</option>
                      <option value="startsWith">starts with</option>
                      <option value="endsWith">ends with</option>
                    {:else if cat === "number"}
                      <option value="equals">=</option>
                      <option value="gt">&gt;</option>
                      <option value="gte">≥</option>
                      <option value="lt">&lt;</option>
                      <option value="lte">≤</option>
                    {:else if cat === "date"}
                      <option value="equals">on</option>
                      <option value="gt">after</option>
                      <option value="lt">before</option>
                    {:else if cat === "boolean"}
                      <option value="equals">equals</option>
                    {/if}
                    <option value="isNull">is null</option>
                    <option value="isNotNull">is not null</option>
                  </select>

                  {#if getFilterOp(col) !== "isNull" && getFilterOp(col) !== "isNotNull"}
                    <input
                      class="filter-input"
                      type={cat === "number" ? "number" : cat === "date" ? "date" : "text"}
                      value={getFilterValue(col)}
                      placeholder="…"
                      oninput={(e) => handleFilterValueChange(col, (e.target as HTMLInputElement).value)}
                      aria-label="Filter value for {col}"
                    />
                  {/if}
                </div>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#if rows.length === 0}
            <tr>
              <td colspan={columns.length} class="no-data">
                {hasAnyFilter ? "No results match your filter" : "No data"}
              </td>
            </tr>
          {:else}
            {#each rows as row, rowIdx (rowIdx)}
              <tr class="data-row" class:alt={rowIdx % 2 === 1}>
                {#each columns as col (col)}
                  {@const val = (row as Record<string, unknown>)[col]}
                  <td class="data-cell" class:null-cell={isCellNull(val)} title={formatCellValue(val)}>
                    {#if isCellNull(val)}
                      <span class="null-marker">NULL</span>
                    {:else}
                      {formatCellValue(val)}
                    {/if}
                  </td>
                {/each}
              </tr>
            {/each}
          {/if}
        </tbody>
      </table>
    </div>

    <LoadMoreButton
      {hasMore}
      {totalReturned}
      {atLimit}
      isLoading={browseState.isLoading}
      onclick={handleLoadMore}
    />
  {/if}
</div>

<style>
  .enhanced-grid {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
    background: var(--color-bg);
  }

  .grid-loading,
  .grid-error,
  .grid-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    flex: 1;
    font-size: 13px;
    color: var(--color-text-muted);
  }

  .error-text {
    color: var(--color-error);
  }

  .retry-btn {
    font-size: 12px;
    padding: 4px 10px;
    border: 1px solid var(--color-error);
    border-radius: 4px;
    background: transparent;
    color: var(--color-error);
    cursor: pointer;
  }

  .filter-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    font-size: 12px;
    flex-shrink: 0;
  }

  .filter-label {
    color: var(--color-accent);
    font-size: 11px;
    font-weight: 500;
  }

  .clear-filters-btn {
    margin-left: auto;
    font-size: 11px;
    padding: 2px 8px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .clear-filters-btn:hover {
    border-color: var(--color-error);
    color: var(--color-error);
  }

  .inline-loading {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: 11px;
    color: var(--color-text-muted);
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  .spinner-sm {
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    display: inline-block;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .table-wrap {
    flex: 1;
    overflow: auto;
  }

  .data-table {
    border-collapse: collapse;
    width: 100%;
    font-size: 12px;
    table-layout: auto;
  }

  thead {
    position: sticky;
    top: 0;
    z-index: 2;
  }

  .header-row th,
  .filter-row th {
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    border-right: 1px solid var(--color-border);
    padding: 0;
  }

  .header-row th:last-child,
  .filter-row th:last-child {
    border-right: none;
  }

  .sort-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 5px 8px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-text);
    font-size: 12px;
    font-weight: 600;
    text-align: left;
    white-space: nowrap;
  }

  .sort-btn:hover {
    background: var(--color-surface-2);
  }

  .col-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 150px;
  }

  .sort-icon {
    color: var(--color-text-muted);
    font-size: 11px;
    flex-shrink: 0;
  }

  .sort-icon.active {
    color: var(--color-accent);
  }

  .filter-cell {
    padding: 2px 4px 3px;
    border-bottom: 1px solid var(--color-border);
    vertical-align: top;
  }

  .filter-wrap {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 80px;
  }

  .filter-op-select {
    font-size: 10px;
    padding: 1px 2px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    color: var(--color-text-muted);
    width: 100%;
    cursor: pointer;
  }

  .filter-input {
    font-size: 11px;
    padding: 2px 4px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    color: var(--color-text);
    width: 100%;
    box-sizing: border-box;
  }

  .filter-input::placeholder {
    color: var(--color-text-muted);
    opacity: 0.6;
  }

  .filter-input:focus,
  .filter-op-select:focus {
    outline: 1px solid var(--color-accent);
  }

  .data-row {
    border-bottom: 1px solid var(--color-border);
  }

  .data-row.alt {
    background: var(--color-surface);
  }

  .data-row:hover {
    background: var(--color-surface-2);
  }

  .data-cell {
    padding: 4px 8px;
    border-right: 1px solid var(--color-border);
    max-width: 300px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    vertical-align: top;
    color: var(--color-text);
    font-family: monospace;
    font-size: 12px;
  }

  .data-cell:last-child {
    border-right: none;
  }

  .data-cell.null-cell {
    color: var(--color-text-muted);
  }

  .null-marker {
    font-style: italic;
    opacity: 0.5;
    font-size: 10px;
    font-family: sans-serif;
  }

  .no-data {
    text-align: center;
    padding: 32px;
    color: var(--color-text-muted);
    font-style: italic;
    font-size: 13px;
  }
</style>
