import type {
  AddedRow,
  CellValue,
  ChangeSet,
  ModifiedRow,
  ParameterizedSql,
  PkValue,
  PrimaryKeyMeta,
  SqlBatch,
  TableMeta,
  TempRowId,
} from "$lib/types";

// ── Identifier quoting ────────────────────────────────────────────────────────

function quotePg(name: string): string {
  return `"${name.replace(/"/g, '""')}"`;
}

function quoteMySql(name: string): string {
  return `\`${name.replace(/`/g, "``")}\``;
}

// ── PK helpers ────────────────────────────────────────────────────────────────

function pkToArray(pk: PkValue): CellValue[] {
  return Array.isArray(pk) ? pk : [pk];
}

// ── Postgres generator ($1, $2, ... params + RETURNING) ───────────────────────

function pgBuildWhereClause(pk: PrimaryKeyMeta, pkValue: PkValue, startIdx: number): string {
  const pkArray = pkToArray(pkValue);
  return pk.columns
    .map((col, i) => `${quotePg(col)} = $${startIdx + i}`)
    .join(" AND ");
}

function pgGenerateInsert(table: TableMeta, tempId: TempRowId, row: AddedRow): ParameterizedSql {
  const updatableCols = Object.keys(row.data).filter((col) => {
    const colMeta = table.columns.find((c) => c.name === col);
    return colMeta ? colMeta.isUpdatable : true;
  });
  const params = updatableCols.map((col) => row.data[col]);
  const colList = updatableCols.map(quotePg).join(", ");
  const placeholders = params.map((_, i) => `$${i + 1}`).join(", ");
  const returning = table.primaryKey.columns.map(quotePg).join(", ");

  return {
    sql: `INSERT INTO ${quotePg(table.tableName)} (${colList}) VALUES (${placeholders}) RETURNING ${returning}`,
    params,
    tempId,
  };
}

function pgGenerateUpdate(table: TableMeta, modified: ModifiedRow): ParameterizedSql {
  const setClauses: string[] = [];
  const params: CellValue[] = [];
  let idx = 1;

  for (const [col, change] of modified.changes) {
    setClauses.push(`${quotePg(col)} = $${idx++}`);
    params.push(change.newValue);
  }

  const whereClause = pgBuildWhereClause(table.primaryKey, modified.primaryKey, idx);
  params.push(...pkToArray(modified.primaryKey));

  return {
    sql: `UPDATE ${quotePg(table.tableName)} SET ${setClauses.join(", ")} WHERE ${whereClause}`,
    params,
  };
}

function pgGenerateDelete(table: TableMeta, pkValue: PkValue): ParameterizedSql {
  const whereClause = pgBuildWhereClause(table.primaryKey, pkValue, 1);
  return {
    sql: `DELETE FROM ${quotePg(table.tableName)} WHERE ${whereClause}`,
    params: pkToArray(pkValue),
  };
}

// ── MySQL/SQLite generator (? params, no RETURNING) ───────────────────────────

function myBuildWhereClause(pk: PrimaryKeyMeta, quote: (s: string) => string): string {
  return pk.columns.map((col) => `${quote(col)} = ?`).join(" AND ");
}

function myGenerateInsert(
  table: TableMeta,
  tempId: TempRowId,
  row: AddedRow,
  quote: (s: string) => string,
): ParameterizedSql {
  const updatableCols = Object.keys(row.data).filter((col) => {
    const colMeta = table.columns.find((c) => c.name === col);
    return colMeta ? colMeta.isUpdatable : true;
  });
  const params = updatableCols.map((col) => row.data[col]);
  const colList = updatableCols.map(quote).join(", ");
  const placeholders = params.map(() => "?").join(", ");

  return {
    sql: `INSERT INTO ${quote(table.tableName)} (${colList}) VALUES (${placeholders})`,
    params,
    tempId,
  };
}

function myGenerateUpdate(
  table: TableMeta,
  modified: ModifiedRow,
  quote: (s: string) => string,
): ParameterizedSql {
  const setClauses: string[] = [];
  const params: CellValue[] = [];

  for (const [col, change] of modified.changes) {
    setClauses.push(`${quote(col)} = ?`);
    params.push(change.newValue);
  }

  const whereClause = myBuildWhereClause(table.primaryKey, quote);
  params.push(...pkToArray(modified.primaryKey));

  return {
    sql: `UPDATE ${quote(table.tableName)} SET ${setClauses.join(", ")} WHERE ${whereClause}`,
    params,
  };
}

function myGenerateDelete(
  table: TableMeta,
  pkValue: PkValue,
  quote: (s: string) => string,
): ParameterizedSql {
  const whereClause = myBuildWhereClause(table.primaryKey, quote);
  return {
    sql: `DELETE FROM ${quote(table.tableName)} WHERE ${whereClause}`,
    params: pkToArray(pkValue),
  };
}

// ── Public API ────────────────────────────────────────────────────────────────

export function generateBatch(
  changeSet: ChangeSet,
  tableMeta: TableMeta,
  dbType: string,
): SqlBatch {
  const statements: ParameterizedSql[] = [];
  const isPostgres = dbType === "postgres";
  const quote = dbType === "mysql" || dbType === "mariadb" ? quoteMySql : quotePg;

  // Order: DELETEs first (avoid FK issues), then UPDATEs, then INSERTs
  for (const pkKey of changeSet.deleted) {
    const pkValue = JSON.parse(pkKey) as PkValue;
    statements.push(
      isPostgres
        ? pgGenerateDelete(tableMeta, pkValue)
        : myGenerateDelete(tableMeta, pkValue, quote),
    );
  }

  for (const [, modifiedRow] of changeSet.modified) {
    statements.push(
      isPostgres
        ? pgGenerateUpdate(tableMeta, modifiedRow)
        : myGenerateUpdate(tableMeta, modifiedRow, quote),
    );
  }

  for (const [tempId, addedRow] of changeSet.added) {
    statements.push(
      isPostgres
        ? pgGenerateInsert(tableMeta, tempId, addedRow)
        : myGenerateInsert(tableMeta, tempId, addedRow, quote),
    );
  }

  return { statements, useTransaction: true };
}

/** Format a batch as readable SQL for preview (with param annotations) */
export function formatBatchForPreview(batch: SqlBatch): string {
  return batch.statements
    .map(
      (s, i) =>
        `-- Statement ${i + 1}\n${s.sql}\n-- Params: [${s.params.map((p) => (p === null ? "NULL" : JSON.stringify(p))).join(", ")}]`,
    )
    .join("\n\n");
}
