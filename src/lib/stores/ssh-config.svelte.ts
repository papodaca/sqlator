import { invoke } from "@tauri-apps/api/core";
import type { SshHostEntry } from "$lib/types";

let hosts = $state<SshHostEntry[]>([]);
let loading = $state(false);
let error = $state<string | null>(null);

export const sshConfig = {
  get hosts() {
    return hosts;
  },
  get loading() {
    return loading;
  },
  get error() {
    return error;
  },

  async load() {
    loading = true;
    error = null;
    try {
      hosts = await invoke<SshHostEntry[]>("list_ssh_hosts");
    } catch (e) {
      error = String(e);
      hosts = [];
    } finally {
      loading = false;
    }
  },
};
