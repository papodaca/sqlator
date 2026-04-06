import { invoke } from "@tauri-apps/api/core";
import type { SshProfile, SshProfileConfig } from "$lib/types";

let profileList = $state<SshProfile[]>([]);
let loading = $state(false);
let error = $state<string | null>(null);

export const sshProfiles = {
  get list() {
    return profileList;
  },
  get loading() {
    return loading;
  },
  get error() {
    return error;
  },

  byId(id: string): SshProfile | undefined {
    return profileList.find((p) => p.id === id);
  },

  async load() {
    loading = true;
    error = null;
    try {
      profileList = await invoke<SshProfile[]>("get_ssh_profiles");
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  },

  async save(config: SshProfileConfig): Promise<SshProfile> {
    const profile = await invoke<SshProfile>("save_ssh_profile", { config });
    profileList = [...profileList, profile].sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    return profile;
  },

  async update(id: string, config: SshProfileConfig): Promise<SshProfile> {
    const profile = await invoke<SshProfile>("update_ssh_profile", {
      id,
      config,
    });
    profileList = profileList
      .map((p) => (p.id === id ? profile : p))
      .sort((a, b) => a.name.localeCompare(b.name));
    return profile;
  },

  async remove(id: string): Promise<void> {
    await invoke("delete_ssh_profile", { id });
    profileList = profileList.filter((p) => p.id !== id);
  },

  async connectionsUsing(profileId: string): Promise<string[]> {
    return await invoke<string[]>("connections_using_ssh_profile", {
      profileId,
    });
  },
};
