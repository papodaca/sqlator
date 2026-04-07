<script lang="ts">
  import type { ResultPaneState, SqlBatch } from "$lib/types";
  import { editStore } from "$lib/stores/edit.svelte";
  import ResultGrid from "./ResultGrid.svelte";
  import ErrorDisplay from "./ErrorDisplay.svelte";
  import SqlPreviewModal from "./SqlPreviewModal.svelte";

  let {
    result,
    isExecuting = false,
    connectionId,
    queryTabId,
    dbType,
    onReExecute,
  }: {
    result: ResultPaneState;
    isExecuting?: boolean;
    connectionId?: string;
    queryTabId?: string;
    dbType?: string;
    onReExecute?: () => void;
  } = $props();

  // SQL preview modal state
  let previewBatch = $state<SqlBatch | null>(null);
  let isSaving = $state(false);
  let saveError = $state<string | null>(null);

  function openPreview() {
    const batch = editStore.generateBatch();
    if (!batch || batch.statements.length === 0) return;
    previewBatch = batch;
    saveError = null;
  }

  async function handleExecute() {
    if (!previewBatch) return;
    isSaving = true;
    saveError = null;

    try {
      const result = await editStore.executeBatch(previewBatch);
      if (result.success) {
        previewBatch = null;
        // Re-execute the original query to refresh the grid
        onReExecute?.();
      } else {
        saveError = result.error?.message ?? "Execution failed";
        // Keep previewBatch open so user can see the error and retry
        previewBatch = null;
      }
    } catch (e) {
      saveError = String(e);
      previewBatch = null;
    } finally {
      isSaving = false;
    }
  }
</script>

<div class="result-pane">
  {#if result.kind === "idle"}
    <div class="result-empty">
      <span class="text-[--color-text-muted] text-sm">
        Results will appear here
      </span>
    </div>
  {:else if result.kind === "loading" || isExecuting}
    <div class="result-loading">
      <div class="spinner"></div>
      <span>Running query...</span>
    </div>
  {:else if result.kind === "results"}
    {#if result.rowCount > 1000}
      <div class="row-limit-notice">
        Showing first 1,000 of {result.rowCount.toLocaleString()} rows
      </div>
    {/if}
    {#if saveError}
      <div class="save-error">
        <strong>Save failed:</strong> {saveError}
        <button onclick={() => (saveError = null)}>✕</button>
      </div>
    {/if}
    <ResultGrid
      columns={result.columns}
      rows={result.rows}
      onSave={openPreview}
    />
  {:else if result.kind === "empty"}
    <div class="result-message">
      <span>Query returned 0 rows ({result.durationMs}ms)</span>
    </div>
  {:else if result.kind === "rowsAffected"}
    <div class="result-message success">
      <span>
        Query OK, {result.count} row{result.count !== 1 ? "s" : ""} affected ({result.durationMs}ms)
      </span>
    </div>
  {:else if result.kind === "error"}
    <ErrorDisplay message={result.message} />
  {/if}
</div>

{#if previewBatch}
  <SqlPreviewModal
    batch={previewBatch}
    isExecuting={isSaving}
    onExecute={handleExecute}
    onCancel={() => (previewBatch = null)}
  />
{/if}

<style>
  .result-pane {
    flex: 1;
    min-height: 100px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .result-empty,
  .result-loading,
  .result-message {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 24px;
    flex: 1;
    color: var(--color-text-muted);
    font-size: 14px;
  }

  .result-message.success {
    color: var(--color-success);
  }

  .row-limit-notice {
    padding: 4px 12px;
    background: oklch(0.92 0.05 80);
    color: oklch(0.35 0.12 80);
    font-size: 12px;
    text-align: center;
  }

  :global(.dark) .row-limit-notice {
    background: oklch(0.22 0.05 80);
    color: oklch(0.75 0.1 80);
  }

  .save-error {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: oklch(0.95 0.04 25);
    color: var(--color-error);
    font-size: 12px;
    border-bottom: 1px solid var(--color-border);
  }

  :global(.dark) .save-error {
    background: oklch(0.2 0.04 25);
  }

  .save-error button {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--color-error);
    cursor: pointer;
    font-size: 12px;
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
