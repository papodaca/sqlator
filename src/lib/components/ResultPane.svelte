<script lang="ts">
  import { query } from "$lib/stores/query.svelte";
  import ResultGrid from "./ResultGrid.svelte";
  import ErrorDisplay from "./ErrorDisplay.svelte";
</script>

<div class="result-pane">
  {#if query.result.kind === "idle"}
    <div class="result-empty">
      <span class="text-[--color-text-muted] text-sm">
        Results will appear here
      </span>
    </div>
  {:else if query.result.kind === "loading"}
    <div class="result-loading">
      <div class="spinner"></div>
      <span>Running query...</span>
    </div>
  {:else if query.result.kind === "results"}
    {#if query.result.rowCount > 1000}
      <div class="row-limit-notice">
        Showing first 1,000 of {query.result.rowCount.toLocaleString()} rows
      </div>
    {/if}
    <ResultGrid
      columns={query.result.columns}
      rows={query.result.rows}
    />
  {:else if query.result.kind === "empty"}
    <div class="result-message">
      <span>Query returned 0 rows ({query.result.durationMs}ms)</span>
    </div>
  {:else if query.result.kind === "rowsAffected"}
    <div class="result-message success">
      <span>
        Query OK, {query.result.count} row{query.result.count !== 1 ? "s" : ""} affected ({query.result.durationMs}ms)
      </span>
    </div>
  {:else if query.result.kind === "error"}
    <ErrorDisplay message={query.result.message} />
  {/if}
</div>

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
