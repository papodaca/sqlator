import { api } from "$lib/api";
import type { SchemaInfo, TableInfo, SchemaColumnInfo } from "$lib/types";

// Per-connection schema state
interface ConnectionSchemaState {
  schemas: SchemaInfo[];
  activeSchema: string | null;
  tables: TableInfo[];
  columns: Record<string, SchemaColumnInfo[]>; // tableName -> columns
  isLoadingSchemas: boolean;
  isLoadingTables: boolean;
  loadingColumns: string[];
  error: string | null;
}

let schemaState = $state<Record<string, ConnectionSchemaState>>({});

const emptyState: ConnectionSchemaState = {
  schemas: [],
  activeSchema: null,
  tables: [],
  columns: {},
  isLoadingSchemas: false,
  isLoadingTables: false,
  loadingColumns: [],
  error: null,
};

function ensureInit(connectionId: string): ConnectionSchemaState {
  if (!schemaState[connectionId]) {
    schemaState[connectionId] = {
      schemas: [],
      activeSchema: null,
      tables: [],
      columns: {},
      isLoadingSchemas: false,
      isLoadingTables: false,
      loadingColumns: [],
      error: null,
    };
  }
  return schemaState[connectionId];
}

export const schemaStore = {
  /** Read-only access — safe to call from $derived */
  getState(connectionId: string): ConnectionSchemaState {
    return schemaState[connectionId] ?? emptyState;
  },

  async loadSchemas(connectionId: string) {
    const state = ensureInit(connectionId);
    if (state.isLoadingSchemas) return;
    state.isLoadingSchemas = true;
    state.error = null;
    try {
      const schemas = await api.invoke<SchemaInfo[]>("get_schemas", { connectionId });
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
    const state = ensureInit(connectionId);
    if (state.isLoadingTables) return;
    state.isLoadingTables = true;
    state.error = null;
    try {
      const tables = await api.invoke<TableInfo[]>("get_tables", {
        connectionId,
        schema: schema ?? state.activeSchema ?? undefined,
      });
      state.tables = tables;
      state.columns = {}; // clear cached columns on table reload
    } catch (e) {
      state.error = String(e);
    } finally {
      state.isLoadingTables = false;
    }
  },

  async loadColumns(connectionId: string, tableName: string, schema?: string) {
    const state = ensureInit(connectionId);
    if (state.loadingColumns.includes(tableName)) return;
    if (tableName in state.columns) return; // already cached

    state.loadingColumns = [...state.loadingColumns, tableName];
    try {
      const cols = await api.invoke<SchemaColumnInfo[]>("get_columns", {
        connectionId,
        tableName,
        schema: schema ?? state.activeSchema ?? undefined,
      });
      state.columns = { ...state.columns, [tableName]: cols };
    } catch (e) {
      state.error = String(e);
    } finally {
      state.loadingColumns = state.loadingColumns.filter((n) => n !== tableName);
    }
  },

  async setSchema(connectionId: string, schema: string) {
    const state = ensureInit(connectionId);
    state.activeSchema = schema;
    await schemaStore.loadTables(connectionId, schema);
  },

  async refresh(connectionId: string) {
    const state = ensureInit(connectionId);
    state.columns = {}; // clear column cache
    state.activeSchema = null; // reset so loadSchemas picks the default
    await schemaStore.loadSchemas(connectionId);
  },

  clearConnection(connectionId: string) {
    delete schemaState[connectionId];
  },
};
