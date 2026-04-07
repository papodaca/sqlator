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
  db_type: "postgres" | "mysql" | "mariadb" | "sqlite";
  host: string;
  port: number;
  database: string;
  username: string;
  masked_url: string;
  ssh_profile_id?: string | null;
  group_id?: string | null;
}

export interface ConnectionConfig {
  name: string;
  color_id: ConnectionColorId;
  url: string;
  ssh_profile_id?: string | null;
  group_id?: string | null;
}

export interface ConnectionGroup {
  id: string;
  name: string;
  color: string | null;
  parent_group_id: string | null;
  order: number;
  collapsed: boolean;
}

export interface ParsedConnectionUrl {
  db_type: "postgres" | "mysql" | "mariadb" | "sqlite";
  host: string;
  port: number;
  database: string;
  username: string;
  password: string | null;
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

// --- Tabs ---

export interface QueryTab {
  id: string;
  label: string;
  sql: string;
  isDirty: boolean;
  result: ResultPaneState;
  isExecuting: boolean;
}

export interface ConnectionTab {
  connectionId: string;
  queryTabs: QueryTab[];
  activeQueryTabId: string | null;
  status: ConnectionStatus;
  error: string | null;
}

// --- Editable Grid: Schema Metadata ---

export type ColumnType =
  | 'integer' | 'bigint' | 'smallint'
  | 'decimal' | 'numeric' | 'float' | 'double'
  | 'varchar' | 'text' | 'char'
  | 'boolean'
  | 'date' | 'time' | 'datetime' | 'timestamp'
  | 'json' | 'jsonb'
  | 'uuid'
  | 'enum'
  | 'unknown';

export interface ColumnMeta {
  name: string;
  type: ColumnType;
  nullable: boolean;
  isAutoIncrement: boolean;
  isGenerated: boolean;
  isUpdatable: boolean;
  defaultValue?: string;
  enumValues?: string[];
}

export interface PrimaryKeyMeta {
  columns: string[];
  exists: boolean;
}

export interface TableMeta {
  tableName: string;
  schema?: string;
  columns: ColumnMeta[];
  primaryKey: PrimaryKeyMeta;
  isEditable: boolean;
  editabilityReason?: string;
}

// --- Editable Grid: Change Tracking ---

export type CellValue = string | number | boolean | null;
export type PkValue = CellValue | CellValue[];
export type TempRowId = `temp_${number}`;
export type ColumnName = string;

export interface CellChange {
  oldValue: CellValue;
  newValue: CellValue;
}

export interface AddedRow {
  tempId: TempRowId;
  data: Record<string, CellValue>;
}

export interface ModifiedRow {
  primaryKey: PkValue;
  changes: Map<ColumnName, CellChange>;
}

export interface ChangeSet {
  added: Map<TempRowId, AddedRow>;
  modified: Map<string, ModifiedRow>; // stringified PkValue as key
  deleted: Set<string>; // stringified PkValue
}

// --- Editable Grid: SQL Generation ---

export interface ParameterizedSql {
  sql: string;
  params: CellValue[];
  tempId?: TempRowId; // For INSERT statements, links result back to added row
}

export interface SqlBatch {
  statements: ParameterizedSql[];
  useTransaction: boolean;
}

export interface BatchError {
  statementIndex: number;
  message: string;
  code?: string;
}

export interface BatchResult {
  success: boolean;
  executedCount: number;
  totalStatements: number;
  error?: BatchError;
  insertedIds?: Record<string, CellValue>;
}

// --- Import / Export ---

export interface ImportResult {
  groups_added: number;
  profiles_added: number;
  connections_added: number;
  connections_skipped: number;
}

// --- Credential storage ---

export type StorageMode = "keyring" | "vault";

export interface VaultSettings {
  timeout_secs: number;
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
