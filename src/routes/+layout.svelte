<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import VaultUnlockPrompt from "$lib/components/VaultUnlockPrompt.svelte";
  import { credentialStorage } from "$lib/stores/credentials.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { tabs } from "$lib/stores/tabs.svelte";
  import { serverMode } from "$lib/stores/server-mode.svelte";

  // aria-live announcement text
  let announcement = $state("");

  let { children }: { children: Snippet } = $props();

  onMount(async () => {
    // Determine server mode first — controls whether we restore session or
    // drop straight into the single-db workspace.
    await serverMode.init();

    if (serverMode.isSingleDb && serverMode.connectionId) {
      // Single-DB mode: skip credential/vault load and session restore.
      // The pool is already pre-connected on the server; just open the tab.
      const id = serverMode.connectionId;
      tabs.openConnection(id);
      tabs.setConnectionStatus(id, "connected");
    } else {
      // Normal multi-db startup
      await credentialStorage.load();
      await tabs.restoreState((id) => connections.connectRaw(id));
    }
  });

  // Announce active tab changes for screen readers
  $effect(() => {
    const queryTab = tabs.activeQueryTab;
    const connTab = tabs.activeConnectionTab;
    if (!queryTab || !connTab) return;
    const connName =
      connections.list.find((c) => c.id === connTab.connectionId)?.name ??
      connTab.connectionId;
    announcement = `${queryTab.label}, ${connName}`;
  });

  // Debounced auto-save: persist tab layout 500ms after any state change.
  // Disabled in single-db mode (nothing to persist across sessions).
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    if (serverMode.isSingleDb) return;
    JSON.stringify(tabs.connectionTabs);
    JSON.stringify(tabs.activeConnectionId);
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      tabs.saveState();
    }, 500);
  });

  function handleKeydown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (!mod) return;

    if (e.key === "t" && !e.shiftKey) {
      const id = tabs.activeConnectionId;
      if (id) { e.preventDefault(); tabs.createQueryTab(id); }
      return;
    }
    if (e.key === "w" && !e.shiftKey) {
      const ct = tabs.activeConnectionTab;
      if (ct?.activeQueryTabId) { e.preventDefault(); tabs.closeQueryTab(ct.connectionId, ct.activeQueryTabId); }
      return;
    }
    if (e.key === "Tab" && !e.shiftKey) {
      if (tabs.connectionTabs.length > 1) { e.preventDefault(); tabs.cycleConnectionTab(1); }
      return;
    }
    if (e.key === "Tab" && e.shiftKey) {
      if (tabs.connectionTabs.length > 1) { e.preventDefault(); tabs.cycleConnectionTab(-1); }
      return;
    }
    if (e.key === "PageDown") { e.preventDefault(); tabs.cycleQueryTab(1); return; }
    if (e.key === "PageUp")   { e.preventDefault(); tabs.cycleQueryTab(-1); return; }
    if (e.key === "]" && e.shiftKey) { e.preventDefault(); tabs.cycleQueryTab(1); return; }
    if (e.key === "[" && e.shiftKey) { e.preventDefault(); tabs.cycleQueryTab(-1); return; }

    const digit = parseInt(e.key);
    if (!e.shiftKey && digit >= 1 && digit <= 9) {
      const target = tabs.connectionTabs[digit - 1];
      if (target) { e.preventDefault(); tabs.setActiveConnection(target.connectionId); }
      return;
    }
  }

  const needsUnlock = $derived(
    !serverMode.isSingleDb &&
    credentialStorage.mode === "vault" &&
    credentialStorage.vaultExists &&
    credentialStorage.vaultLocked
  );
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- Screen reader live region -->
<div aria-live="polite" aria-atomic="true" class="sr-only">{announcement}</div>

<div class="app-layout" class:single-db={serverMode.isSingleDb}>
  {#if !serverMode.isSingleDb}
    <Sidebar />
  {/if}
  <main class="main-content">
    {@render children()}
  </main>
</div>

{#if needsUnlock}
  <VaultUnlockPrompt />
{/if}

<style>
  .app-layout {
    display: flex;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
  }

  .main-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background-color: var(--color-bg);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
