<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getColorHex } from "$lib/constants/colors";
  import { connections } from "$lib/stores/connections.svelte";
  import { tabs } from "$lib/stores/tabs.svelte";
  import type { ConnectionTab } from "$lib/types";

  let {
    connectionTabs,
    activeConnectionId,
    onselect,
    onclose,
    onnew,
  }: {
    connectionTabs: ConnectionTab[];
    activeConnectionId: string | null;
    onselect: (connectionId: string) => void;
    onclose: (connectionId: string) => void;
    onnew: () => void;
  } = $props();

  let scrollEl = $state<HTMLDivElement | null>(null);
  let canScrollLeft = $state(false);
  let canScrollRight = $state(false);
  let contextMenu = $state<{ connectionId: string; x: number; y: number } | null>(null);

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
    if (!scrollEl || !activeConnectionId) return;
    const el = scrollEl.querySelector(`[data-conn-id="${activeConnectionId}"]`) as HTMLElement | null;
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  function scrollBy(delta: number) {
    scrollEl?.scrollBy({ left: delta, behavior: "smooth" });
  }

  function getConnectionInfo(connectionId: string) {
    return connections.list.find((c) => c.id === connectionId) ?? null;
  }

  function statusColor(tab: ConnectionTab): string {
    if (tab.status === "connected") return "#22c55e";
    if (tab.status === "connecting") return "#f59e0b";
    if (tab.status === "error") return "var(--color-error)";
    return "var(--color-text-muted)";
  }

  function handleMiddleClick(e: MouseEvent, connectionId: string) {
    if (e.button === 1) {
      e.preventDefault();
      onclose(connectionId);
    }
  }

  function handleContextMenu(e: MouseEvent, connectionId: string) {
    e.preventDefault();
    contextMenu = { connectionId, x: e.clientX, y: e.clientY };
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  function handleTabKeydown(e: KeyboardEvent, connectionId: string) {
    const ids = connectionTabs.map((t) => t.connectionId);
    const idx = ids.indexOf(connectionId);

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
    }
  }

  function focusTab(id: string) {
    const el = scrollEl?.querySelector(`[data-conn-id="${id}"]`) as HTMLElement | null;
    el?.focus();
  }

  async function handleCloseOthers(keepId: string) {
    tabs.closeOtherConnectionTabs(keepId);
    // Disconnect the closed ones
    const closedIds = connectionTabs
      .filter((t) => t.connectionId !== keepId)
      .map((t) => t.connectionId);
    await Promise.allSettled(closedIds.map((id) => invoke("disconnect_database", { id })));
    closeContextMenu();
  }

  async function handleCloseAll() {
    const allIds = connectionTabs.map((t) => t.connectionId);
    tabs.closeAllConnectionTabs();
    await Promise.allSettled(allIds.map((id) => invoke("disconnect_database", { id })));
    closeContextMenu();
  }
</script>

<svelte:window onclick={closeContextMenu} />

<div class="tab-bar-wrap">
  {#if canScrollLeft}
    <button class="scroll-btn" onclick={() => scrollBy(-140)} aria-label="Scroll tabs left" tabindex="-1">
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"></polyline></svg>
    </button>
  {/if}

  <div class="connection-tab-bar" role="tablist" aria-label="Connection tabs" bind:this={scrollEl}>
    {#each connectionTabs as tab (tab.connectionId)}
      {@const info = getConnectionInfo(tab.connectionId)}
      <div
        class="tab"
        class:active={tab.connectionId === activeConnectionId}
        role="tab"
        aria-selected={tab.connectionId === activeConnectionId}
        title={info?.name ?? tab.connectionId}
        data-conn-id={tab.connectionId}
        tabindex={tab.connectionId === activeConnectionId ? 0 : -1}
        onclick={() => onselect(tab.connectionId)}
        onkeydown={(e) => handleTabKeydown(e, tab.connectionId)}
        onmousedown={(e) => handleMiddleClick(e, tab.connectionId)}
        oncontextmenu={(e) => handleContextMenu(e, tab.connectionId)}
      >
        {#if info}
          <span
            class="color-dot"
            style="background-color: {getColorHex(info.color_id)}"
          ></span>
        {/if}
        <span
          class="status-dot"
          style="background-color: {statusColor(tab)}"
          class:pulse={tab.status === "connecting"}
          title={tab.status}
        ></span>
        <span class="tab-label">{info?.name ?? tab.connectionId}</span>
        <button
          class="close-btn"
          tabindex="-1"
          aria-label="Close {info?.name ?? tab.connectionId}"
          onclick={(e) => { e.stopPropagation(); onclose(tab.connectionId); }}
          onmousedown={(e) => e.stopPropagation()}
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
          </svg>
        </button>
      </div>
    {/each}

    <button class="new-tab-btn" onclick={onnew} title="Open connection" aria-label="Open new connection" tabindex="-1">
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
        <line x1="12" y1="5" x2="12" y2="19"></line>
        <line x1="5" y1="12" x2="19" y2="12"></line>
      </svg>
    </button>
  </div>

  {#if canScrollRight}
    <button class="scroll-btn scroll-right" onclick={() => scrollBy(140)} aria-label="Scroll tabs right" tabindex="-1">
      <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"></polyline></svg>
    </button>
  {/if}
</div>

{#if contextMenu}
  {@const info = getConnectionInfo(contextMenu.connectionId)}
  <div
    class="context-menu"
    style="left: {contextMenu.x}px; top: {contextMenu.y}px"
    role="menu"
  >
    <button class="menu-item" role="menuitem" onclick={() => { onclose(contextMenu!.connectionId); closeContextMenu(); }}>
      Disconnect &amp; Close
    </button>
    <div class="menu-divider"></div>
    <button class="menu-item" role="menuitem" onclick={() => handleCloseOthers(contextMenu!.connectionId)}>
      Close Others
    </button>
    <button class="menu-item" role="menuitem" onclick={handleCloseAll}>
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
  }

  .connection-tab-bar {
    display: flex;
    align-items: stretch;
    overflow-x: auto;
    overflow-y: hidden;
    flex: 1;
    scrollbar-width: none;
    min-height: 38px;
  }

  .connection-tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 12px;
    min-width: 120px;
    max-width: 220px;
    height: 38px;
    border-right: 1px solid var(--color-border);
    cursor: pointer;
    flex-shrink: 0;
    color: var(--color-text-muted);
    font-size: 13px;
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
    font-weight: 500;
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

  .color-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot.pulse {
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
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
    padding: 0 12px;
    height: 38px;
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

  .context-menu {
    position: fixed;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 4px;
    z-index: 200;
    min-width: 160px;
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
