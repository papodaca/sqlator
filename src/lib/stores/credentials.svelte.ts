import { api } from "$lib/api";
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
        api.invoke<boolean>("check_keyring_available"),
        api.invoke<StorageMode>("get_storage_mode"),
        api.invoke<boolean>("vault_exists"),
        api.invoke<boolean>("is_vault_locked"),
        api.invoke<VaultSettings>("get_vault_settings"),
      ]);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  },

  async setMode(newMode: StorageMode, migrate = false) {
    await api.invoke("set_storage_mode", { mode: newMode, migrate });
    mode = newMode;
  },

  async createVault(password: string) {
    await api.invoke("create_vault", { password });
    vaultExists = true;
    vaultLocked = false;
    mode = "vault";
  },

  async unlockVault(password: string) {
    await api.invoke("unlock_vault", { password });
    vaultLocked = false;
  },

  async lockVault() {
    await api.invoke("lock_vault");
    vaultLocked = true;
  },

  async saveSettings(s: VaultSettings) {
    await api.invoke("save_vault_settings", { settings: s });
    settings = s;
  },

  async refreshLockState() {
    vaultLocked = await api.invoke<boolean>("is_vault_locked");
  },
};
