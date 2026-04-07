<script lang="ts">
  import type { CellValue } from "$lib/types";

  let {
    value,
    onSave,
    onCancel,
    nullable = true,
  }: {
    value: CellValue;
    onSave: (v: CellValue) => void;
    onCancel: () => void;
    nullable?: boolean;
  } = $props();

  let selectEl = $state<HTMLSelectElement | null>(null);
  let localValue = $state(
    value === null ? "null" : value === true || value === "true" ? "true" : "false",
  );

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
    if (localValue === "null") onSave(null);
    else onSave(localValue === "true");
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
    <option value="null">NULL</option>
  {/if}
  <option value="true">true</option>
  <option value="false">false</option>
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
