<script lang="ts">
  import { onMount } from "svelte";
  import { credentialStorage } from "$lib/stores/credentials.svelte";

  let newPassword = $state("");
  let confirmPassword = $state("");
  let passwordError = $state("");
  let saving = $state(false);
  let saveMsg = $state("");

  // Vault settings form
  let timeoutMinutes = $state(15);

  onMount(async () => {
    await credentialStorage.load();
    timeoutMinutes = Math.round(credentialStorage.settings.timeout_secs / 60);
  });

  // ── Mode switching ────────────────────────────────────────────────────────

  async function switchToKeyring() {
    saving = true;
    saveMsg = "";
    try {
      await credentialStorage.setMode("keyring", true);
      saveMsg = "Switched to OS keyring. Credentials migrated.";
    } catch (e) {
      saveMsg = String(e);
    } finally {
      saving = false;
    }
  }

  async function switchToVault() {
    saving = true;
    saveMsg = "";
    try {
      await credentialStorage.setMode("vault", true);
      saveMsg = "Switched to encrypted vault. Credentials migrated.";
    } catch (e) {
      saveMsg = String(e);
    } finally {
      saving = false;
    }
  }

  // ── Vault creation ────────────────────────────────────────────────────────

  async function handleCreateVault() {
    passwordError = "";
    if (!newPassword) { passwordError = "Password is required."; return; }
    if (newPassword !== confirmPassword) { passwordError = "Passwords do not match."; return; }
    saving = true;
    try {
      await credentialStorage.createVault(newPassword);
      newPassword = "";
      confirmPassword = "";
      saveMsg = "Vault created and unlocked.";
    } catch (e) {
      passwordError = String(e);
    } finally {
      saving = false;
    }
  }

  // ── Vault lock / unlock ───────────────────────────────────────────────────

  async function handleLock() {
    await credentialStorage.lockVault();
  }

  // ── Settings ──────────────────────────────────────────────────────────────

  async function handleSaveSettings() {
    saving = true;
    saveMsg = "";
    try {
      await credentialStorage.saveSettings({ timeout_secs: timeoutMinutes * 60 });
      saveMsg = "Settings saved.";
    } catch (e) {
      saveMsg = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="settings">
  <h3 class="section-title">Credential Storage</h3>

  <!-- Current mode indicator -->
  <div class="mode-row">
    <span class="mode-label">Active mode:</span>
    <span class="mode-badge" class:keyring={credentialStorage.mode === "keyring"} class:vault={credentialStorage.mode === "vault"}>
      {credentialStorage.mode === "keyring" ? "OS Keyring" : credentialStorage.mode === "vault" ? "Encrypted Vault" : "Unknown"}
    </span>
  </div>

  <!-- Mode switchers -->
  <div class="mode-cards">
    <!-- Keyring card -->
    <div class="mode-card" class:active={credentialStorage.mode === "keyring"}>
      <div class="card-header">
        <span class="card-title">OS Keyring</span>
        {#if credentialStorage.keyringAvailable}
          <span class="badge available">Available</span>
        {:else}
          <span class="badge unavailable">Not available</span>
        {/if}
      </div>
      <p class="card-desc">Uses macOS Keychain, Windows DPAPI, or Linux Secret Service. Requires a desktop session.</p>
      {#if credentialStorage.mode !== "keyring"}
        <button
          class="btn btn-secondary"
          onclick={switchToKeyring}
          disabled={!credentialStorage.keyringAvailable || saving}
        >
          Use OS Keyring
        </button>
      {/if}
    </div>

    <!-- Vault card -->
    <div class="mode-card" class:active={credentialStorage.mode === "vault"}>
      <div class="card-header">
        <span class="card-title">Encrypted Vault</span>
        <span class="badge available">Always available</span>
      </div>
      <p class="card-desc">Argon2id + AES-256-GCM encrypted file. Works everywhere, including headless servers. Requires a master password.</p>
      {#if credentialStorage.mode !== "vault"}
        {#if credentialStorage.vaultExists}
          <button class="btn btn-secondary" onclick={switchToVault} disabled={saving}>
            Use Encrypted Vault
          </button>
        {:else}
          <p class="hint">Create a vault below first, then switch to it.</p>
        {/if}
      {:else}
        <!-- Vault is active — show lock status -->
        <div class="vault-status">
          {#if credentialStorage.vaultLocked}
            <span class="status-dot locked"></span> Locked
          {:else}
            <span class="status-dot unlocked"></span> Unlocked
            <button class="btn-link" onclick={handleLock}>Lock now</button>
          {/if}
        </div>
      {/if}
    </div>
  </div>

  <!-- Create vault (only shown if vault doesn't exist yet) -->
  {#if !credentialStorage.vaultExists}
    <div class="create-vault">
      <h4 class="sub-title">Create Vault</h4>
      <p class="hint">
        Set a master password to create an encrypted vault. This password cannot be recovered — store it safely.
      </p>
      <label class="field">
        <span class="label">Master password</span>
        <input type="password" bind:value={newPassword} class="input" placeholder="At least 12 characters" />
      </label>
      <label class="field">
        <span class="label">Confirm password</span>
        <input type="password" bind:value={confirmPassword} class="input" placeholder="Repeat password" />
      </label>
      {#if passwordError}
        <div class="msg error">{passwordError}</div>
      {/if}
      <button class="btn btn-primary" onclick={handleCreateVault} disabled={saving}>
        {saving ? "Creating…" : "Create Vault"}
      </button>
    </div>
  {/if}

  <!-- Vault timeout setting (shown when vault mode is active) -->
  {#if credentialStorage.mode === "vault"}
    <div class="vault-settings">
      <h4 class="sub-title">Vault Settings</h4>
      <label class="field">
        <span class="label">Auto-lock after (minutes, 0 = never)</span>
        <input type="number" bind:value={timeoutMinutes} min="0" max="1440" class="input input-sm" />
      </label>
      <button class="btn btn-secondary" onclick={handleSaveSettings} disabled={saving}>
        Save
      </button>
    </div>
  {/if}

  {#if saveMsg}
    <div class="msg success">{saveMsg}</div>
  {/if}
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--color-text);
  }

  .sub-title {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text);
  }

  .mode-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }

  .mode-label {
    color: var(--color-text-muted);
  }

  .mode-badge {
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 12px;
    font-weight: 600;
  }

  .mode-badge.keyring {
    background: color-mix(in oklab, var(--color-accent) 15%, transparent);
    color: var(--color-accent);
  }

  .mode-badge.vault {
    background: color-mix(in oklab, oklch(0.6 0.2 60) 15%, transparent);
    color: oklch(0.5 0.2 60);
  }

  .mode-cards {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .mode-card {
    padding: 14px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 0.15s;
  }

  .mode-card.active {
    border-color: var(--color-accent);
  }

  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }

  .card-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text);
  }

  .badge {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 4px;
    font-weight: 500;
  }

  .badge.available {
    background: oklch(0.92 0.06 145);
    color: oklch(0.35 0.15 145);
  }

  :global(.dark) .badge.available {
    background: oklch(0.22 0.06 145);
    color: oklch(0.7 0.12 145);
  }

  .badge.unavailable {
    background: oklch(0.92 0.04 25);
    color: oklch(0.45 0.12 25);
  }

  :global(.dark) .badge.unavailable {
    background: oklch(0.22 0.04 25);
    color: oklch(0.7 0.1 25);
  }

  .card-desc {
    margin: 0;
    font-size: 12px;
    color: var(--color-text-muted);
    line-height: 1.45;
  }

  .vault-status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--color-text-muted);
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
  }

  .status-dot.locked { background: oklch(0.6 0.15 25); }
  .status-dot.unlocked { background: oklch(0.55 0.2 145); }

  .btn-link {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 12px;
    color: var(--color-accent);
    padding: 0;
    margin-left: 4px;
  }

  .btn-link:hover { text-decoration: underline; }

  .create-vault, .vault-settings {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .label {
    font-size: 12px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .input {
    padding: 7px 10px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 13px;
    outline: none;
    transition: border-color 0.15s;
  }

  .input:focus { border-color: var(--color-accent); }

  .input-sm { max-width: 120px; }

  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--color-text-muted);
    line-height: 1.4;
  }

  .msg {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
  }

  .msg.success {
    background: oklch(0.95 0.05 145);
    color: oklch(0.35 0.15 145);
  }

  :global(.dark) .msg.success {
    background: oklch(0.22 0.05 145);
    color: oklch(0.7 0.12 145);
  }

  .msg.error {
    background: oklch(0.95 0.05 25);
    color: oklch(0.4 0.15 25);
  }

  :global(.dark) .msg.error {
    background: oklch(0.25 0.05 25);
    color: oklch(0.75 0.12 25);
  }

  .btn {
    padding: 7px 14px;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: background-color 0.15s;
    align-self: flex-start;
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
