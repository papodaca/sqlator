<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import VaultUnlockPrompt from "$lib/components/VaultUnlockPrompt.svelte";
  import { credentialStorage } from "$lib/stores/credentials.svelte";

  let { children }: { children: Snippet } = $props();

  onMount(async () => {
    await credentialStorage.load();
  });

  const needsUnlock = $derived(
    credentialStorage.mode === "vault" &&
    credentialStorage.vaultExists &&
    credentialStorage.vaultLocked
  );
</script>

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
