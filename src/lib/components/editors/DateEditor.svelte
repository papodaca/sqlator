<script lang="ts">
  import type { CellValue } from "$lib/types";

  let {
    value,
    onSave,
    onCancel,
    type: inputType = "date",
  }: {
    value: CellValue;
    onSave: (v: CellValue) => void;
    onCancel: () => void;
    type?: "date" | "datetime-local" | "time";
  } = $props();

  let inputEl = $state<HTMLInputElement | null>(null);
  let localValue = $state(value === null ? "" : String(value).slice(0, inputType === "date" ? 10 : 19).replace(" ", "T"));

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
  });
</script>

<input
  bind:this={inputEl}
  type={inputType}
  bind:value={localValue}
  onkeydown={handleKeydown}
  onblur={commit}
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
</style>
