<script lang="ts">
  import "../app.css";
  import type { Snippet } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="app-layout">
  <Sidebar />
  <main class="main-content">
    {@render children()}
  </main>
</div>

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
