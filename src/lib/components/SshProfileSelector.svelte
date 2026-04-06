<script lang="ts">
  import { onMount } from "svelte";
  import { sshProfiles } from "$lib/stores/ssh-profiles.svelte";
  import SshProfileForm from "./SshProfileForm.svelte";
  import type { SshProfile } from "$lib/types";

  let {
    value = $bindable<string | null>(null),
    onchange,
  }: {
    value?: string | null;
    onchange?: (profileId: string | null) => void;
  } = $props();

  let showForm = $state(false);
  let editingProfile = $state<SshProfile | null>(null);

  onMount(async () => {
    if (sshProfiles.list.length === 0 && !sshProfiles.loading) {
      await sshProfiles.load();
    }
  });

  const selected = $derived(
    value ? sshProfiles.byId(value) ?? null : null,
  );

  function select(id: string | null) {
    value = id;
    onchange?.(id);
  }

  function openNewForm() {
    editingProfile = null;
    showForm = true;
  }

  function openEditForm(profile: SshProfile, e: MouseEvent) {
    e.stopPropagation();
    editingProfile = profile;
    showForm = true;
  }
</script>

<div class="ssh-selector">
  <div class="header">
    <span class="label">SSH tunnel</span>
    <button class="manage-btn" type="button" onclick={openNewForm}>
      + New profile
    </button>
  </div>

  {#if sshProfiles.loading}
    <div class="hint">Loading profiles…</div>
  {:else if sshProfiles.list.length === 0}
    <div class="empty">
      No SSH profiles yet.
      <button class="link-btn" type="button" onclick={openNewForm}
        >Create one</button
      >
    </div>
  {:else}
    <div class="option-list">
      <!-- None option -->
      <button
        class="option"
        class:selected={value === null}
        type="button"
        onclick={() => select(null)}
      >
        <span class="option-name">None (direct connection)</span>
      </button>

      {#each sshProfiles.list as profile (profile.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="option"
          class:selected={value === profile.id}
          role="radio"
          aria-checked={value === profile.id}
          tabindex="0"
          onclick={() => select(profile.id)}
          onkeydown={(e) => e.key === "Enter" && select(profile.id)}
        >
          <span class="option-name">{profile.name}</span>
          <span class="option-meta">
            {profile.username}@{profile.host}:{profile.port}
            · {profile.auth_method}
          </span>
          <button
            class="edit-btn"
            type="button"
            title="Edit profile"
            onclick={(e) => openEditForm(profile, e)}
          >
            ✎
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if showForm}
  <SshProfileForm
    editing={editingProfile}
    onclose={() => (showForm = false)}
    onsaved={(p) => {
      if (!editingProfile) select(p.id);
    }}
  />
{/if}

<style>
  .ssh-selector {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .manage-btn,
  .link-btn {
    font-size: 12px;
    color: var(--color-accent);
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
  }

  .manage-btn:hover,
  .link-btn:hover {
    text-decoration: underline;
  }

  .hint,
  .empty {
    font-size: 13px;
    color: var(--color-text-muted);
    padding: 8px 0;
  }

  .option-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    overflow: hidden;
  }

  .option {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 8px 32px 8px 10px;
    background: none;
    border: none;
    border-bottom: 1px solid var(--color-border);
    cursor: pointer;
    text-align: left;
    font-size: 14px;
    color: var(--color-text);
    transition: background-color 0.1s;
    width: 100%;
  }

  .option:last-child {
    border-bottom: none;
  }

  .option:hover {
    background: var(--color-surface-2);
  }

  .option.selected {
    background: color-mix(in oklab, var(--color-accent) 10%, transparent);
  }

  .option-name {
    font-weight: 500;
    font-size: 13px;
  }

  .option-meta {
    font-size: 12px;
    color: var(--color-text-muted);
  }

  .edit-btn {
    position: absolute;
    right: 8px;
    top: 50%;
    transform: translateY(-50%);
    background: none;
    border: none;
    cursor: pointer;
    font-size: 14px;
    color: var(--color-text-muted);
    padding: 2px 4px;
    border-radius: 4px;
    opacity: 0;
    transition: opacity 0.15s;
  }

  .option:hover .edit-btn {
    opacity: 1;
  }

  .edit-btn:hover {
    color: var(--color-accent);
    background: var(--color-surface-2);
  }
</style>
