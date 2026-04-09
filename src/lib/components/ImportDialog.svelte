<script lang="ts">
  import { api } from "$lib/api";
  import { connections } from "$lib/stores/connections.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { sshProfiles } from "$lib/stores/ssh-profiles.svelte";
  import type { ImportResult } from "$lib/types";

  let { onclose }: { onclose: () => void } = $props();

  type Step = "pick" | "preview" | "done";

  let step = $state<Step>("pick");
  let fileJson = $state<string | null>(null);
  let fileName = $state("");
  let duplicateMode = $state<"skip" | "rename">("skip");
  let result = $state<ImportResult | null>(null);
  let importing = $state(false);
  let error = $state("");

  // Parsed preview (client-side, before confirming)
  interface Preview {
    connections: number;
    ssh_profiles: number;
    groups: number;
  }
  let preview = $state<Preview | null>(null);

  function handleFileChange(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    fileName = file.name;
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const text = reader.result as string;
        const parsed = JSON.parse(text);
        if (parsed.version !== "1.0") {
          error = `Unsupported export version: ${parsed.version ?? "unknown"}`;
          return;
        }
        fileJson = text;
        preview = {
          connections: parsed.connections?.length ?? 0,
          ssh_profiles: parsed.ssh_profiles?.length ?? 0,
          groups: parsed.groups?.length ?? 0,
        };
        error = "";
        step = "preview";
      } catch {
        error = "Invalid file — not a valid sqlator export.";
      }
    };
    reader.readAsText(file);
  }

  async function handleImport() {
    if (!fileJson) return;
    importing = true;
    error = "";
    try {
      result = await api.invoke<ImportResult>("import_connections", {
        json: fileJson,
        duplicateMode,
      });
      // Reload all affected stores
      await Promise.all([
        connections.load(),
        groups.load(),
        sshProfiles.load(),
      ]);
      step = "done";
    } catch (e) {
      error = String(e);
    } finally {
      importing = false;
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
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <h2 class="title">Import Connections</h2>

    {#if step === "pick"}
      <p class="hint">Select a <code>.json</code> file exported from sqlator.</p>

      <label class="file-btn">
        Choose file
        <input type="file" accept=".json,application/json" onchange={handleFileChange} />
      </label>

      {#if error}
        <p class="error">{error}</p>
      {/if}

    {:else if step === "preview"}
      <p class="file-name">📄 {fileName}</p>

      <div class="preview-grid">
        <div class="preview-item">
          <span class="preview-num">{preview?.groups}</span>
          <span class="preview-label">group{preview?.groups !== 1 ? "s" : ""}</span>
        </div>
        <div class="preview-item">
          <span class="preview-num">{preview?.ssh_profiles}</span>
          <span class="preview-label">SSH profile{preview?.ssh_profiles !== 1 ? "s" : ""}</span>
        </div>
        <div class="preview-item">
          <span class="preview-num">{preview?.connections}</span>
          <span class="preview-label">connection{preview?.connections !== 1 ? "s" : ""}</span>
        </div>
      </div>

      <p class="section-label">If a name already exists:</p>
      <div class="radio-group">
        <label class="radio-option">
          <input type="radio" bind:group={duplicateMode} value="skip" />
          Skip — don't import duplicates
        </label>
        <label class="radio-option">
          <input type="radio" bind:group={duplicateMode} value="rename" />
          Rename — append (1), (2)…
        </label>
      </div>

      <p class="security-note">
        Passwords and key passphrases are never exported. You'll need to re-enter credentials after import.
      </p>

      {#if error}
        <p class="error">{error}</p>
      {/if}

      <div class="actions">
        <button class="btn btn-secondary" onclick={() => { step = "pick"; fileJson = null; }}>
          Back
        </button>
        <button class="btn btn-primary" onclick={handleImport} disabled={importing}>
          {importing ? "Importing…" : "Import"}
        </button>
      </div>

    {:else if step === "done" && result}
      <div class="result">
        <div class="result-icon">✓</div>
        <p class="result-title">Import complete</p>
        <div class="result-grid">
          {#if result.groups_added > 0}
            <span>{result.groups_added} group{result.groups_added !== 1 ? "s" : ""} added</span>
          {/if}
          {#if result.profiles_added > 0}
            <span>{result.profiles_added} SSH profile{result.profiles_added !== 1 ? "s" : ""} added</span>
          {/if}
          <span>{result.connections_added} connection{result.connections_added !== 1 ? "s" : ""} added</span>
          {#if result.connections_skipped > 0}
            <span class="skipped">{result.connections_skipped} skipped (duplicate name)</span>
          {/if}
        </div>
      </div>

      <div class="actions">
        <button class="btn btn-primary" onclick={onclose}>Done</button>
      </div>
    {/if}

    {#if step === "pick"}
      <div class="actions-top">
        <button class="btn btn-secondary" onclick={onclose}>Cancel</button>
      </div>
    {/if}
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

  .dialog {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 24px;
    width: 420px;
    max-width: 90vw;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .title {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .hint {
    margin: 0;
    font-size: 13px;
    color: var(--color-text-muted);
  }

  .file-name {
    margin: 0;
    font-size: 13px;
    color: var(--color-text-muted);
    word-break: break-all;
  }

  .file-btn {
    display: inline-flex;
    align-items: center;
    padding: 8px 16px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
    transition: background-color 0.15s;
    align-self: flex-start;
  }

  .file-btn:hover {
    background: var(--color-border);
  }

  .file-btn input[type="file"] {
    display: none;
  }

  .preview-grid {
    display: flex;
    gap: 16px;
  }

  .preview-item {
    flex: 1;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }

  .preview-num {
    font-size: 24px;
    font-weight: 700;
    color: var(--color-accent);
  }

  .preview-label {
    font-size: 11px;
    color: var(--color-text-muted);
    text-align: center;
  }

  .section-label {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .radio-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .radio-option {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
  }

  .security-note {
    margin: 0;
    font-size: 12px;
    color: var(--color-text-muted);
    background: var(--color-surface-2);
    border-radius: 6px;
    padding: 8px 10px;
    border-left: 3px solid var(--color-accent);
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }

  .actions-top {
    display: flex;
    justify-content: flex-end;
  }

  .result {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 8px 0;
    text-align: center;
  }

  .result-icon {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: color-mix(in oklab, var(--color-accent) 15%, transparent);
    color: var(--color-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 22px;
  }

  .result-title {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  .result-grid {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
    color: var(--color-text-muted);
  }

  .skipped {
    opacity: 0.7;
  }

  .error {
    margin: 0;
    font-size: 13px;
    color: var(--color-error);
  }

  .btn {
    padding: 8px 16px;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
    border: 1px solid transparent;
    transition: opacity 0.15s;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    background: var(--color-accent);
    color: white;
  }

  .btn-secondary {
    background: var(--color-surface-2);
    border-color: var(--color-border);
    color: var(--color-text);
  }

  code {
    font-family: monospace;
    background: var(--color-surface-2);
    padding: 1px 4px;
    border-radius: 3px;
  }
</style>
