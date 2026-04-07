<script lang="ts">
  import type { CellValue } from "$lib/types";

  let {
    value,
    onSave,
    onCancel,
  }: {
    value: CellValue;
    onSave: (v: CellValue) => void;
    onCancel: () => void;
  } = $props();

  let textareaEl = $state<HTMLTextAreaElement | null>(null);
  let localValue = $state(
    value === null ? "" : typeof value === "string" ? value : JSON.stringify(value, null, 2),
  );
  let jsonError = $state<string | null>(null);

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      commit();
    }
    // Allow regular Enter for newlines in JSON
  }

  function commit() {
    if (localValue === "") {
      onSave(null);
      return;
    }
    try {
      JSON.parse(localValue);
      jsonError = null;
      onSave(localValue);
    } catch {
      jsonError = "Invalid JSON";
    }
  }

  $effect(() => {
    textareaEl?.focus();
  });
</script>

<div class="json-editor">
  <textarea
    bind:this={textareaEl}
    bind:value={localValue}
    onkeydown={handleKeydown}
    onblur={commit}
    rows={4}
    spellcheck={false}
    class:error={jsonError !== null}
  ></textarea>
  {#if jsonError}
    <span class="error-msg">{jsonError}</span>
  {/if}
  <span class="hint">Ctrl+Enter to save</span>
</div>

<style>
  .json-editor {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 100;
    background: var(--color-surface);
    border: 2px solid var(--color-accent);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
    min-width: 300px;
  }

  textarea {
    width: 100%;
    min-height: 80px;
    border: none;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 8px;
    font-family: var(--font-mono);
    font-size: 12px;
    resize: vertical;
    outline: none;
  }

  textarea.error {
    background: color-mix(in oklch, var(--color-error) 10%, transparent);
  }

  .error-msg {
    display: block;
    color: var(--color-error);
    font-size: 11px;
    padding: 2px 8px;
  }

  .hint {
    display: block;
    color: var(--color-text-muted);
    font-size: 10px;
    padding: 2px 8px 4px;
  }
</style>
