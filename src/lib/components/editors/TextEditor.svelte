<script lang="ts">
  import type { CellValue } from "$lib/types";

  let {
    value,
    onSave,
    onCancel,
    readonly = false,
  }: {
    value: CellValue;
    onSave: (v: CellValue) => void;
    onCancel: () => void;
    readonly?: boolean;
  } = $props();

  let inputEl = $state<HTMLInputElement | null>(null);
  let localValue = $state(value === null ? "" : String(value));

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
  }

  function commit() {
    onSave(localValue === "" ? null : localValue);
  }

  $effect(() => {
    inputEl?.focus();
    inputEl?.select();
  });
</script>

<input
  bind:this={inputEl}
  type="text"
  bind:value={localValue}
  onkeydown={handleKeydown}
  onblur={commit}
  placeholder="NULL"
  {readonly}
/>

<style>
  input {
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
  }

  input::placeholder {
    color: var(--color-text-muted);
    font-style: italic;
    opacity: 0.7;
  }
</style>
