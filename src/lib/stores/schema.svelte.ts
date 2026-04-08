import { invoke } from "@tauri-apps/api/core";
import type { SchemaInfo, TableInfo, SchemaColumnInfo } from "$lib/types";

// Per-connection schema state
interface ConnectionSchemaState {
  schemas: SchemaInfo[];
  activeSchema: string | null;
  tables: TableInfo[];
  columns: Map<string, SchemaColumnInfo[]>; // tableName -> columns
  isLoadingSchemas: boolean;
  isLoadingTables: boolean;
  loadingColumns: Set<string>;
  error: string | null;
}

let schemaState = $state<Map<string, ConnectionSchemaState>>(new Map());

function getOrInit(connectionId: string): ConnectionSchemaState {
  if (!schemaState.has(connectionId)) {
    schemaState.set(connectionId, {
      schemas: [],
      activeSchema: null,
      tables: [],
      columns: new Map(),
      isLoadingSchemas: false,
      isLoadingTables: false,
      loadingColumns: new Set(),
      error: null,
    });
  }
  return schemaState.get(connectionId)!;
}

export const schemaStore = {
  getState(connectionId: string): ConnectionSchemaState {
    return getOrInit(connectionId);
  },

  async loadSchemas(connectionId: string) {
    const state = getOrInit(connectionId);
    if (state.isLoadingSchemas) return;
    state.isLoadingSchemas = true;
    state.error = null;
    try {
      const schemas = await invoke<SchemaInfo[]>("get_schemas", { connectionId });
      state.schemas = schemas;
      // Auto-select default schema
      const defaultSchema = schemas.find((s) => s.isDefault);
      if (!state.activeSchema) {
        state.activeSchema = defaultSchema?.name ?? schemas[0]?.name ?? null;
      }
      // Load tables for active schema
      if (state.activeSchema) {
        await schemaStore.loadTables(connectionId, state.activeSchema);
      }
    } catch (e) {
      state.error = String(e);
    } finally {
      state.isLoadingSchemas = false;
    }
  },

  async loadTables(connectionId: string, schema?: string) {
    const state = getOrInit(connectionId);
    if (state.isLoadingTables) return;
    state.isLoadingTables = true;
    state.error = null;
    try {
      const tables = await invoke<TableInfo[]>("get_tables", {
        connectionId,
        schema: schema ?? state.activeSchema ?? undefined,
      });
      state.tables = tables;
      state.columns = new Map(); // clear cached columns on table reload
    } catch (e) {
      state.error = String(e);
    } finally {
      state.isLoadingTables = false;
    }
  },

  async loadColumns(connectionId: string, tableName: string, schema?: string) {
    const state = getOrInit(connectionId);
    if (state.loadingColumns.has(tableName)) return;
    if (state.columns.has(tableName)) return; // already cached

    state.loadingColumns.add(tableName);
    try {
      const cols = await invoke<SchemaColumnInfo[]>("get_columns", {
        connectionId,
        tableName,
        schema: schema ?? state.activeSchema ?? undefined,
      });
      state.columns.set(tableName, cols);
    } catch (e) {
      state.error = String(e);
    } finally {
      state.loadingColumns.delete(tableName);
    }
  },

  async setSchema(connectionId: string, schema: string) {
    const state = getOrInit(connectionId);
    state.activeSchema = schema;
    await schemaStore.loadTables(connectionId, schema);
  },

  async refresh(connectionId: string) {
    const state = getOrInit(connectionId);
    state.columns = new Map(); // clear column cache
    state.activeSchema = null; // reset so loadSchemas picks the default
    await schemaStore.loadSchemas(connectionId);
  },

  clearConnection(connectionId: string) {
    schemaState.delete(connectionId);
  },
};
