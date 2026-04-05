<script lang="ts">
  import SqlEditor from "$lib/components/SqlEditor.svelte";
  import EditorToolbar from "$lib/components/EditorToolbar.svelte";
  import ResultPane from "$lib/components/ResultPane.svelte";
  import { connections } from "$lib/stores/connections.svelte";
</script>

<div class="editor-area">
  {#if connections.active}
    <EditorToolbar />
    <SqlEditor />
    <ResultPane />
  {:else}
    <div class="empty-state">
      <p class="text-[--color-text-muted] text-lg">
        Select a connection to start querying
      </p>
    </div>
  {/if}
</div>

<style>
  .editor-area {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  .empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
  }
</style>
