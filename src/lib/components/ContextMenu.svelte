<script lang="ts">
  export interface ContextMenuItem {
    label: string;
    action: string;
    disabled?: boolean;
    danger?: boolean;
  }

  let {
    x,
    y,
    items,
    onselect,
    onclose,
  }: {
    x: number;
    y: number;
    items: ContextMenuItem[];
    onselect: (action: string) => void;
    onclose: () => void;
  } = $props();

  function handleItem(action: string, disabled: boolean | undefined) {
    if (disabled) return;
    onselect(action);
    onclose();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      onclose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- Backdrop to catch outside clicks -->
<div class="backdrop" role="presentation" onclick={onclose}></div>

<div
  class="context-menu"
  style="left: {x}px; top: {y}px;"
  role="menu"
>
  {#each items as item}
    <button
      class="menu-item"
      class:danger={item.danger}
      class:disabled={item.disabled}
      onclick={() => handleItem(item.action, item.disabled)}
      role="menuitem"
      disabled={item.disabled}
    >
      {item.label}
    </button>
  {/each}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 199;
  }

  .context-menu {
    position: fixed;
    z-index: 200;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
    padding: 4px 0;
    min-width: 160px;
  }

  .menu-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 6px 14px;
    font-size: 13px;
    background: none;
    border: none;
    color: var(--color-text);
    cursor: pointer;
    font-family: inherit;
    transition: background 0.1s;
  }

  .menu-item:hover:not(:disabled) {
    background: var(--color-surface);
  }

  .menu-item.danger {
    color: var(--color-error);
  }

  .menu-item:disabled,
  .menu-item.disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
