import type {
  AddedRow,
  BatchResult,
  CellValue,
  ChangeSet,
  ColumnName,
  ModifiedRow,
  PkValue,
  SqlBatch,
  TableMeta,
  TempRowId,
} from "$lib/types";
import { api } from "$lib/api";
import { generateBatch } from "$lib/services/sql-generator";

// Serialize PkValue to a stable string key
export function pkToKey(pk: PkValue): string {
  if (Array.isArray(pk)) return JSON.stringify(pk);
  return JSON.stringify(pk);
}

function extractPkValue(row: Record<string, unknown>, pkColumns: string[]): PkValue {
  if (pkColumns.length === 1) {
    return row[pkColumns[0]] as CellValue;
  }
  return pkColumns.map((col) => row[col] as CellValue);
}

let tempIdCounter = 0;
function generateTempId(): TempRowId {
  return `temp_${++tempIdCounter}`;
}

// ── Edit Store ────────────────────────────────────────────────────────────────

class EditStore {
  changeSet = $state<ChangeSet>({
    added: new Map(),
    modified: new Map(),
    deleted: new Set(),
  });

  tableMeta = $state<TableMeta | null>(null);
  connectionId = $state<string | null>(null);
  dbType = $state<string>("postgres");
  lastSql = $state<string>("");

  // Which query tab's results this store is tracking
  queryTabId = $state<string | null>(null);

  hasChanges = $derived(
    this.changeSet.added.size > 0 ||
      this.changeSet.modified.size > 0 ||
      this.changeSet.deleted.size > 0,
  );

  changeCount = $derived(
    this.changeSet.added.size +
      this.changeSet.modified.size +
      this.changeSet.deleted.size,
  );

  isEditable = $derived(this.tableMeta?.isEditable ?? false);

  editabilityReason = $derived(
    this.tableMeta && !this.tableMeta.isEditable
      ? (this.tableMeta.editabilityReason ?? "Not editable")
      : null,
  );

  reset(connectionId: string, dbType: string, sql: string, queryTabId: string) {
    this.changeSet = {
      added: new Map(),
      modified: new Map(),
      deleted: new Set(),
    };
    this.tableMeta = null;
    this.connectionId = connectionId;
    this.dbType = dbType;
    this.lastSql = sql;
    this.queryTabId = queryTabId;
    tempIdCounter = 0;
  }

  setTableMeta(meta: TableMeta | null) {
    this.tableMeta = meta;
  }

  // ── Cell operations ─────────────────────────────────────────────────────────

  modifyCell(row: Record<string, unknown>, columnName: ColumnName, newValue: CellValue) {
    if (!this.tableMeta?.primaryKey.exists) return;
    const pkColumns = this.tableMeta.primaryKey.columns;
    const pkValue = extractPkValue(row, pkColumns);
    const pkKey = pkToKey(pkValue);

    const oldValue = row[columnName] as CellValue;

    // Get or create modified row entry
    let modifiedRow = this.changeSet.modified.get(pkKey);
    if (!modifiedRow) {
      modifiedRow = {
        primaryKey: pkValue,
        changes: new Map(),
      } satisfies ModifiedRow;
    }

    if (newValue === oldValue) {
      // Revert this cell
      modifiedRow.changes.delete(columnName);
      if (modifiedRow.changes.size === 0) {
        const next = new Map(this.changeSet.modified);
        next.delete(pkKey);
        this.changeSet = { ...this.changeSet, modified: next };
        return;
      }
    } else {
      modifiedRow.changes.set(columnName, { oldValue, newValue });
    }

    const next = new Map(this.changeSet.modified);
    next.set(pkKey, modifiedRow);
    this.changeSet = { ...this.changeSet, modified: next };
  }

  addRow(): TempRowId {
    const tempId = generateTempId();
    const added = new Map(this.changeSet.added);
    added.set(tempId, { tempId, data: {} });
    this.changeSet = { ...this.changeSet, added };
    return tempId;
  }

  modifyAddedCell(tempId: TempRowId, columnName: ColumnName, newValue: CellValue) {
    const row = this.changeSet.added.get(tempId);
    if (!row) return;
    const added = new Map(this.changeSet.added);
    added.set(tempId, { ...row, data: { ...row.data, [columnName]: newValue } });
    this.changeSet = { ...this.changeSet, added };
  }

  deleteRow(row: Record<string, unknown>) {
    if (!this.tableMeta?.primaryKey.exists) return;
    const pkValue = extractPkValue(row, this.tableMeta.primaryKey.columns);
    const pkKey = pkToKey(pkValue);

    // If the row was modified, remove those modifications
    const modified = new Map(this.changeSet.modified);
    modified.delete(pkKey);

    const deleted = new Set(this.changeSet.deleted);
    deleted.add(pkKey);
    this.changeSet = { ...this.changeSet, modified, deleted };
  }

  deleteAddedRow(tempId: TempRowId) {
    const added = new Map(this.changeSet.added);
    added.delete(tempId);
    this.changeSet = { ...this.changeSet, added };
  }

  undoDeleteRow(rowKey: string) {
    const deleted = new Set(this.changeSet.deleted);
    deleted.delete(rowKey);
    this.changeSet = { ...this.changeSet, deleted };
  }

  discardAllChanges() {
    this.changeSet = {
      added: new Map(),
      modified: new Map(),
      deleted: new Set(),
    };
  }

  // ── Query helpers ────────────────────────────────────────────────────────────

  isCellModified(row: Record<string, unknown>, columnName: ColumnName): boolean {
    if (!this.tableMeta?.primaryKey.exists) return false;
    const pkKey = pkToKey(extractPkValue(row, this.tableMeta.primaryKey.columns));
    return this.changeSet.modified.get(pkKey)?.changes.has(columnName) ?? false;
  }

  isRowDeleted(row: Record<string, unknown>): boolean {
    if (!this.tableMeta?.primaryKey.exists) return false;
    const pkKey = pkToKey(extractPkValue(row, this.tableMeta.primaryKey.columns));
    return this.changeSet.deleted.has(pkKey);
  }

  getCellDisplayValue(row: Record<string, unknown>, columnName: ColumnName): CellValue {
    if (!this.tableMeta?.primaryKey.exists) return row[columnName] as CellValue;
    const pkKey = pkToKey(extractPkValue(row, this.tableMeta.primaryKey.columns));
    const change = this.changeSet.modified.get(pkKey)?.changes.get(columnName);
    if (change !== undefined) return change.newValue;
    return row[columnName] as CellValue;
  }

  // ── SQL generation & execution ───────────────────────────────────────────────

  generateBatch(): SqlBatch | null {
    if (!this.tableMeta || !this.hasChanges) return null;
    return generateBatch(this.changeSet, this.tableMeta, this.dbType);
  }

  async executeBatch(batch: SqlBatch): Promise<BatchResult> {
    if (!this.connectionId) throw new Error("No connection");
    const result = await api.invoke<BatchResult>("execute_batch", {
      connectionId: this.connectionId,
      batch,
    });
    if (result.success) {
      this.discardAllChanges();
    }
    return result;
  }
}

export const editStore = new EditStore();
