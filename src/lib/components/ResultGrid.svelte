<script lang="ts">
  import { createVirtualizer } from "@tanstack/svelte-virtual";
  import { get } from "svelte/store";

  let {
    columns,
    rows,
  }: {
    columns: string[];
    rows: Record<string, unknown>[];
  } = $props();

  let scrollEl = $state<HTMLDivElement | null>(null);

  let virtualizer = $derived(
    scrollEl
      ? createVirtualizer({
          count: rows.length,
          getScrollElement: () => scrollEl!,
          estimateSize: () => 36,
          overscan: 10,
        })
      : null,
  );

  let virtualItems = $derived(
    virtualizer ? get(virtualizer).getVirtualItems() : [],
  );
  let totalSize = $derived(
    virtualizer ? get(virtualizer).getTotalSize() : 0,
  );

  function formatCell(value: unknown): string {
    if (value === null || value === undefined) return "NULL";
    if (typeof value === "object") return JSON.stringify(value);
    return String(value);
  }

  function isNull(value: unknown): boolean {
    return value === null || value === undefined;
  }
</script>

<div class="grid-wrapper">
  <!-- Header -->
  <div class="grid-header">
    <div class="grid-row header-row">
      {#each columns as col}
        <div class="grid-cell header-cell" title={col}>
          {col}
        </div>
      {/each}
    </div>
  </div>

  <!-- Virtual scrolling body -->
  <div class="grid-body" bind:this={scrollEl}>
    <div style="height: {totalSize}px; position: relative;">
      {#each virtualItems as item (item.key)}
        {@const row = rows[item.index]}
        <div
          class="grid-row"
          class:alt={item.index % 2 === 1}
          style="position: absolute; top: {item.start}px; width: 100%; height: {item.size}px;"
        >
          {#each columns as col}
            <div
              class="grid-cell"
              class:null-cell={isNull(row[col])}
              title={formatCell(row[col])}
            >
              {formatCell(row[col])}
            </div>
          {/each}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .grid-wrapper {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: 13px;
  }

  .grid-header {
    flex-shrink: 0;
    overflow: hidden;
    border-bottom: 2px solid var(--color-border);
  }

  .grid-body {
    flex: 1;
    overflow: auto;
  }

  .grid-row {
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--color-border);
  }

  .grid-row.alt {
    background: var(--color-surface);
  }

  .header-row {
    background: var(--color-surface-2);
    font-weight: 600;
    color: var(--color-text);
  }

  .grid-cell {
    min-width: 80px;
    max-width: 300px;
    padding: 0 12px;
    height: 36px;
    line-height: 36px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-right: 1px solid var(--color-border);
    flex: 1;
  }

  .header-cell {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    color: var(--color-text-muted);
  }

  .null-cell {
    color: var(--color-text-muted);
    font-style: italic;
    opacity: 0.7;
  }
</style>
