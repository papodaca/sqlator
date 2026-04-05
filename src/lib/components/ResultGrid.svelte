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

<div class="grid-wrapper" bind:this={scrollEl}>
  <table>
    <thead>
      <tr>
        {#each columns as col}
          <th>{col}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#if (virtualItems[0]?.start ?? 0) > 0}
        <tr class="spacer" style="height: {virtualItems[0].start}px;">
          <td colspan={columns.length}></td>
        </tr>
      {/if}
      {#each virtualItems as item (item.key)}
        {@const row = rows[item.index]}
        <tr class:alt={item.index % 2 === 1}>
          {#each columns as col}
            <td class:null-cell={isNull(row[col])} title={formatCell(row[col])}>
              {formatCell(row[col])}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .grid-wrapper {
    flex: 1;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 13px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  thead {
    position: sticky;
    top: 0;
    z-index: 1;
  }

  th {
    background: var(--color-surface-2);
    text-align: left;
    padding: 0 12px;
    height: 36px;
    line-height: 36px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    color: var(--color-text-muted);
    font-weight: 600;
    border-bottom: 2px solid var(--color-border);
    white-space: nowrap;
    position: relative;
  }

  th:not(:last-child)::after {
    content: "";
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--color-border);
  }

  .spacer td {
    padding: 0;
    border: none;
  }

  td {
    padding: 0 12px;
    height: 36px;
    line-height: 36px;
    border-bottom: 1px solid var(--color-border);
    border-right: 1px solid var(--color-border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 300px;
  }

  td:last-child {
    border-right: none;
  }

  tr.alt td {
    background: var(--color-surface);
  }

  .null-cell {
    color: var(--color-text-muted);
    font-style: italic;
    opacity: 0.7;
  }
</style>
