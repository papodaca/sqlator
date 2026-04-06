<script lang="ts">
  import { onMount } from "svelte";
  import { sshConfig } from "$lib/stores/ssh-config.svelte";
  import type { SshHostEntry } from "$lib/types";

  let {
    value = $bindable<SshHostEntry | null>(null),
    onselect,
    placeholder = "Select from ~/.ssh/config…",
  }: {
    value?: SshHostEntry | null;
    onselect?: (host: SshHostEntry | null) => void;
    placeholder?: string;
  } = $props();

  let open = $state(false);
  let search = $state("");

  onMount(() => {
    if (sshConfig.hosts.length === 0 && !sshConfig.loading) {
      sshConfig.load();
    }
  });

  const filtered = $derived(
    search.trim()
      ? sshConfig.hosts.filter(
          (h) =>
            h.alias.toLowerCase().includes(search.toLowerCase()) ||
            h.hostname.toLowerCase().includes(search.toLowerCase()),
        )
      : sshConfig.hosts,
  );

  function select(host: SshHostEntry | null) {
    value = host;
    onselect?.(host);
    open = false;
    search = "";
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      open = false;
      search = "";
    }
  }
</script>

<div class="ssh-dropdown" class:open>
  <!-- Trigger -->
  <button
    class="trigger"
    type="button"
    onclick={() => (open = !open)}
    aria-haspopup="listbox"
    aria-expanded={open}
  >
    {#if value}
      <span class="alias">{value.alias}</span>
      <span class="hostname-hint">{value.hostname}:{value.port}</span>
    {:else}
      <span class="placeholder">{placeholder}</span>
    {/if}
    <svg
      class="chevron"
      class:rotated={open}
      xmlns="http://www.w3.org/2000/svg"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <polyline points="6 9 12 15 18 9"></polyline>
    </svg>
  </button>

  {#if open}
    <!-- Backdrop to close on outside click -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="backdrop" onclick={() => (open = false)}></div>

    <div class="popover" role="listbox">
      <!-- Search -->
      <div class="search-row">
        <input
          type="text"
          class="search-input"
          bind:value={search}
          placeholder="Filter hosts…"
          onkeydown={handleKeydown}
          autofocus
        />
        {#if sshConfig.loading}
          <span class="loading-indicator">…</span>
        {/if}
      </div>

      {#if sshConfig.error}
        <div class="error-row">{sshConfig.error}</div>
      {:else if filtered.length === 0}
        <div class="empty-row">
          {sshConfig.hosts.length === 0
            ? "No hosts in ~/.ssh/config"
            : "No matches"}
        </div>
      {:else}
        <!-- Clear selection -->
        {#if value}
          <button class="option clear" type="button" onclick={() => select(null)}>
            <span class="alias">— None —</span>
          </button>
        {/if}

        {#each filtered as host (host.alias)}
          <button
            class="option"
            class:selected={value?.alias === host.alias}
            type="button"
            role="option"
            aria-selected={value?.alias === host.alias}
            onclick={() => select(host)}
          >
            <span class="alias">{host.alias}</span>
            <span class="meta">
              {host.hostname}:{host.port}
              {#if host.user}· {host.user}{/if}
              {#if host.proxy_jump}
                <span class="badge">via {host.proxy_jump}</span>
              {/if}
            </span>
          </button>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .ssh-dropdown {
    position: relative;
    width: 100%;
  }

  .trigger {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 14px;
    cursor: pointer;
    text-align: left;
    transition: border-color 0.15s;
  }

  .trigger:hover,
  .open .trigger {
    border-color: var(--color-accent);
  }

  .alias {
    font-weight: 500;
    flex-shrink: 0;
  }

  .hostname-hint,
  .meta {
    font-size: 12px;
    color: var(--color-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .placeholder {
    color: var(--color-text-muted);
    flex: 1;
  }

  .chevron {
    margin-left: auto;
    flex-shrink: 0;
    color: var(--color-text-muted);
    transition: transform 0.15s;
  }

  .chevron.rotated {
    transform: rotate(180deg);
  }

  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 10;
  }

  .popover {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
    z-index: 20;
    overflow: hidden;
    max-height: 280px;
    display: flex;
    flex-direction: column;
  }

  .search-row {
    display: flex;
    align-items: center;
    padding: 8px;
    border-bottom: 1px solid var(--color-border);
    gap: 6px;
  }

  .search-input {
    flex: 1;
    padding: 4px 8px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 13px;
    outline: none;
  }

  .search-input:focus {
    border-color: var(--color-accent);
  }

  .loading-indicator {
    font-size: 13px;
    color: var(--color-text-muted);
  }

  .error-row,
  .empty-row {
    padding: 12px;
    font-size: 13px;
    color: var(--color-text-muted);
    text-align: center;
  }

  .error-row {
    color: oklch(0.55 0.15 25);
  }

  .option {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 8px 12px;
    background: none;
    border: none;
    cursor: pointer;
    text-align: left;
    font-size: 14px;
    color: var(--color-text);
    transition: background-color 0.1s;
  }

  .option:hover {
    background: var(--color-surface-2);
  }

  .option.selected {
    background: color-mix(in oklab, var(--color-accent) 12%, transparent);
  }

  .option.clear {
    color: var(--color-text-muted);
    font-size: 13px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--color-border);
  }

  .badge {
    display: inline-block;
    font-size: 11px;
    padding: 1px 5px;
    border-radius: 3px;
    background: color-mix(in oklab, var(--color-accent) 15%, transparent);
    color: var(--color-accent);
  }
</style>
