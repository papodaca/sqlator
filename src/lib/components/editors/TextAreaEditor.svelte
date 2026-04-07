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
  let localValue = $state(value === null ? "" : String(value));

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      commit();
    }
  }

  function commit() {
    onSave(localValue === "" ? null : localValue);
  }

  $effect(() => {
    textareaEl?.focus();
  });
</script>

<div class="textarea-editor">
  <textarea
    bind:this={textareaEl}
    bind:value={localValue}
    onkeydown={handleKeydown}
    onblur={commit}
    rows={5}
    spellcheck={false}
  ></textarea>
  <span class="hint">Ctrl+Enter to save, Esc to cancel</span>
</div>

<style>
  .textarea-editor {
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
    min-height: 100px;
    border: none;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 8px;
    font-family: var(--font-mono);
    font-size: 13px;
    resize: vertical;
    outline: none;
  }

  .hint {
    display: block;
    color: var(--color-text-muted);
    font-size: 10px;
    padding: 2px 8px 4px;
  }
</style>
