import { invoke, Channel } from "@tauri-apps/api/core";
import type { ConnectionTab, QueryTab, ResultPaneState, ConnectionStatus } from "$lib/types";

// ── Persistence types ─────────────────────────────────────────────────────────

interface PersistedQueryTab {
  id: string;
  label: string;
  sql: string;
}

interface PersistedConnectionTab {
  connectionId: string;
  queryTabs: PersistedQueryTab[];
  activeQueryTabId: string | null;
}

interface PersistedTabState {
  connectionTabs: PersistedConnectionTab[];
  activeConnectionId: string | null;
}

let connectionTabs = $state<ConnectionTab[]>([]);
let activeConnectionId = $state<string | null>(null);

function makeQueryTab(index: number): QueryTab {
  return {
    id: crypto.randomUUID(),
    label: `Query ${index}`,
    sql: "",
    isDirty: false,
    result: { kind: "idle" },
    isExecuting: false,
  };
}

export const tabs = {
  get connectionTabs() {
    return connectionTabs;
  },

  get activeConnectionId() {
    return activeConnectionId;
  },

  get activeConnectionTab(): ConnectionTab | null {
    return connectionTabs.find((t) => t.connectionId === activeConnectionId) ?? null;
  },

  get activeQueryTab(): QueryTab | null {
    const ct = this.activeConnectionTab;
    if (!ct) return null;
    return ct.queryTabs.find((t) => t.id === ct.activeQueryTabId) ?? null;
  },

  /** Open a connection tab; if already open, focus it. Returns true if newly opened. */
  openConnection(connectionId: string): boolean {
    const existing = connectionTabs.find((t) => t.connectionId === connectionId);
    if (existing) {
      activeConnectionId = connectionId;
      return false;
    }
    const firstTab = makeQueryTab(1);
    connectionTabs = [
      ...connectionTabs,
      {
        connectionId,
        queryTabs: [firstTab],
        activeQueryTabId: firstTab.id,
        status: "disconnected",
        error: null,
      },
    ];
    activeConnectionId = connectionId;
    return true;
  },

  closeConnection(connectionId: string) {
    connectionTabs = connectionTabs.filter((t) => t.connectionId !== connectionId);
    if (activeConnectionId === connectionId) {
      activeConnectionId = connectionTabs.at(-1)?.connectionId ?? null;
    }
  },

  setActiveConnection(connectionId: string) {
    activeConnectionId = connectionId;
  },

  setConnectionStatus(connectionId: string, status: ConnectionStatus, error?: string | null) {
    connectionTabs = connectionTabs.map((t) =>
      t.connectionId === connectionId ? { ...t, status, error: error ?? null } : t
    );
  },

  createQueryTab(connectionId: string) {
    const ct = connectionTabs.find((t) => t.connectionId === connectionId);
    if (!ct) return;
    const newTab = makeQueryTab(ct.queryTabs.length + 1);
    connectionTabs = connectionTabs.map((t) =>
      t.connectionId === connectionId
        ? { ...t, queryTabs: [...t.queryTabs, newTab], activeQueryTabId: newTab.id }
        : t
    );
  },

  closeQueryTab(connectionId: string, queryTabId: string) {
    const ct = connectionTabs.find((t) => t.connectionId === connectionId);
    if (!ct || ct.queryTabs.length <= 1) return;
    const remaining = ct.queryTabs.filter((t) => t.id !== queryTabId);
    const closedWasActive = ct.activeQueryTabId === queryTabId;
    const newActiveId = closedWasActive
      ? (remaining.at(-1)?.id ?? null)
      : ct.activeQueryTabId;
    connectionTabs = connectionTabs.map((t) =>
      t.connectionId === connectionId
        ? { ...t, queryTabs: remaining, activeQueryTabId: newActiveId }
        : t
    );
  },

  setActiveQueryTab(connectionId: string, queryTabId: string) {
    connectionTabs = connectionTabs.map((t) =>
      t.connectionId === connectionId ? { ...t, activeQueryTabId: queryTabId } : t
    );
  },

  updateSql(connectionId: string, queryTabId: string, sql: string) {
    connectionTabs = connectionTabs.map((ct) =>
      ct.connectionId === connectionId
        ? {
            ...ct,
            queryTabs: ct.queryTabs.map((qt) =>
              qt.id === queryTabId ? { ...qt, sql, isDirty: true } : qt
            ),
          }
        : ct
    );
  },

  /** Cycle to the next/previous connection tab. */
  cycleConnectionTab(direction: 1 | -1) {
    if (connectionTabs.length === 0) return;
    const idx = connectionTabs.findIndex((t) => t.connectionId === activeConnectionId);
    const next = (idx + direction + connectionTabs.length) % connectionTabs.length;
    activeConnectionId = connectionTabs[next].connectionId;
  },

  /** Cycle to the next/previous query tab within the active connection. */
  cycleQueryTab(direction: 1 | -1) {
    const ct = this.activeConnectionTab;
    if (!ct || ct.queryTabs.length === 0) return;
    const idx = ct.queryTabs.findIndex((t) => t.id === ct.activeQueryTabId);
    const next = (idx + direction + ct.queryTabs.length) % ct.queryTabs.length;
    this.setActiveQueryTab(ct.connectionId, ct.queryTabs[next].id);
  },

  async executeQuery(connectionId: string, queryTabId: string, sql: string) {
    if (!sql.trim()) return;

    // Mark tab as executing
    this._setTabExecuting(connectionId, queryTabId, true, { kind: "loading" });

    const columns: string[] = [];
    const rows: Record<string, unknown>[] = [];
    let buffer: Record<string, unknown>[] = [];

    const onEvent = new Channel<{ event: string; data: Record<string, unknown> }>();
    let flushTimer: ReturnType<typeof setInterval> | null = null;

    const flush = () => {
      if (buffer.length > 0) {
        rows.push(...buffer);
        buffer = [];
      }
    };

    onEvent.onmessage = (msg) => {
      switch (msg.event) {
        case "columns":
          columns.push(...(msg.data as unknown as { names: string[] }).names);
          flushTimer = setInterval(flush, 50);
          break;

        case "row": {
          const values = (msg.data as unknown as { values: unknown[] }).values;
          const row: Record<string, unknown> = {};
          columns.forEach((col, i) => { row[col] = values[i]; });
          buffer.push(row);
          break;
        }

        case "done": {
          flush();
          if (flushTimer) clearInterval(flushTimer);
          const { row_count, duration_ms } = msg.data as unknown as {
            row_count: number;
            duration_ms: number;
          };
          const result: ResultPaneState =
            rows.length === 0
              ? { kind: "empty", durationMs: duration_ms }
              : { kind: "results", columns, rows, rowCount: row_count, durationMs: duration_ms };
          this._setTabExecuting(connectionId, queryTabId, false, result);
          break;
        }

        case "rowsAffected": {
          if (flushTimer) clearInterval(flushTimer);
          const { count, duration_ms: dms } = msg.data as unknown as {
            count: number;
            duration_ms: number;
          };
          this._setTabExecuting(connectionId, queryTabId, false, {
            kind: "rowsAffected",
            count,
            durationMs: dms,
          });
          break;
        }

        case "error": {
          if (flushTimer) clearInterval(flushTimer);
          const { message } = msg.data as unknown as { message: string };
          this._setTabExecuting(connectionId, queryTabId, false, { kind: "error", message });
          break;
        }
      }
    };

    try {
      await invoke("execute_query", { connectionId, sql, onEvent });
    } catch (e) {
      if (flushTimer) clearInterval(flushTimer);
      this._setTabExecuting(connectionId, queryTabId, false, {
        kind: "error",
        message: String(e),
      });
    }
  },

  _setTabExecuting(
    connectionId: string,
    queryTabId: string,
    isExecuting: boolean,
    result: ResultPaneState
  ) {
    connectionTabs = connectionTabs.map((ct) =>
      ct.connectionId === connectionId
        ? {
            ...ct,
            queryTabs: ct.queryTabs.map((qt) =>
              qt.id === queryTabId ? { ...qt, isExecuting, result } : qt
            ),
          }
        : ct
    );
  },

  // ── Persistence ──────────────────────────────────────────────────────────────

  async saveState() {
    const state: PersistedTabState = {
      activeConnectionId,
      connectionTabs: connectionTabs.map((ct) => ({
        connectionId: ct.connectionId,
        activeQueryTabId: ct.activeQueryTabId,
        queryTabs: ct.queryTabs.map((qt) => ({
          id: qt.id,
          label: qt.label,
          sql: qt.sql,
        })),
      })),
    };
    try {
      await invoke("save_tab_state", { tabState: state });
    } catch (e) {
      console.warn("Failed to save tab state:", e);
    }
  },

  async restoreState(connectRaw: (id: string) => Promise<void>) {
    let persisted: PersistedTabState | null = null;
    try {
      persisted = await invoke<PersistedTabState | null>("get_tab_state");
    } catch {
      return;
    }
    if (!persisted || persisted.connectionTabs.length === 0) return;

    // Restore tabs in "connecting" state, then connect each async
    connectionTabs = persisted.connectionTabs.map((ct) => ({
      connectionId: ct.connectionId,
      activeQueryTabId: ct.activeQueryTabId,
      status: "connecting" as ConnectionStatus,
      error: null,
      queryTabs: ct.queryTabs.map((qt) => ({
        id: qt.id,
        label: qt.label,
        sql: qt.sql,
        isDirty: false,
        result: { kind: "idle" as const },
        isExecuting: false,
      })),
    }));
    activeConnectionId = persisted.activeConnectionId ?? persisted.connectionTabs[0]?.connectionId ?? null;

    // Connect each in background — non-blocking
    for (const ct of persisted.connectionTabs) {
      connectRaw(ct.connectionId)
        .then(() => {
          this.setConnectionStatus(ct.connectionId, "connected");
        })
        .catch((e: unknown) => {
          this.setConnectionStatus(ct.connectionId, "error", String(e));
        });
    }
  },
};
