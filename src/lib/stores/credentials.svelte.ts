import { invoke } from "@tauri-apps/api/core";
import type { StorageMode, VaultSettings } from "$lib/types";

let mode = $state<StorageMode | null>(null);
let keyringAvailable = $state(false);
let vaultExists = $state(false);
let vaultLocked = $state(true);
let settings = $state<VaultSettings>({ timeout_secs: 15 * 60 });
let loading = $state(false);
let error = $state<string | null>(null);

export const credentialStorage = {
  get mode() { return mode; },
  get keyringAvailable() { return keyringAvailable; },
  get vaultExists() { return vaultExists; },
  get vaultLocked() { return vaultLocked; },
  get settings() { return settings; },
  get loading() { return loading; },
  get error() { return error; },

  async load() {
    loading = true;
    error = null;
    try {
      [keyringAvailable, mode, vaultExists, vaultLocked, settings] = await Promise.all([
        invoke<boolean>("check_keyring_available"),
        invoke<StorageMode>("get_storage_mode"),
        invoke<boolean>("vault_exists"),
        invoke<boolean>("is_vault_locked"),
        invoke<VaultSettings>("get_vault_settings"),
      ]);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  },

  async setMode(newMode: StorageMode, migrate = false) {
    await invoke("set_storage_mode", { mode: newMode, migrate });
    mode = newMode;
  },

  async createVault(password: string) {
    await invoke("create_vault", { password });
    vaultExists = true;
    vaultLocked = false;
    mode = "vault";
  },

  async unlockVault(password: string) {
    await invoke("unlock_vault", { password });
    vaultLocked = false;
  },

  async lockVault() {
    await invoke("lock_vault");
    vaultLocked = true;
  },

  async saveSettings(s: VaultSettings) {
    await invoke("save_vault_settings", { settings: s });
    settings = s;
  },

  async refreshLockState() {
    vaultLocked = await invoke<boolean>("is_vault_locked");
  },
};
