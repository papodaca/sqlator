<script lang="ts">
  import { credentialStorage } from "$lib/stores/credentials.svelte";

  let password = $state("");
  let unlocking = $state(false);
  let unlockError = $state("");

  async function handleUnlock() {
    if (!password.trim()) return;
    unlocking = true;
    unlockError = "";
    try {
      await credentialStorage.unlockVault(password);
      password = "";
    } catch (e) {
      unlockError = String(e);
    } finally {
      unlocking = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleUnlock();
  }
</script>

<div class="overlay">
  <div class="prompt">
    <div class="icon">🔒</div>
    <h2 class="title">Vault Locked</h2>
    <p class="description">
      Your credential vault is locked. Enter your master password to access SSH
      profile credentials.
    </p>

    <label class="field">
      <span class="label">Master password</span>
      <input
        type="password"
        bind:value={password}
        onkeydown={handleKeydown}
        placeholder="Enter master password"
        class="input"
        autofocus
      />
    </label>

    {#if unlockError}
      <div class="error">{unlockError}</div>
    {/if}

    <button
      class="btn-unlock"
      onclick={handleUnlock}
      disabled={!password.trim() || unlocking}
    >
      {unlocking ? "Unlocking…" : "Unlock Vault"}
    </button>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
  }

  .prompt {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 14px;
    padding: 32px;
    width: 380px;
    max-width: 90vw;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    text-align: center;
  }

  .icon {
    font-size: 36px;
  }

  .title {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    color: var(--color-text);
  }

  .description {
    margin: 0;
    font-size: 13px;
    color: var(--color-text-muted);
    line-height: 1.5;
  }

  .field {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    text-align: left;
  }

  .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .input {
    padding: 9px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 14px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    transition: border-color 0.15s;
  }

  .input:focus {
    border-color: var(--color-accent);
  }

  .error {
    width: 100%;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    background: oklch(0.95 0.05 25);
    color: oklch(0.4 0.15 25);
    text-align: left;
  }

  :global(.dark) .error {
    background: oklch(0.25 0.05 25);
    color: oklch(0.75 0.12 25);
  }

  .btn-unlock {
    width: 100%;
    padding: 10px;
    border-radius: 8px;
    border: none;
    background: var(--color-accent);
    color: white;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .btn-unlock:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-unlock:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
