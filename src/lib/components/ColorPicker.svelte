<script lang="ts">
  import { CONNECTION_COLORS } from "$lib/constants/colors";
  import type { ConnectionColorId } from "$lib/types";

  let { value = $bindable("blue"), onchange }: { value: ConnectionColorId; onchange?: (id: ConnectionColorId) => void } = $props();
</script>

<div class="color-picker" role="radiogroup" aria-label="Connection color">
  {#each CONNECTION_COLORS as color}
    <button
      class="color-swatch"
      class:selected={value === color.id}
      style="background-color: {color.hex}"
      title={color.label}
      role="radio"
      aria-checked={value === color.id}
      onclick={() => { value = color.id; onchange?.(color.id); }}
    ></button>
  {/each}
</div>

<style>
  .color-picker {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .color-swatch {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition: border-color 0.15s, transform 0.1s;
    padding: 0;
  }

  .color-swatch:hover {
    transform: scale(1.15);
  }

  .color-swatch.selected {
    border-color: var(--color-text);
    transform: scale(1.15);
  }
</style>
