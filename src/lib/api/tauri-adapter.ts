import { invoke as tauriInvoke, Channel } from "@tauri-apps/api/core";
import { openPath as tauriOpenPath } from "@tauri-apps/plugin-opener";
import type { ApiAdapter, StreamEvent } from "./adapter";

export const tauriAdapter: ApiAdapter = {
  invoke<T = void>(command: string, args?: Record<string, unknown>): Promise<T> {
    return tauriInvoke<T>(command, args);
  },

  executeQueryStream(
    connectionId: string,
    sql: string,
    onEvent: (msg: StreamEvent) => void,
  ): Promise<void> {
    const channel = new Channel<{ event: string; data: Record<string, unknown> }>();
    channel.onmessage = onEvent;
    return tauriInvoke<void>("execute_query", { connectionId, sql, onEvent: channel });
  },

  openPath(path: string): void {
    tauriOpenPath(path).catch((e) => console.error("openPath failed:", e));
  },
};
