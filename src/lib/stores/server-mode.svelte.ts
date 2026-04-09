import { api } from "$lib/api";

export type ServerMode = "multi-db" | "single-db";

interface ServerInfo {
  mode: ServerMode;
  connectionId?: string;
  connectionName?: string;
}

let mode = $state<ServerMode>("multi-db");
let connectionId = $state<string | null>(null);
let connectionName = $state<string | null>(null);
let initialized = $state(false);

export const serverMode = {
  get mode() {
    return mode;
  },
  get connectionId() {
    return connectionId;
  },
  get connectionName() {
    return connectionName;
  },
  get isSingleDb() {
    return mode === "single-db";
  },

  /**
   * Fetch server mode from the backend.
   * In Tauri mode (VITE_TARGET !== "web") this is always "multi-db".
   * Only needs to be called once at app startup.
   */
  async init() {
    if (initialized) return;
    initialized = true;

    // Tauri apps never have single-db mode
    if (import.meta.env.VITE_TARGET !== "web") return;

    try {
      const info = await api.invoke<ServerInfo>("server-info");
      mode = info.mode;
      connectionId = info.connectionId ?? null;
      connectionName = info.connectionName ?? null;
    } catch {
      // Non-fatal — fall back to multi-db
    }
  },
};
