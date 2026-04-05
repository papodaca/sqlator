<script lang="ts">
  import ColorPicker from "./ColorPicker.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import type { ConnectionColorId, ConnectionInfo } from "$lib/types";

  let {
    editing = null,
    onclose,
  }: {
    editing?: ConnectionInfo | null;
    onclose: () => void;
  } = $props();

  let name = $state(editing?.name ?? "");
  let url = $state("");
  let colorId = $state<ConnectionColorId>(
    (editing?.color_id as ConnectionColorId) ?? "blue",
  );
  let testStatus = $state<"idle" | "testing" | "success" | "error">("idle");
  let testMessage = $state("");
  let saving = $state(false);
  let saveError = $state("");

  async function handleTest() {
    if (!url.trim()) return;
    testStatus = "testing";
    testMessage = "";
    try {
      testMessage = await connections.test(url);
      testStatus = "success";
    } catch (e) {
      testMessage = String(e);
      testStatus = "error";
    }
  }

  async function handleSave() {
    if (!name.trim() || !url.trim()) return;
    saving = true;
    saveError = "";
    try {
      const config = { name: name.trim(), color_id: colorId, url: url.trim() };
      if (editing) {
        await connections.update(editing.id, config);
      } else {
        await connections.save(config);
      }
      onclose();
    } catch (e) {
      saveError = String(e);
    } finally {
      saving = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onclick={onclose}>
  <div class="form-dialog" onclick={(e: MouseEvent) => e.stopPropagation()}>
    <h2 class="form-title">{editing ? "Edit" : "New"} Connection</h2>

    <label class="field">
      <span class="label">Name</span>
      <input
        type="text"
        bind:value={name}
        placeholder="My Database"
        class="input"
        autofocus
      />
    </label>

    <label class="field">
      <span class="label">Connection URL</span>
      <input
        type="text"
        bind:value={url}
        placeholder="postgres://user:pass@host:5432/dbname"
        class="input font-mono text-sm"
      />
    </label>

    <div class="field">
      <span class="label">Color</span>
      <ColorPicker bind:value={colorId} />
    </div>

    {#if testStatus !== "idle"}
      <div
        class="test-result"
        class:success={testStatus === "success"}
        class:error={testStatus === "error"}
      >
        {#if testStatus === "testing"}
          Testing connection...
        {:else}
          {testMessage}
        {/if}
      </div>
    {/if}

    {#if saveError}
      <div class="test-result error">{saveError}</div>
    {/if}

    <div class="actions">
      <button class="btn btn-secondary" onclick={onclose}>Cancel</button>
      <button
        class="btn btn-secondary"
        onclick={handleTest}
        disabled={!url.trim() || testStatus === "testing"}
      >
        Test Connection
      </button>
      <button
        class="btn btn-primary"
        onclick={handleSave}
        disabled={!name.trim() || !url.trim() || saving}
      >
        {saving ? "Saving..." : "Save"}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .form-dialog {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 24px;
    width: 480px;
    max-width: 90vw;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .form-title {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .input {
    padding: 8px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
  }

  .input:focus {
    border-color: var(--color-accent);
  }

  .test-result {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
  }

  .test-result.success {
    background: oklch(0.95 0.05 145);
    color: oklch(0.35 0.15 145);
  }

  :global(.dark) .test-result.success {
    background: oklch(0.25 0.05 145);
    color: oklch(0.75 0.12 145);
  }

  .test-result.error {
    background: oklch(0.95 0.05 25);
    color: oklch(0.4 0.15 25);
  }

  :global(.dark) .test-result.error {
    background: oklch(0.25 0.05 25);
    color: oklch(0.75 0.12 25);
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 8px;
  }

  .btn {
    padding: 8px 16px;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: background-color 0.15s;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    background: var(--color-accent);
    color: white;
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-secondary {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--color-border);
  }
</style>
