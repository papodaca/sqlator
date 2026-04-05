import { invoke, Channel } from "@tauri-apps/api/core";
import type { ResultPaneState } from "$lib/types";

let resultState = $state<ResultPaneState>({ kind: "idle" });
let isExecuting = $state(false);

export const query = {
  get result() {
    return resultState;
  },
  get isExecuting() {
    return isExecuting;
  },

  async execute(connectionId: string, sql: string) {
    if (!sql.trim() || isExecuting) return;

    isExecuting = true;
    resultState = { kind: "loading" };

    const columns: string[] = [];
    const rows: Record<string, unknown>[] = [];
    let buffer: Record<string, unknown>[] = [];

    // Set up streaming channel
    const onEvent = new Channel<{
      event: string;
      data: Record<string, unknown>;
    }>();

    // Batch flush timer
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
          // Start batch flushing
          flushTimer = setInterval(flush, 50);
          break;

        case "row": {
          const values = (msg.data as unknown as { values: unknown[] })
            .values;
          const row: Record<string, unknown> = {};
          columns.forEach((col, i) => {
            row[col] = values[i];
          });
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
          if (rows.length === 0) {
            resultState = { kind: "empty", durationMs: duration_ms };
          } else {
            resultState = {
              kind: "results",
              columns,
              rows,
              rowCount: row_count,
              durationMs: duration_ms,
            };
          }
          isExecuting = false;
          break;
        }

        case "rowsAffected": {
          if (flushTimer) clearInterval(flushTimer);
          const { count, duration_ms: dms } = msg.data as unknown as {
            count: number;
            duration_ms: number;
          };
          resultState = { kind: "rowsAffected", count, durationMs: dms };
          isExecuting = false;
          break;
        }

        case "error": {
          if (flushTimer) clearInterval(flushTimer);
          const { message } = msg.data as unknown as { message: string };
          resultState = { kind: "error", message };
          isExecuting = false;
          break;
        }
      }
    };

    try {
      await invoke("execute_query", {
        connectionId,
        sql,
        onEvent,
      });
    } catch (e) {
      if (flushTimer) clearInterval(flushTimer);
      resultState = { kind: "error", message: String(e) };
      isExecuting = false;
    }
  },

  reset() {
    resultState = { kind: "idle" };
    isExecuting = false;
  },
};
