<script lang="ts">
  import { editStore } from "$lib/stores/edit.svelte";

  let {
    onAddRow,
    onSave,
  }: {
    onAddRow: () => void;
    onSave: () => void;
  } = $props();
</script>

{#if editStore.tableMeta}
  <div class="grid-toolbar">
    <div class="toolbar-left">
      {#if !editStore.isEditable}
        <span class="readonly-badge" title={editStore.editabilityReason ?? ""}>
          Read-only
        </span>
      {:else}
        <button class="toolbar-btn" onclick={onAddRow} title="Add row (Ctrl/Cmd+N)">
          + Add Row
        </button>
      {/if}
    </div>

    <div class="toolbar-right">
      {#if editStore.hasChanges}
        <span class="change-badge">{editStore.changeCount} change{editStore.changeCount !== 1 ? "s" : ""}</span>
        <button class="toolbar-btn discard" onclick={() => editStore.discardAllChanges()}>
          Discard All
        </button>
        <button class="toolbar-btn save" onclick={onSave} title="Save changes (Ctrl/Cmd+S)">
          Save Changes
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .grid-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    background: var(--color-surface-2);
    border-bottom: 1px solid var(--color-border);
    min-height: 32px;
    flex-shrink: 0;
  }

  .toolbar-left,
  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .toolbar-btn {
    font-size: 12px;
    padding: 3px 10px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
    font-family: var(--font-sans, inherit);
    transition: background 0.1s;
  }

  .toolbar-btn:hover {
    background: var(--color-surface);
  }

  .toolbar-btn.discard {
    color: var(--color-text-muted);
  }

  .toolbar-btn.save {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  .toolbar-btn.save:hover {
    background: var(--color-accent);
    color: white;
  }

  .change-badge {
    font-size: 11px;
    color: var(--color-text-muted);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 1px 8px;
  }

  .readonly-badge {
    font-size: 11px;
    color: var(--color-text-muted);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    padding: 2px 8px;
    cursor: help;
  }
</style>
