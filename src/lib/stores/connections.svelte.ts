import { invoke } from "@tauri-apps/api/core";
import type {
  ConnectionInfo,
  ConnectionConfig,
  ConnectionStatus,
} from "$lib/types";

let connectionList = $state<ConnectionInfo[]>([]);
let activeId = $state<string | null>(null);
let status = $state<ConnectionStatus>("disconnected");
let connectionError = $state<string | null>(null);

export const connections = {
  get list() {
    return connectionList;
  },
  get activeId() {
    return activeId;
  },
  get active(): ConnectionInfo | null {
    return connectionList.find((c) => c.id === activeId) ?? null;
  },
  get status() {
    return status;
  },
  get error() {
    return connectionError;
  },

  async load() {
    try {
      connectionList = await invoke<ConnectionInfo[]>("get_connections");
    } catch (e) {
      console.error("Failed to load connections:", e);
    }
  },

  async save(config: ConnectionConfig): Promise<ConnectionInfo> {
    const info = await invoke<ConnectionInfo>("save_connection", { config });
    connectionList = [...connectionList, info];
    return info;
  },

  async update(
    id: string,
    config: ConnectionConfig,
  ): Promise<ConnectionInfo> {
    const info = await invoke<ConnectionInfo>("update_connection", {
      id,
      config,
    });
    connectionList = connectionList.map((c) => (c.id === id ? info : c));
    return info;
  },

  async remove(id: string) {
    await invoke("delete_connection", { id });
    connectionList = connectionList.filter((c) => c.id !== id);
    if (activeId === id) {
      activeId = null;
      status = "disconnected";
    }
  },

  async test(url: string): Promise<string> {
    return await invoke<string>("test_connection", { url });
  },

  async select(id: string) {
    if (activeId === id && status === "connected") return;

    activeId = id;
    status = "connecting";
    connectionError = null;

    try {
      await invoke("connect_database", { id });
      status = "connected";
    } catch (e) {
      status = "error";
      connectionError = String(e);
    }
  },

  async retry() {
    if (activeId) {
      await this.select(activeId);
    }
  },

  /** Apply an updated ConnectionInfo (e.g. after a group move) without a full reload. */
  applyMove(info: ConnectionInfo) {
    connectionList = connectionList.map((c) => (c.id === info.id ? info : c));
  },
};
