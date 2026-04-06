<script lang="ts">
  import { sshProfiles } from "$lib/stores/ssh-profiles.svelte";
  import { sshConfig } from "$lib/stores/ssh-config.svelte";
  import SshHostDropdown from "./SshHostDropdown.svelte";
  import type {
    SshProfile,
    SshProfileConfig,
    SshAuthMethod,
    SshHostEntry,
  } from "$lib/types";

  let {
    editing = null,
    onclose,
    onsaved,
  }: {
    editing?: SshProfile | null;
    onclose: () => void;
    onsaved?: (profile: SshProfile) => void;
  } = $props();

  // ── form state ───────────────────────────────────────────────────────────────
  let name = $state("");
  let host = $state("");
  let port = $state(22);
  let username = $state("");
  let authMethod = $state<SshAuthMethod>("key");
  let keyPath = $state("");
  let keyPassphrase = $state("");
  let password = $state("");
  let localPortBinding = $state<number | "">("");
  let keepaliveInterval = $state<number | "">("");
  let saving = $state(false);
  let saveError = $state("");

  // Sync form fields whenever the `editing` prop changes
  $effect(() => {
    name = editing?.name ?? "";
    host = editing?.host ?? "";
    port = editing?.port ?? 22;
    username = editing?.username ?? "";
    authMethod = editing?.auth_method ?? "key";
    keyPath = editing?.key_path ?? "";
    keyPassphrase = "";
    password = "";
    localPortBinding = editing?.local_port_binding ?? "";
    keepaliveInterval = editing?.keepalive_interval ?? "";
    saving = false;
    saveError = "";
  });

  // Populate from ~/.ssh/config host selection
  function applyConfigHost(entry: SshHostEntry | null) {
    if (!entry) return;
    host = entry.hostname;
    port = entry.port;
    if (entry.user) username = entry.user;
    if (entry.identity_file) {
      keyPath = entry.identity_file;
      authMethod = "key";
    }
    if (!name) name = entry.alias;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }

  function buildConfig(): SshProfileConfig {
    return {
      name: name.trim(),
      host: host.trim(),
      port,
      username: username.trim(),
      auth_method: authMethod,
      key_path: authMethod === "key" && keyPath.trim() ? keyPath.trim() : null,
      password:
        authMethod === "password" && password ? password : null,
      key_passphrase:
        authMethod === "key" && keyPassphrase ? keyPassphrase : null,
      proxy_jump: [],
      local_port_binding:
        localPortBinding !== "" ? Number(localPortBinding) : null,
      keepalive_interval:
        keepaliveInterval !== "" ? Number(keepaliveInterval) : null,
    };
  }

  const isValid = $derived(
    name.trim().length > 0 &&
      host.trim().length > 0 &&
      username.trim().length > 0,
  );

  async function handleSave() {
    if (!isValid) return;
    saving = true;
    saveError = "";
    try {
      const config = buildConfig();
      let profile: SshProfile;
      if (editing) {
        profile = await sshProfiles.update(editing.id, config);
      } else {
        profile = await sshProfiles.save(config);
      }
      onsaved?.(profile);
      onclose();
    } catch (e) {
      saveError = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onclick={onclose}>
  <div class="dialog" onclick={(e: MouseEvent) => e.stopPropagation()}>
    <h2 class="title">{editing ? "Edit" : "New"} SSH Profile</h2>

    <!-- Import from ~/.ssh/config -->
    <div class="field">
      <span class="label">Import from ~/.ssh/config</span>
      <SshHostDropdown onselect={applyConfigHost} placeholder="Pick a host to auto-fill…" />
    </div>

    <hr class="divider" />

    <!-- Name -->
    <label class="field">
      <span class="label">Profile name <span class="required">*</span></span>
      <input
        type="text"
        class="input"
        bind:value={name}
        placeholder="Production bastion"
        autofocus
      />
    </label>

    <!-- Host + Port -->
    <div class="row">
      <label class="field grow">
        <span class="label">SSH host <span class="required">*</span></span>
        <input
          type="text"
          class="input"
          bind:value={host}
          placeholder="bastion.example.com"
        />
      </label>
      <label class="field port">
        <span class="label">Port</span>
        <input
          type="number"
          class="input"
          bind:value={port}
          min="1"
          max="65535"
        />
      </label>
    </div>

    <!-- Username -->
    <label class="field">
      <span class="label">Username <span class="required">*</span></span>
      <input
        type="text"
        class="input"
        bind:value={username}
        placeholder="deploy"
      />
    </label>

    <!-- Auth method tabs -->
    <div class="field">
      <span class="label">Authentication</span>
      <div class="auth-tabs" role="tablist">
        {#each (["key", "password", "agent"] as const) as method}
          <button
            class="auth-tab"
            class:active={authMethod === method}
            type="button"
            role="tab"
            aria-selected={authMethod === method}
            onclick={() => (authMethod = method)}
          >
            {method === "key" ? "Key file" : method === "password" ? "Password" : "SSH agent"}
          </button>
        {/each}
      </div>
    </div>

    {#if authMethod === "key"}
      <label class="field">
        <span class="label">Identity file path</span>
        <input
          type="text"
          class="input font-mono"
          bind:value={keyPath}
          placeholder="~/.ssh/id_ed25519"
        />
      </label>
      <label class="field">
        <span class="label">Key passphrase <span class="muted">(leave blank to keep existing)</span></span>
        <input
          type="password"
          class="input"
          bind:value={keyPassphrase}
          placeholder="••••••••"
          autocomplete="new-password"
        />
      </label>
    {:else if authMethod === "password"}
      <label class="field">
        <span class="label">Password <span class="muted">(leave blank to keep existing)</span></span>
        <input
          type="password"
          class="input"
          bind:value={password}
          placeholder="••••••••"
          autocomplete="new-password"
        />
      </label>
    {:else}
      <p class="agent-note">
        Credentials will be read from your running SSH agent (<code>SSH_AUTH_SOCK</code>).
      </p>
    {/if}

    <!-- Advanced (collapsed details) -->
    <details class="advanced">
      <summary>Advanced options</summary>
      <div class="advanced-body">
        <label class="field">
          <span class="label">Local port binding <span class="muted">(0 or blank = auto)</span></span>
          <input
            type="number"
            class="input"
            bind:value={localPortBinding}
            min="0"
            max="65535"
            placeholder="auto"
          />
        </label>
        <label class="field">
          <span class="label">Keep-alive interval (seconds, 0 = disabled)</span>
          <input
            type="number"
            class="input"
            bind:value={keepaliveInterval}
            min="0"
            placeholder="30"
          />
        </label>
      </div>
    </details>

    {#if saveError}
      <div class="error-msg">{saveError}</div>
    {/if}

    <div class="actions">
      <button class="btn btn-secondary" onclick={onclose}>Cancel</button>
      <button
        class="btn btn-primary"
        onclick={handleSave}
        disabled={!isValid || saving}
      >
        {saving ? "Saving…" : "Save profile"}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
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
    width: 520px;
    max-width: 95vw;
    max-height: 90vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .title {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text);
  }

  .divider {
    border: none;
    border-top: 1px solid var(--color-border);
    margin: 0;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .row {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }

  .grow { flex: 1; }

  .port { width: 90px; flex-shrink: 0; }

  .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .required { color: oklch(0.55 0.15 25); }
  .muted { font-weight: 400; opacity: 0.7; }

  .input {
    padding: 8px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
    width: 100%;
    box-sizing: border-box;
  }

  .input:focus { border-color: var(--color-accent); }

  .font-mono { font-family: var(--font-mono, monospace); }

  .auth-tabs {
    display: flex;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    overflow: hidden;
  }

  .auth-tab {
    flex: 1;
    padding: 7px 12px;
    font-size: 13px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: background-color 0.15s, color 0.15s;
  }

  .auth-tab + .auth-tab {
    border-left: 1px solid var(--color-border);
  }

  .auth-tab.active {
    background: var(--color-accent);
    color: #fff;
  }

  .agent-note {
    font-size: 13px;
    color: var(--color-text-muted);
    margin: 0;
    padding: 10px 12px;
    background: var(--color-surface-2);
    border-radius: 6px;
  }

  .agent-note code {
    font-family: var(--font-mono, monospace);
    font-size: 12px;
  }

  .advanced {
    border: 1px solid var(--color-border);
    border-radius: 6px;
  }

  .advanced summary {
    padding: 8px 12px;
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-muted);
    cursor: pointer;
    user-select: none;
  }

  .advanced-body {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    border-top: 1px solid var(--color-border);
  }

  .error-msg {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    background: oklch(0.95 0.05 25);
    color: oklch(0.4 0.15 25);
  }

  :global(.dark) .error-msg {
    background: oklch(0.25 0.05 25);
    color: oklch(0.75 0.12 25);
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
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

  .btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .btn-primary {
    background: var(--color-accent);
    color: white;
  }

  .btn-primary:hover:not(:disabled) { background: var(--color-accent-hover); }

  .btn-secondary {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .btn-secondary:hover:not(:disabled) { background: var(--color-border); }
</style>
