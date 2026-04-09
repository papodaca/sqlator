/** Streaming event from execute_query */
export interface StreamEvent {
  event: string;
  data: Record<string, unknown>;
}

/**
 * Abstraction over Tauri IPC (desktop) and HTTP/WS (web server).
 * All stores and components should use this interface instead of
 * importing directly from @tauri-apps/api.
 */
export interface ApiAdapter {
  /**
   * Invoke a backend command. Equivalent to Tauri's `invoke()`.
   * In web mode, maps to a POST /api/<command> fetch call.
   */
  invoke<T = void>(command: string, args?: Record<string, unknown>): Promise<T>;

  /**
   * Execute a SQL query with streaming results.
   * Replaces the Tauri Channel pattern for execute_query.
   * In web mode, uses a WebSocket to /api/query.
   */
  executeQueryStream(
    connectionId: string,
    sql: string,
    onEvent: (msg: StreamEvent) => void,
  ): Promise<void>;

  /**
   * Open a filesystem path. Used after export_connections.
   * In web mode, triggers a file download instead.
   */
  openPath(path: string): void;
}
