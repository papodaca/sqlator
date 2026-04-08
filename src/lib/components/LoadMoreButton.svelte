<script lang="ts">
  let {
    isLoading = false,
    hasMore = false,
    totalReturned = 0,
    atLimit = false,
    onclick,
  }: {
    isLoading?: boolean;
    hasMore?: boolean;
    totalReturned?: number;
    atLimit?: boolean;
    onclick: () => void;
  } = $props();
</script>

<div class="load-more-bar">
  <span class="row-count">
    {#if atLimit}
      Showing {totalReturned} rows (limit reached)
    {:else}
      Showing {totalReturned} row{totalReturned === 1 ? "" : "s"}
    {/if}
  </span>

  {#if hasMore && !atLimit}
    <button class="load-more-btn" disabled={isLoading} {onclick}>
      {#if isLoading}
        <span class="spinner"></span>
        Loading…
      {:else}
        Load 50 more
      {/if}
    </button>
  {:else if atLimit}
    <span class="limit-notice">Maximum 1000 rows reached</span>
  {/if}
</div>

<style>
  .load-more-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    border-top: 1px solid var(--color-border);
    background: var(--color-surface);
    flex-shrink: 0;
    font-size: 12px;
  }

  .row-count {
    color: var(--color-text-muted);
    flex: 1;
  }

  .load-more-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    font-size: 12px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    cursor: pointer;
    color: var(--color-text);
  }

  .load-more-btn:hover:not(:disabled) {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: white;
  }

  .load-more-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .limit-notice {
    color: var(--color-text-muted);
    font-style: italic;
    font-size: 11px;
  }

  .spinner {
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--color-border);
    border-top-color: currentColor;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    display: inline-block;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
