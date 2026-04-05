<script lang="ts">
  let { message }: { message: string } = $props();
</script>

<div class="error-display">
  <div class="error-header">
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <circle cx="12" cy="12" r="10"></circle>
      <line x1="15" y1="9" x2="9" y2="15"></line>
      <line x1="9" y1="9" x2="15" y2="15"></line>
    </svg>
    <span>Query Error</span>
  </div>
  <pre class="error-message">{message}</pre>
</div>

<style>
  .error-display {
    margin: 8px;
    border: 1px solid var(--color-error);
    border-radius: 8px;
    overflow: hidden;
  }

  .error-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: oklch(0.95 0.05 25);
    color: oklch(0.4 0.15 25);
    font-size: 13px;
    font-weight: 600;
  }

  :global(.dark) .error-header {
    background: oklch(0.22 0.05 25);
    color: oklch(0.75 0.12 25);
  }

  .error-message {
    padding: 12px;
    margin: 0;
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--color-text);
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--color-bg);
  }
</style>
