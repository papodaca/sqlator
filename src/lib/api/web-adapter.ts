import type { ApiAdapter, StreamEvent } from "./adapter";

/**
 * Maps a snake_case Tauri command name to a kebab-case HTTP path segment.
 * e.g. "get_connections" → "get-connections"
 */
function commandToPath(command: string): string {
  return command.replace(/_/g, "-");
}

export const webAdapter: ApiAdapter = {
  async invoke<T = void>(command: string, args?: Record<string, unknown>): Promise<T> {
    const resp = await fetch(`/api/${commandToPath(command)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(args ?? {}),
    });

    if (!resp.ok) {
      const msg = await resp.text().catch(() => resp.statusText);
      throw msg;
    }

    // 204 No Content
    if (resp.status === 204) return undefined as T;

    return resp.json() as Promise<T>;
  },

  executeQueryStream(
    connectionId: string,
    sql: string,
    onEvent: (msg: StreamEvent) => void,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const ws = new WebSocket(`${protocol}//${window.location.host}/api/query`);

      ws.onopen = () => {
        ws.send(JSON.stringify({ connectionId, sql }));
      };

      ws.onmessage = (e) => {
        let msg: StreamEvent;
        try {
          msg = JSON.parse(e.data as string) as StreamEvent;
        } catch {
          return;
        }
        onEvent(msg);
        // done/error/rowsAffected are terminal events — close cleanly
        if (msg.event === "done" || msg.event === "rowsAffected" || msg.event === "error") {
          ws.close();
          resolve();
        }
      };

      ws.onerror = () => {
        reject(new Error("WebSocket error during query execution"));
      };

      ws.onclose = (e) => {
        if (!e.wasClean) {
          reject(new Error(`WebSocket closed unexpectedly (code ${e.code})`));
        }
      };
    });
  },

  openPath(path: string): void {
    // In web mode there's no filesystem access. The path was written by the
    // server — open it as a download via /api/export-file?path=...
    const url = `/api/export-file?path=${encodeURIComponent(path)}`;
    window.open(url, "_blank");
  },
};
