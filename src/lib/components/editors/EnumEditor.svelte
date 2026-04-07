<script lang="ts">
  import type { CellValue } from "$lib/types";

  let {
    value,
    onSave,
    onCancel,
    enumValues = [],
    nullable = true,
  }: {
    value: CellValue;
    onSave: (v: CellValue) => void;
    onCancel: () => void;
    enumValues?: string[];
    nullable?: boolean;
  } = $props();

  let selectEl = $state<HTMLSelectElement | null>(null);
  let localValue = $state(value === null ? "__null__" : String(value));

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    } else if (e.key === "Enter") {
      e.preventDefault();
      commit();
    }
  }

  function commit() {
    onSave(localValue === "__null__" ? null : localValue);
  }

  $effect(() => {
    selectEl?.focus();
  });
</script>

<select
  bind:this={selectEl}
  bind:value={localValue}
  onkeydown={handleKeydown}
  onblur={commit}
  onchange={commit}
>
  {#if nullable}
    <option value="__null__">NULL</option>
  {/if}
  {#each enumValues as ev}
    <option value={ev}>{ev}</option>
  {/each}
</select>

<style>
  select {
    width: 100%;
    height: 100%;
    border: none;
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 0 8px;
    font-family: var(--font-mono);
    font-size: 13px;
    cursor: pointer;
  }
</style>
