export type ConnectionColorId =
  | "red"
  | "orange"
  | "yellow"
  | "green"
  | "teal"
  | "blue"
  | "violet"
  | "pink"
  | "slate"
  | "white";

export interface ConnectionInfo {
  id: string;
  name: string;
  color_id: ConnectionColorId;
  db_type: "postgres" | "mysql" | "sqlite";
  host: string;
  port: number;
  database: string;
  username: string;
  masked_url: string;
}

export interface ConnectionConfig {
  name: string;
  color_id: ConnectionColorId;
  url: string;
}

export type ConnectionStatus =
  | "disconnected"
  | "connecting"
  | "connected"
  | "error";

export type ResultPaneState =
  | { kind: "idle" }
  | { kind: "loading" }
  | {
      kind: "results";
      columns: string[];
      rows: Record<string, unknown>[];
      rowCount: number;
      durationMs: number;
    }
  | { kind: "empty"; durationMs: number }
  | { kind: "rowsAffected"; count: number; durationMs: number }
  | { kind: "error"; message: string };

export interface QueryEvent {
  event: "columns" | "row" | "done" | "rowsAffected" | "error";
  data: unknown;
}

// --- SSH ---

export interface SshHostEntry {
  alias: string;
  hostname: string;
  port: number;
  user: string | null;
  identity_file: string | null;
  proxy_jump: string | null;
}

export type SshAuthMethod = "key" | "password" | "agent";

export interface SshJumpHost {
  host: string;
  port: number;
  username: string;
  auth_method: SshAuthMethod;
  key_path: string | null;
}

/** Mirrors core::models::SshProfile — no secrets */
export interface SshProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_method: SshAuthMethod;
  key_path: string | null;
  proxy_jump: SshJumpHost[];
  local_port_binding: number | null;
  keepalive_interval: number | null;
}

/** Payload sent to save_ssh_profile / update_ssh_profile */
export interface SshProfileConfig {
  name: string;
  host: string;
  port: number;
  username: string;
  auth_method: SshAuthMethod;
  key_path: string | null;
  /** Provide to store/update; omit or empty to leave unchanged */
  password: string | null;
  /** Provide to store/update; omit or empty to leave unchanged */
  key_passphrase: string | null;
  proxy_jump: SshJumpHost[];
  local_port_binding: number | null;
  keepalive_interval: number | null;
}
