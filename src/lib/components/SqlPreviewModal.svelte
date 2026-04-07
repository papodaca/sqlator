<script lang="ts">
  import { onMount } from "svelte";
  import { EditorView, basicSetup } from "codemirror";
  import { sql } from "@codemirror/lang-sql";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { theme } from "$lib/stores/theme.svelte";
  import type { SqlBatch } from "$lib/types";
  import { formatBatchForPreview } from "$lib/services/sql-generator";

  let {
    batch,
    isExecuting = false,
    onExecute,
    onCancel,
  }: {
    batch: SqlBatch;
    isExecuting?: boolean;
    onExecute: () => void;
    onCancel: () => void;
  } = $props();

  let editorEl = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;

  let sqlText = $derived(formatBatchForPreview(batch));

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      if (!isExecuting) onExecute();
    }
  }

  $effect(() => {
    if (!editorEl) return;

    if (view) {
      view.destroy();
      view = null;
    }

    const extensions = [
      basicSetup,
      sql(),
      EditorView.editable.of(false),
      EditorView.theme({
        "&": { fontSize: "13px", fontFamily: "var(--font-mono)" },
        ".cm-content": { caretColor: "var(--color-text)" },
        ".cm-gutters": {
          backgroundColor: "var(--color-surface)",
          borderRight: "1px solid var(--color-border)",
          color: "var(--color-text-muted)",
        },
      }),
    ];

    if (theme.isDark) {
      extensions.push(oneDark);
    }

    view = new EditorView({ doc: sqlText, extensions, parent: editorEl });

    return () => {
      view?.destroy();
      view = null;
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="modal-backdrop" role="presentation" onclick={onCancel}>
  <div class="modal" role="dialog" aria-modal="true" onclick={(e) => e.stopPropagation()}>
    <header class="modal-header">
      <h2>Preview SQL Changes</h2>
      <span class="stmt-count">
        {batch.statements.length} statement{batch.statements.length !== 1 ? "s" : ""}
      </span>
      <button class="close-btn" onclick={onCancel} aria-label="Close">✕</button>
    </header>

    <div class="modal-body">
      <div class="editor-container" bind:this={editorEl}></div>
    </div>

    <footer class="modal-footer">
      <span class="footer-hint">Ctrl+Enter to execute</span>
      <div class="footer-actions">
        <button class="btn secondary" onclick={onCancel} disabled={isExecuting}>
          Cancel
        </button>
        <button class="btn primary" onclick={onExecute} disabled={isExecuting}>
          {#if isExecuting}
            Executing…
          {:else}
            Execute ({batch.statements.length})
          {/if}
        </button>
      </div>
    </footer>
  </div>
</div>

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 500;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .modal {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    width: 700px;
    max-width: 90vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .modal-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .modal-header h2 {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
    flex: 1;
  }

  .stmt-count {
    font-size: 12px;
    color: var(--color-text-muted);
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 2px 10px;
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 2px 6px;
    border-radius: 4px;
  }

  .close-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .modal-body {
    flex: 1;
    overflow: hidden;
    min-height: 200px;
  }

  .editor-container {
    height: 100%;
    overflow: auto;
  }

  .editor-container :global(.cm-editor) {
    height: 100%;
  }

  .editor-container :global(.cm-scroller) {
    overflow: auto;
  }

  .modal-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-top: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .footer-hint {
    font-size: 11px;
    color: var(--color-text-muted);
  }

  .footer-actions {
    display: flex;
    gap: 8px;
  }

  .btn {
    font-size: 13px;
    padding: 6px 16px;
    border-radius: 5px;
    cursor: pointer;
    border: 1px solid var(--color-border);
    font-family: inherit;
    transition: background 0.1s;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn.secondary {
    background: transparent;
    color: var(--color-text);
  }

  .btn.secondary:hover:not(:disabled) {
    background: var(--color-surface-2);
  }

  .btn.primary {
    background: var(--color-accent);
    color: white;
    border-color: var(--color-accent);
    font-weight: 500;
  }

  .btn.primary:hover:not(:disabled) {
    opacity: 0.9;
  }
</style>
