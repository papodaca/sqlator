<script lang="ts">
  import type { QueryTab } from "$lib/types";

  let {
    tabs,
    activeId,
    onselect,
    onclose,
    onnew,
  }: {
    tabs: QueryTab[];
    activeId: string | null;
    onselect: (id: string) => void;
    onclose: (id: string) => void;
    onnew: () => void;
  } = $props();

  function handleMiddleClick(e: MouseEvent, id: string) {
    if (e.button === 1) {
      e.preventDefault();
      onclose(id);
    }
  }

  function handleKeydown(e: KeyboardEvent, id: string) {
    if (e.key === "Enter" || e.key === " ") {
      onselect(id);
    }
  }
</script>

<div class="query-tab-bar" role="tablist" aria-label="Query tabs">
  {#each tabs as tab (tab.id)}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      class="tab"
      class:active={tab.id === activeId}
      role="tab"
      aria-selected={tab.id === activeId}
      aria-controls="query-panel-{tab.id}"
      id="query-tab-{tab.id}"
      title={tab.label}
      onclick={() => onselect(tab.id)}
      onkeydown={(e) => handleKeydown(e, tab.id)}
      onmousedown={(e) => handleMiddleClick(e, tab.id)}
      tabindex={tab.id === activeId ? 0 : -1}
    >
      {#if tab.isDirty}
        <span class="dirty-dot" title="Unsaved changes"></span>
      {/if}
      <span class="tab-label">{tab.label}</span>
      <button
        class="close-btn"
        tabindex="-1"
        aria-label="Close {tab.label}"
        onclick={(e) => { e.stopPropagation(); onclose(tab.id); }}
        onmousedown={(e) => e.stopPropagation()}
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <line x1="18" y1="6" x2="6" y2="18"></line>
          <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
      </button>
    </div>
  {/each}

  <button class="new-tab-btn" onclick={onnew} title="New query tab (Ctrl+T)" aria-label="New query tab">
    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
      <line x1="12" y1="5" x2="12" y2="19"></line>
      <line x1="5" y1="12" x2="19" y2="12"></line>
    </svg>
  </button>
</div>

<style>
  .query-tab-bar {
    display: flex;
    align-items: stretch;
    overflow-x: auto;
    overflow-y: hidden;
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    scrollbar-width: none;
    min-height: 34px;
  }

  .query-tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 10px 0 12px;
    min-width: 100px;
    max-width: 200px;
    height: 34px;
    border-right: 1px solid var(--color-border);
    cursor: pointer;
    flex-shrink: 0;
    position: relative;
    color: var(--color-text-muted);
    font-size: 12px;
    user-select: none;
    transition: background-color 0.1s;
  }

  .tab:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .tab:hover .close-btn {
    opacity: 1;
  }

  .tab.active {
    background: var(--color-bg);
    color: var(--color-text);
    border-bottom: 2px solid var(--color-accent);
  }

  .tab.active .close-btn {
    opacity: 1;
  }

  .tab-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dirty-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-accent);
    flex-shrink: 0;
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--color-text-muted);
    border-radius: 3px;
    padding: 0;
    opacity: 0;
    transition: opacity 0.1s, background-color 0.1s;
  }

  .close-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .tab.active .close-btn {
    opacity: 1;
  }

  .new-tab-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 10px;
    height: 34px;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--color-text-muted);
    flex-shrink: 0;
    transition: background-color 0.1s, color 0.1s;
  }

  .new-tab-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }
</style>
