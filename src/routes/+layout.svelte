<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import VaultUnlockPrompt from "$lib/components/VaultUnlockPrompt.svelte";
  import { credentialStorage } from "$lib/stores/credentials.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { tabs } from "$lib/stores/tabs.svelte";

  let { children }: { children: Snippet } = $props();

  onMount(async () => {
    await credentialStorage.load();
    // Restore previous session (connects each tab in background)
    await tabs.restoreState((id) => connections.connectRaw(id));
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
  }

  const needsUnlock = $derived(
    credentialStorage.mode === "vault" &&
    credentialStorage.vaultExists &&
    credentialStorage.vaultLocked
  );
</script>

<svelte:window onkeydown={handleKeydown} />

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
</style>
