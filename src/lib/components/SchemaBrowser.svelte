<script lang="ts">
  import type { TableInfo } from "$lib/types";
  import { schemaStore } from "$lib/stores/schema.svelte";
  import SchemaDropdown from "./SchemaDropdown.svelte";
  import SchemaTree from "./SchemaTree.svelte";

  let {
    connectionId,
    isConnected = false,
    onopen,
  }: {
    connectionId: string;
    isConnected?: boolean;
    onopen: (table: TableInfo) => void;
  } = $props();

  const schemaState = $derived(schemaStore.getState(connectionId));

  let hasLoaded = $state(false);

  $effect(() => {
    if (isConnected && connectionId && !hasLoaded) {
      hasLoaded = true;
      schemaStore.loadSchemas(connectionId);
    }
    if (!isConnected) {
      hasLoaded = false;
    }
  });

  async function handleRefresh() {
    await schemaStore.refresh(connectionId);
  }

  async function handleSchemaChange(schema: string) {
    await schemaStore.setSchema(connectionId, schema);
  }
</script>

<div class="schema-browser" class:disabled={!isConnected}>
  <div class="browser-header">
    <span class="browser-title">Schema</span>
    <button
      class="refresh-btn"
      onclick={handleRefresh}
      disabled={!isConnected || schemaState.isLoadingTables || schemaState.isLoadingSchemas}
      title="Refresh schema"
      aria-label="Refresh schema"
    >
      ↺
    </button>
  </div>

  {#if !isConnected}
    <div class="disconnected-banner">
      Connect to browse schema
    </div>
  {:else}
    <SchemaDropdown
      schemas={schemaState.schemas}
      activeSchema={schemaState.activeSchema}
      isLoading={schemaState.isLoadingSchemas}
      onchange={handleSchemaChange}
    />

    <SchemaTree {connectionId} onopen={onopen} />
  {/if}
</div>

<style>
  .schema-browser {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--color-border);
    min-height: 0;
    flex: 1;
    overflow: hidden;
  }

  .schema-browser.disabled {
    opacity: 0.5;
  }

  .browser-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 4px;
    flex-shrink: 0;
  }

  .browser-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .refresh-btn {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 14px;
    padding: 0;
    line-height: 1;
  }

  .refresh-btn:hover:not(:disabled) {
    background: var(--color-surface-2);
    border-color: var(--color-border);
    color: var(--color-text);
  }

  .refresh-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .disconnected-banner {
    padding: 8px 12px;
    font-size: 11px;
    color: var(--color-text-muted);
    text-align: center;
  }
</style>
