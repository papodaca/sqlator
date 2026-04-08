<script lang="ts">
  import type { SchemaInfo } from "$lib/types";

  let {
    schemas,
    activeSchema,
    isLoading = false,
    onchange,
  }: {
    schemas: SchemaInfo[];
    activeSchema: string | null;
    isLoading?: boolean;
    onchange: (schema: string) => void;
  } = $props();
</script>

{#if schemas.length > 1}
  <div class="schema-dropdown-wrap">
    <label class="schema-label" for="schema-select">Schema</label>
    <select
      id="schema-select"
      class="schema-select"
      value={activeSchema ?? ""}
      disabled={isLoading}
      onchange={(e) => onchange((e.target as HTMLSelectElement).value)}
    >
      {#each schemas as schema (schema.name)}
        <option value={schema.name}>{schema.name}{schema.isDefault ? " (default)" : ""}</option>
      {/each}
    </select>
  </div>
{/if}

<style>
  .schema-dropdown-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
  }

  .schema-label {
    font-size: 11px;
    color: var(--color-text-muted);
    white-space: nowrap;
  }

  .schema-select {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    padding: 2px 4px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    color: var(--color-text);
    cursor: pointer;
  }

  .schema-select:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
