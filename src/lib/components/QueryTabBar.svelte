<script lang="ts">
  import { tabs } from "$lib/stores/tabs.svelte";
  import type { QueryTab } from "$lib/types";

  let {
    connectionId,
    queryTabs,
    activeId,
    onselect,
    onclose,
    onnew,
  }: {
    connectionId: string;
    queryTabs: QueryTab[];
    activeId: string | null;
    onselect: (id: string) => void;
    onclose: (id: string) => void;
    onnew: () => void;
  } = $props();

  let scrollEl = $state<HTMLDivElement | null>(null);
  let canScrollLeft = $state(false);
  let canScrollRight = $state(false);
  let contextMenu = $state<{ tabId: string; x: number; y: number } | null>(null);
  let renamingId = $state<string | null>(null);
  let renameValue = $state("");

  function updateScrollState() {
    if (!scrollEl) return;
    canScrollLeft = scrollEl.scrollLeft > 1;
    canScrollRight = scrollEl.scrollLeft + scrollEl.clientWidth < scrollEl.scrollWidth - 1;
  }

  $effect(() => {
    if (!scrollEl) return;
    updateScrollState();
    const ro = new ResizeObserver(updateScrollState);
    ro.observe(scrollEl);
    scrollEl.addEventListener("scroll", updateScrollState, { passive: true });
    return () => {
      ro.disconnect();
      scrollEl?.removeEventListener("scroll", updateScrollState);
    };
  });

  // Scroll active tab into view when it changes
  $effect(() => {
    if (!scrollEl || !activeId) return;
    const el = scrollEl.querySelector(`[data-tab-id="${activeId}"]`) as HTMLElement | null;
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  function scrollBy(delta: number) {
    scrollEl?.scrollBy({ left: delta, behavior: "smooth" });
  }

  function handleMiddleClick(e: MouseEvent, id: string) {
    if (e.button === 1) {
      e.preventDefault();
      onclose(id);
    }
  }

  function handleContextMenu(e: MouseEvent, id: string) {
    e.preventDefault();
    contextMenu = { tabId: id, x: e.clientX, y: e.clientY };
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  function startRename(tab: QueryTab) {
    renamingId = tab.id;
    renameValue = tab.label;
    closeContextMenu();
  }

  function commitRename() {
    if (renamingId) {
      tabs.renameQueryTab(connectionId, renamingId, renameValue);
      renamingId = null;
    }
  }

  function handleRenameKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") commitRename();
    if (e.key === "Escape") renamingId = null;
    e.stopPropagation();
  }

  // Arrow key / Home / End navigation within the tablist
  function handleTabKeydown(e: KeyboardEvent, id: string) {
    const ids = queryTabs.map((t) => t.id);
    const idx = ids.indexOf(id);

    if (e.key === "ArrowRight") {
      e.preventDefault();
      const next = (idx + 1) % ids.length;
      onselect(ids[next]);
      focusTab(ids[next]);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      const prev = (idx - 1 + ids.length) % ids.length;
      onselect(ids[prev]);
      focusTab(ids[prev]);
    } else if (e.key === "Home") {
      e.preventDefault();
      onselect(ids[0]);
      focusTab(ids[0]);
    } else if (e.key === "End") {
      e.preventDefault();
      onselect(ids[ids.length - 1]);
      focusTab(ids[ids.length - 1]);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      onclose(id);
    }
  }

  function focusTab(id: string) {
    const el = scrollEl?.querySelector(`[data-tab-id="${id}"]`) as HTMLElement | null;
    el?.focus();
  }
</script>

<svelte:window onclick={closeContextMenu} />

<div class="tab-bar-wrap">
  {#if canScrollLeft}
    <button class="scroll-btn scroll-left" onclick={() => scrollBy(-120)} aria-label="Scroll tabs left" tabindex="-1">
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"></polyline></svg>
    </button>
  {/if}

  <div class="query-tab-bar" role="tablist" aria-label="Query tabs" bind:this={scrollEl}>
    {#each queryTabs as tab (tab.id)}
      <div
        class="tab"
        class:active={tab.id === activeId}
        role="tab"
        aria-selected={tab.id === activeId}
        aria-controls="query-panel-{tab.id}"
        id="query-tab-{tab.id}"
        data-tab-id={tab.id}
        title={tab.label}
        tabindex={tab.id === activeId ? 0 : -1}
        onclick={() => onselect(tab.id)}
        onkeydown={(e) => handleTabKeydown(e, tab.id)}
        onmousedown={(e) => handleMiddleClick(e, tab.id)}
        oncontextmenu={(e) => handleContextMenu(e, tab.id)}
        ondblclick={() => startRename(tab)}
      >
        {#if tab.isDirty}
          <span class="dirty-dot" title="Unsaved changes"></span>
        {/if}

        {#if renamingId === tab.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="rename-input"
            bind:value={renameValue}
            onblur={commitRename}
            onkeydown={handleRenameKeydown}
            onclick={(e) => e.stopPropagation()}
            autofocus
          />
        {:else}
          <span class="tab-label">{tab.label}</span>
        {/if}

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

    <button class="new-tab-btn" onclick={onnew} title="New query tab (Ctrl+T)" aria-label="New query tab" tabindex="-1">
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
        <line x1="12" y1="5" x2="12" y2="19"></line>
        <line x1="5" y1="12" x2="19" y2="12"></line>
      </svg>
    </button>
  </div>

  {#if canScrollRight}
    <button class="scroll-btn scroll-right" onclick={() => scrollBy(120)} aria-label="Scroll tabs right" tabindex="-1">
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"></polyline></svg>
    </button>
  {/if}
</div>

{#if contextMenu}
  {@const menuTab = queryTabs.find((t) => t.id === contextMenu!.tabId)}
  <div
    class="context-menu"
    style="left: {contextMenu.x}px; top: {contextMenu.y}px"
    role="menu"
  >
    <button class="menu-item" role="menuitem" onclick={() => { onclose(contextMenu!.tabId); closeContextMenu(); }}>
      Close
    </button>
    <button class="menu-item" role="menuitem" onclick={() => { startRename(menuTab!); }}>
      Rename
    </button>
    <div class="menu-divider"></div>
    <button class="menu-item" role="menuitem" onclick={() => { tabs.closeOtherQueryTabs(connectionId, contextMenu!.tabId); closeContextMenu(); }}>
      Close Others
    </button>
    <button class="menu-item" role="menuitem" onclick={() => { tabs.closeAllQueryTabs(connectionId); closeContextMenu(); }}>
      Close All
    </button>
  </div>
{/if}

<style>
  .tab-bar-wrap {
    display: flex;
    align-items: stretch;
    background: var(--color-surface);
    border-bottom: 1px solid var(--color-border);
    position: relative;
  }

  .query-tab-bar {
    display: flex;
    align-items: stretch;
    overflow-x: auto;
    overflow-y: hidden;
    flex: 1;
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
    outline: none;
  }

  .tab:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .tab:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
    z-index: 1;
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

  .rename-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--color-accent);
    border-radius: 3px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 12px;
    padding: 1px 4px;
    outline: none;
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
    background: color-mix(in oklab, var(--color-error) 15%, transparent);
    color: var(--color-error);
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

  /* Scroll buttons */
  .scroll-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    flex-shrink: 0;
    border: none;
    border-right: 1px solid var(--color-border);
    background: var(--color-surface);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: background-color 0.1s, color 0.1s;
  }

  .scroll-btn:hover {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .scroll-right {
    border-right: none;
    border-left: 1px solid var(--color-border);
  }

  /* Context menu */
  .context-menu {
    position: fixed;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 4px;
    z-index: 200;
    min-width: 140px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.18);
  }

  .menu-item {
    display: block;
    width: 100%;
    padding: 6px 12px;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 13px;
    text-align: left;
    border-radius: 4px;
    color: var(--color-text);
  }

  .menu-item:hover {
    background: var(--color-surface-2);
  }

  .menu-divider {
    height: 1px;
    background: var(--color-border);
    margin: 4px 0;
  }
</style>
