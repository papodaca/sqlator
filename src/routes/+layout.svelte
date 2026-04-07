<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import VaultUnlockPrompt from "$lib/components/VaultUnlockPrompt.svelte";
  import { credentialStorage } from "$lib/stores/credentials.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { tabs } from "$lib/stores/tabs.svelte";

  // aria-live announcement text
  let announcement = $state("");

  let { children }: { children: Snippet } = $props();

  onMount(async () => {
    await credentialStorage.load();
    // Restore previous session (connects each tab in background)
    await tabs.restoreState((id) => connections.connectRaw(id));
  });

  // Announce active tab changes for screen readers
  $effect(() => {
    const queryTab = tabs.activeQueryTab;
    const connTab = tabs.activeConnectionTab;
    if (!queryTab || !connTab) return;
    const connName = connections.list.find((c) => c.id === connTab.connectionId)?.name ?? connTab.connectionId;
    announcement = `${queryTab.label}, ${connName}`;
  });

  // Debounced auto-save: persist tab layout 500ms after any state change.
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    // Read reactive state to subscribe; JSON.stringify ensures deep tracking.
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

    // Ctrl+T — new query tab
    if (e.key === "t" && !e.shiftKey) {
      const id = tabs.activeConnectionId;
      if (id) {
        e.preventDefault();
        tabs.createQueryTab(id);
      }
      return;
    }

    // Ctrl+W — close current query tab
    if (e.key === "w" && !e.shiftKey) {
      const ct = tabs.activeConnectionTab;
      if (ct?.activeQueryTabId) {
        e.preventDefault();
        tabs.closeQueryTab(ct.connectionId, ct.activeQueryTabId);
      }
      return;
    }

    // Ctrl+Tab — next connection tab
    if (e.key === "Tab" && !e.shiftKey) {
      if (tabs.connectionTabs.length > 1) {
        e.preventDefault();
        tabs.cycleConnectionTab(1);
      }
      return;
    }

    // Ctrl+Shift+Tab — previous connection tab
    if (e.key === "Tab" && e.shiftKey) {
      if (tabs.connectionTabs.length > 1) {
        e.preventDefault();
        tabs.cycleConnectionTab(-1);
      }
      return;
    }

    // Ctrl+PageDown — next query tab
    if (e.key === "PageDown") {
      e.preventDefault();
      tabs.cycleQueryTab(1);
      return;
    }

    // Ctrl+PageUp — previous query tab
    if (e.key === "PageUp") {
      e.preventDefault();
      tabs.cycleQueryTab(-1);
      return;
    }

    // Ctrl+Shift+] — next query tab (VS Code style)
    if (e.key === "]" && e.shiftKey) {
      e.preventDefault();
      tabs.cycleQueryTab(1);
      return;
    }

    // Ctrl+Shift+[ — previous query tab (VS Code style)
    if (e.key === "[" && e.shiftKey) {
      e.preventDefault();
      tabs.cycleQueryTab(-1);
      return;
    }

    // Ctrl+1-9 — jump to nth connection tab
    const digit = parseInt(e.key);
    if (!e.shiftKey && digit >= 1 && digit <= 9) {
      const target = tabs.connectionTabs[digit - 1];
      if (target) {
        e.preventDefault();
        tabs.setActiveConnection(target.connectionId);
      }
      return;
    }
  }

  const needsUnlock = $derived(
    credentialStorage.mode === "vault" &&
    credentialStorage.vaultExists &&
    credentialStorage.vaultLocked
  );
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- Screen reader live region -->
<div aria-live="polite" aria-atomic="true" class="sr-only">{announcement}</div>

<div class="app-layout">
  <Sidebar />
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
