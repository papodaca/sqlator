---
title: "feat: Editable Datagrid with SQL Generation"
type: feat
status: active
date: 2026-04-04
origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md
---

# ✨ Editable Datagrid with SQL Generation

Transform the read-only result grid into an editable grid that generates INSERT/UPDATE/DELETE SQL statements based on user modifications. Users can edit cells inline, add/delete rows, preview generated SQL, and execute with Ctrl/Cmd+S.

---

## Overview

Building on the MVP's read-only result grid (TanStack Virtual, max 1000 rows), this feature adds:

1. **Inline Cell Editing** — Double-click or Enter to edit cells directly
2. **Row Operations** — Add new rows, delete existing rows via multiple entry points
3. **Schema Metadata Fetching** — Detect tables, columns, primary keys, and column types
4. **SQL Generation** — Dialect-aware INSERT/UPDATE/DELETE with parameterized queries
5. **Batch Preview & Execute** — Accumulate changes, preview SQL, execute on Ctrl/Cmd+S

---

## Problem Statement

The MVP grid is read-only — users must manually write INSERT/UPDATE/DELETE statements in the SQL editor. For quick data corrections or small datasets, this is tedious. A spreadsheet-like editing experience with automatic SQL generation dramatically improves productivity for common data manipulation tasks.

**Real-world scenarios:**
- Fix a typo in a customer's email address
- Delete a few obsolete records
- Add a handful of new rows with test data
- Update status flags across multiple rows

---

## Proposed Solution

A multi-layered architecture that:

1. **Fetches schema metadata** after successful SELECT to identify:
   - Source table(s) and their primary key columns
   - Column types, nullability, auto-increment flags
   - Computed/generated columns (read-only)

2. **Tracks changes** in frontend state:
   - Added rows (temp IDs, partial data)
   - Modified cells (original + new values per PK)
   - Deleted rows (PK values only)

3. **Generates SQL** on Ctrl/Cmd+S:
   - Parameterized queries (no SQL injection risk)
   - Dialect-specific syntax (Postgres `RETURNING`, etc.)
   - Wrapped in a transaction for atomic execution

4. **Previews & executes** via modal:
   - Syntax-highlighted SQL preview
   - Execute or cancel options
   - Error handling with rollback

---

## Technical Approach

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Svelte 5 Frontend                          │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  ResultGrid.svelte (Enhanced)                             │  │
│  │  ┌────────────────────────────────────────────────────┐  │  │
│  │  │  Virtual Grid (TanStack)                           │  │  │
│  │  │  • Inline cell editors (type-aware)                │  │  │
│  │  │  • Row selection + context menu                    │  │  │
│  │  │  • Change visualization (color indicators)         │  │  │
│  │  │  • Empty row at bottom for INSERT                  │  │  │
│  │  └────────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────────┐  │  │
│  │  │  GridToolbar.svelte                                │  │  │
│  │  │  • "Add Row" button                                │  │  │
│  │  │  • "Discard All Changes" button                    │  │  │
│  │  │  • Pending changes count badge                     │  │  │
│  │  └────────────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                  │
│  ┌───────────────────────────┼──────────────────────────────┐  │
│  │  stores/edit.svelte.ts    │  stores/schema.svelte.ts     │  │
│  │  • changeSet: Added/      │  • tableMeta: TableMeta |    │  │
│  │    Modified/Deleted       │    null                      │  │
│  │  • hasChanges: boolean    │  • editability: 'full' |     │  │
│  │  • sqlPreview: string     │    'readonly' | 'partial'    │  │
│  └───────────────────────────┴──────────────────────────────┘  │
│                              │                                  │
│              invoke() (IPC)  │  Channel<SchemaEvent>            │
└──────────────────────────────┼──────────────────────────────────┘
                               │
┌──────────────────────────────┼──────────────────────────────────┐
│           Tauri 2 Rust Backend                                   │
│                              │                                   │
│  ┌───────────────────────────▼───────────────────────────────┐  │
│  │  Commands:                                                 │  │
│  │  • fetch_schema_metadata(query, conn_id) → TableMeta       │  │
│  │  • execute_batch(sql_batch, conn_id) → BatchResult        │  │
│  └───────────────────────────────────────────────────────────┘  │
│                              │                                   │
│  ┌───────────────────────────▼───────────────────────────────┐  │
│  │  Schema Detector (new)                                     │  │
│  │  • Parse SELECT to find table sources                      │  │
│  │  • Query information_schema for PKs, types                 │  │
│  │  • Cache per-connection with TTL (5 min)                   │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  SQL Generator (new)                                       │  │
│  │  • PostgresGenerator: RETURNING, $1/$2 params              │  │
│  │  • MySqlGenerator: ? params, re-query for inserted IDs     │  │
│  │  • SqliteGenerator: ? params, re-query for inserted IDs    │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  sqlx AnyPool (existing)                                   │  │
│  │  • Transaction support for batch execution                 │  │
│  │  • Parameterized queries for safety                        │  │
│  └───────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### Schema Metadata Model

```typescript
// src/lib/types.ts

export interface TableMeta {
  tableName: string;
  schema?: string;           // Postgres schema, MySQL database
  columns: ColumnMeta[];
  primaryKey: PrimaryKeyMeta;
  isEditable: boolean;       // false for views, CTEs, joins
  editabilityReason?: string; // "Multi-table join", "View without PK", etc.
}

export interface ColumnMeta {
  name: string;
  type: ColumnType;
  nullable: boolean;
  isAutoIncrement: boolean;
  isGenerated: boolean;      // Computed/generated column
  isUpdatable: boolean;      // false for auto-increment, generated
  defaultValue?: string;
  enumValues?: string[];     // For ENUM types
}

export interface PrimaryKeyMeta {
  columns: string[];         // Composite PK support
  exists: boolean;
}

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
```

### Change Tracking Model

```typescript
// src/lib/stores/edit.svelte.ts

export interface ChangeSet {
  added: Map<TempRowId, AddedRow>;
  modified: Map<PkValue, ModifiedRow>;
  deleted: Set<PkValue>;
}

export interface AddedRow {
  tempId: TempRowId;
  data: Record<string, CellValue>;
}

export interface ModifiedRow {
  primaryKey: PkValue;
  changes: Map<ColumnName, CellChange>;
}

export interface CellChange {
  oldValue: CellValue;
  newValue: CellValue;
}

export type CellValue = string | number | boolean | null;
export type PkValue = CellValue | CellValue[];  // Composite PK support
export type TempRowId = `temp_${number}`;
export type ColumnName = string;
```

### SQL Generation Model

```typescript
// src/lib/types.ts

export interface ParameterizedSql {
  sql: string;               // "UPDATE users SET name = $1, email = $2 WHERE id = $3"
  params: CellValue[];       // ["Alice", "alice@example.com", 42]
}

export interface SqlBatch {
  statements: ParameterizedSql[];
  useTransaction: boolean;   // Always true for MVP
}

export interface BatchResult {
  success: boolean;
  executedCount: number;
  totalStatements: number;
  error?: BatchError;
  insertedIds?: Map<TempRowId, PkValue>;  // For INSERT RETURNING
}

export interface BatchError {
  statementIndex: number;
  message: string;
  code?: string;             // FK violation, unique constraint, etc.
}
```

---

## Implementation Phases

### Phase 1: Schema Metadata Fetching

**Goal:** Detect source table, primary keys, and column metadata from SELECT queries.

**Rust commands:**

```rust
// src-tauri/src/commands/schema.rs

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMeta {
    pub table_name: String,
    pub schema: Option<String>,
    pub columns: Vec<ColumnMeta>,
    pub primary_key: PrimaryKeyMeta,
    pub is_editable: bool,
    pub editability_reason: Option<String>,
}

#[tauri::command]
pub async fn fetch_schema_metadata(
    state: State<'_, AppState>,
    connection_id: String,
    query: String,
) -> Result<Option<TableMeta>, CommandError> {
    // 1. Parse query to extract table references (simple regex or sqlparser crate)
    // 2. If multiple tables → return is_editable: false with reason
    // 3. If view/CTE → check if underlying table has PK
    // 4. Query information_schema for:
    //    - Column names, types, nullability
    //    - Primary key columns
    //    - Auto-increment columns
    //    - Generated/computed columns
    // 5. Cache in AppState with 5-minute TTL
}
```

**Schema queries by database type:**

```sql
-- Postgres
SELECT 
    c.column_name,
    c.data_type,
    c.is_nullable,
    c.column_default,
    c.is_identity,
    CASE WHEN c.is_generated = 'ALWAYS' THEN true ELSE false END as is_generated
FROM information_schema.columns c
WHERE c.table_schema = $1 AND c.table_name = $2;

SELECT kcu.column_name
FROM information_schema.table_constraints tc
JOIN information_schema.key_column_usage kcu 
    ON tc.constraint_name = kcu.constraint_name
WHERE tc.constraint_type = 'PRIMARY KEY'
    AND tc.table_schema = $1 
    AND tc.table_name = $2
ORDER BY kcu.ordinal_position;

-- MySQL
SELECT 
    c.COLUMN_NAME,
    c.DATA_TYPE,
    c.IS_NULLABLE,
    c.COLUMN_DEFAULT,
    c.EXTRA LIKE '%auto_increment%' as is_auto_increment,
    c.EXTRA LIKE '%GENERATED%' as is_generated
FROM information_schema.columns c
WHERE c.TABLE_SCHEMA = DATABASE() AND c.TABLE_NAME = ?;

SELECT kcu.COLUMN_NAME
FROM information_schema.TABLE_CONSTRAINTS tc
JOIN information_schema.KEY_COLUMN_USAGE kcu
    ON tc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME
WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
    AND tc.TABLE_SCHEMA = DATABASE()
    AND tc.TABLE_NAME = ?
ORDER BY kcu.ORDINAL_POSITION;

-- SQLite
PRAGMA table_info(table_name);  -- cid, name, type, notnull, dflt_value, pk
```

**Frontend components:**

- `src/lib/stores/schema.svelte.ts` — `$state` store for `TableMeta | null`
- `src/lib/services/schema-fetcher.ts` — Invoke `fetch_schema_metadata` after successful SELECT

**Success criteria:**
- [ ] After SELECT, `TableMeta` is fetched and stored
- [ ] Primary key columns are correctly identified
- [ ] Auto-increment and generated columns marked as read-only
- [ ] Multi-table joins return `isEditable: false` with reason
- [ ] Schema cached for 5 minutes per connection

**Key files:**
- `src-tauri/src/commands/schema.rs`
- `src-tauri/src/schema_cache.rs`
- `src/lib/stores/schema.svelte.ts`
- `src/lib/services/schema-fetcher.ts`

---

### Phase 2: Change Tracking State

**Goal:** Track added rows, modified cells, and deleted rows in reactive state.

**Frontend store:**

```typescript
// src/lib/stores/edit.svelte.ts

class EditStore {
  // Reactive state
  changeSet = $state<ChangeSet>({
    added: new Map(),
    modified: new Map(),
    deleted: new Set(),
  });
  
  tableMeta = $state<TableMeta | null>(null);
  
  // Computed
  hasChanges = $derived(
    this.changeSet.added.size > 0 ||
    this.changeSet.modified.size > 0 ||
    this.changeSet.deleted.size > 0
  );
  
  changeCount = $derived(
    this.changeSet.added.size +
    this.changeSet.modified.size +
    this.changeSet.deleted.size
  );
  
  // Methods
  modifyCell(pkValue: PkValue, columnName: string, newValue: CellValue) { ... }
  addRow(data: Record<string, CellValue>) { ... }
  deleteRow(pkValue: PkValue) { ... }
  discardAllChanges() { ... }
  isCellModified(pkValue: PkValue, columnName: string): boolean { ... }
  getCellDisplayValue(row: Row, columnName: string): CellValue { ... }
}
```

**Row identification logic:**

```typescript
// Extract PK value from a result row
function extractPkValue(row: Row, pkColumns: string[]): PkValue {
  if (pkColumns.length === 1) {
    return row[pkColumns[0]];
  }
  return pkColumns.map(col => row[col]);
}

// For new rows, generate temp ID
let tempIdCounter = 0;
function generateTempId(): TempRowId {
  return `temp_${++tempIdCounter}`;
}
```

**Success criteria:**
- [ ] Cell edits tracked with old/new values
- [ ] Added rows have temp IDs
- [ ] Deleted rows tracked by PK value
- [ ] `hasChanges` computed correctly
- [ ] `discardAllChanges` clears all state
- [ ] Original values preserved for revert

**Key files:**
- `src/lib/stores/edit.svelte.ts`
- `src/lib/types.ts`

---

### Phase 3: Inline Cell Editing

**Goal:** Enable inline editing with type-aware cell editors.

**Cell editor types:**

| Column Type | Editor Component | Behavior |
|-------------|------------------|----------|
| `boolean` | `BooleanEditor.svelte` | Dropdown: true / false / null |
| `date` | `DateEditor.svelte` | Native date picker (`<input type="date">`) |
| `datetime`, `timestamp` | `DateTimeEditor.svelte` | Native datetime picker |
| `enum` | `EnumEditor.svelte` | Dropdown with `enumValues` from metadata |
| `text`, `varchar` | `TextEditor.svelte` | Simple text input |
| `text` (length > 500) | `TextAreaEditor.svelte` | Expandable modal for long text |
| `integer`, `bigint`, `decimal` | `NumberEditor.svelte` | Number input with type validation |
| `json`, `jsonb` | `JsonEditor.svelte` | Text area with JSON validation |
| `uuid` | `TextEditor.svelte` | Text input with UUID format hint |

**Editor component pattern:**

```svelte
<!-- src/lib/components/editors/TextEditor.svelte -->
<script lang="ts">
  let { value, onSave, onCancel, readonly } = $props();
  
  let inputEl: HTMLInputElement;
  let localValue = $state(value ?? '');
  
  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      onSave(localValue);
    } else if (e.key === 'Escape') {
      onCancel();
    }
  }
  
  $effect(() => {
    inputEl?.focus();
    inputEl?.select();
  });
</script>

<input
  bind:this={inputEl}
  type="text"
  bind:value={localValue}
  onkeydown={handleKeydown}
  onblur={() => onSave(localValue)}
  {readonly}
  class="w-full bg-surface border border-accent px-1"
/>
```

**Grid cell rendering:**

```svelte
<!-- In ResultGrid.svelte -->
{#each visibleRows as row, rowIndex}
  <tr>
    {#each columns as column, colIndex}
      <td
        ondblclick={() => startEdit(row, column)}
        class:modified={editStore.isCellModified(pkValue, column.name)}
      >
        {#if editingCell?.row === rowIndex && editingCell?.col === colIndex}
          <CellEditor
            type={getColumnEditorType(column)}
            value={editStore.getCellDisplayValue(row, column.name)}
            onSave={(v) => saveCellEdit(row, column.name, v)}
            onCancel={cancelCellEdit}
            readonly={!column.isUpdatable}
          />
        {:else}
          <CellDisplay value={row[column.name]} type={column.type} />
        {/if}
      </td>
    {/each}
  </tr>
{/each}
```

**NULL vs empty string handling:**

```typescript
// Right-click on cell → context menu
const CELL_CONTEXT_ACTIONS = [
  { label: 'Edit', action: 'edit' },
  { label: 'Set to NULL', action: 'set-null', enabled: column.nullable },
  { label: 'Set to empty string', action: 'set-empty', enabled: isTextColumn(column) },
];
```

**Success criteria:**
- [ ] Double-click or Enter starts editing
- [ ] Escape cancels edit, reverts to original
- [ ] Enter or blur commits edit
- [ ] Type-specific editors for boolean, date, enum
- [ ] Read-only columns are not editable
- [ ] NULL vs empty string distinction supported
- [ ] Modified cells have yellow background

**Key files:**
- `src/lib/components/editors/TextEditor.svelte`
- `src/lib/components/editors/BooleanEditor.svelte`
- `src/lib/components/editors/DateEditor.svelte`
- `src/lib/components/editors/EnumEditor.svelte`
- `src/lib/components/editors/NumberEditor.svelte`
- `src/lib/components/editors/JsonEditor.svelte`
- `src/lib/components/editors/TextAreaEditor.svelte`

---

### Phase 4: Row Operations

**Goal:** Add/delete rows with multiple entry points.

**Add row entry points:**

1. **Empty row at grid bottom:**
   - Always-visible row with "+" icon
   - Click any cell to start adding
   - On first edit, becomes a real pending row

2. **Toolbar button:**
   ```svelte
   <button onclick={addRow} disabled={!tableMeta?.isEditable}>
     + Add Row
   </button>
   ```

3. **Context menu:**
   ```svelte
   <ContextMenu>
     <button onclick={addRow}>Add Row</button>
   </ContextMenu>
   ```

4. **Keyboard shortcut:**
   ```typescript
   // Ctrl/Cmd + N
   keymap.of([{ key: 'Mod-n', run: addRow }]);
   ```

**Delete row entry points:**

1. **Context menu:**
   ```svelte
   <ContextMenu>
     <button onclick={() => deleteRow(selectedRow)}>Delete Row</button>
   </ContextMenu>
   ```

2. **Keyboard shortcut:**
   ```typescript
   // Delete or Backspace key
   window.addEventListener('keydown', (e) => {
     if ((e.key === 'Delete' || e.key === 'Backspace') && selectedRow) {
       deleteRow(selectedRow);
     }
   });
   ```

**Row visualization:**

| Row State | Visual Indicator |
|-----------|------------------|
| Added | Green left border (4px), light green background |
| Modified (any cell) | Yellow background on modified cells |
| Deleted | Red background, strikethrough text, opacity 0.6 |
| Normal | Default styling |

**Success criteria:**
- [ ] Empty row at bottom for quick add
- [ ] Toolbar "Add Row" button works
- [ ] Context menu "Add Row" works
- [ ] Ctrl/Cmd+N keyboard shortcut works
- [ ] Context menu "Delete Row" works
- [ ] Delete/Backspace keyboard shortcut works
- [ ] Added rows show green border
- [ ] Deleted rows show red strikethrough
- [ ] Delete removes row from grid view (filtered out)

**Key files:**
- `src/lib/components/ResultGrid.svelte`
- `src/lib/components/GridToolbar.svelte`
- `src/lib/components/ContextMenu.svelte`

---

### Phase 5: SQL Generation

**Goal:** Generate parameterized INSERT/UPDATE/DELETE statements from change set.

**Frontend SQL generator:**

```typescript
// src/lib/services/sql-generator.ts

interface SqlGenerator {
  generateBatch(changeSet: ChangeSet, tableMeta: TableMeta): SqlBatch;
}

function createGenerator(dbType: DbType): SqlGenerator {
  switch (dbType) {
    case 'postgres': return new PostgresGenerator();
    case 'mysql': return new MySqlGenerator();
    case 'sqlite': return new SqliteGenerator();
  }
}

class PostgresGenerator implements SqlGenerator {
  generateBatch(changeSet: ChangeSet, tableMeta: TableMeta): SqlBatch {
    const statements: ParameterizedSql[] = [];
    
    // Order: DELETE first (avoid FK issues), then UPDATE, then INSERT
    for (const pkValue of changeSet.deleted) {
      statements.push(this.generateDelete(tableMeta, pkValue));
    }
    
    for (const [pkValue, modifiedRow] of changeSet.modified) {
      statements.push(this.generateUpdate(tableMeta, pkValue, modifiedRow));
    }
    
    for (const [tempId, addedRow] of changeSet.added) {
      statements.push(this.generateInsert(tableMeta, tempId, addedRow));
    }
    
    return { statements, useTransaction: true };
  }
  
  generateInsert(table: TableMeta, tempId: TempRowId, row: AddedRow): ParameterizedSql {
    const columns = Object.keys(row.data);
    const values = Object.values(row.data);
    const placeholders = values.map((_, i) => `$${i + 1}`).join(', ');
    const returning = table.primaryKey.columns.join(', ');
    
    return {
      sql: `INSERT INTO ${this.quoteIdentifier(table.tableName)} (${columns.map(this.quoteIdentifier).join(', ')}) VALUES (${placeholders}) RETURNING ${returning}`,
      params: values,
    };
  }
  
  generateUpdate(table: TableMeta, pkValue: PkValue, modified: ModifiedRow): ParameterizedSql {
    const setClauses: string[] = [];
    const params: CellValue[] = [];
    let paramIndex = 1;
    
    for (const [col, change] of modified.changes) {
      setClauses.push(`${this.quoteIdentifier(col)} = $${paramIndex++}`);
      params.push(change.newValue);
    }
    
    const whereClause = this.buildPkWhere(table.primaryKey, pkValue, paramIndex);
    params.push(...this.pkToArray(pkValue));
    
    return {
      sql: `UPDATE ${this.quoteIdentifier(table.tableName)} SET ${setClauses.join(', ')} WHERE ${whereClause}`,
      params,
    };
  }
  
  generateDelete(table: TableMeta, pkValue: PkValue): ParameterizedSql {
    const whereClause = this.buildPkWhere(table.primaryKey, pkValue, 1);
    
    return {
      sql: `DELETE FROM ${this.quoteIdentifier(table.tableName)} WHERE ${whereClause}`,
      params: this.pkToArray(pkValue),
    };
  }
  
  private quoteIdentifier(name: string): string {
    return `"${name.replace(/"/g, '""')}"`;
  }
  
  private buildPkWhere(pk: PrimaryKeyMeta, pkValue: PkValue, startIdx: number): string {
    const pkArray = this.pkToArray(pkValue);
    return pk.columns.map((col, i) => 
      `${this.quoteIdentifier(col)} = $${startIdx + i}`
    ).join(' AND ');
  }
  
  private pkToArray(pkValue: PkValue): CellValue[] {
    return Array.isArray(pkValue) ? pkValue : [pkValue];
  }
}
```

**MySQL/SQLite generators:**

```typescript
class MySqlGenerator implements SqlGenerator {
  // Same logic but:
  // - Use ? placeholders instead of $1, $2
  // - No RETURNING clause; re-query for inserted IDs
  // - Quote with backticks
  
  generateInsert(table: TableMeta, tempId: TempRowId, row: AddedRow): ParameterizedSql {
    const columns = Object.keys(row.data);
    const values = Object.values(row.data);
    const placeholders = values.map(() => '?').join(', ');
    
    return {
      sql: `INSERT INTO \`${table.tableName}\` (${columns.map(c => `\`${c}\``).join(', ')}) VALUES (${placeholders})`,
      params: values,
    };
  }
}
```

**Success criteria:**
- [ ] Postgres uses `$1, $2` placeholders and `RETURNING`
- [ ] MySQL uses `?` placeholders, no `RETURNING`
- [ ] SQLite uses `?` placeholders, no `RETURNING`
- [ ] All identifiers are quoted to prevent SQL injection
- [ ] Values sent as parameters, never interpolated
- [ ] Composite primary keys handled correctly
- [ ] SQL generation is deterministic (order: DELETE, UPDATE, INSERT)

**Key files:**
- `src/lib/services/sql-generator.ts`
- `src/lib/services/generators/postgres.ts`
- `src/lib/services/generators/mysql.ts`
- `src/lib/services/generators/sqlite.ts`

---

### Phase 6: Batch Execution Backend

**Goal:** Execute SQL batches atomically with transaction support.

**Rust command:**

```rust
// src-tauri/src/commands/edits.rs

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlBatch {
    pub statements: Vec<ParameterizedSql>,
    pub use_transaction: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterizedSql {
    pub sql: String,
    pub params: Vec<serde_json::Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult {
    pub success: bool,
    pub executed_count: usize,
    pub total_statements: usize,
    pub error: Option<BatchError>,
    pub inserted_ids: HashMap<String, serde_json::Value>, // temp_id -> pk_value
}

#[tauri::command]
pub async fn execute_batch(
    state: State<'_, AppState>,
    connection_id: String,
    batch: SqlBatch,
) -> Result<BatchResult, CommandError> {
    let pool = state.pools.get(&connection_id)
        .ok_or(CommandError::NotFound("Connection not found".into()))?;
    
    let mut tx = pool.begin().await?;
    let mut executed = 0;
    let mut inserted_ids = HashMap::new();
    
    for stmt in batch.statements {
        match execute_statement(&mut tx, &stmt, &mut inserted_ids).await {
            Ok(()) => executed += 1,
            Err(e) => {
                tx.rollback().await?;
                return Ok(BatchResult {
                    success: false,
                    executed_count: executed,
                    total_statements: batch.statements.len(),
                    error: Some(BatchError {
                        statement_index: executed,
                        message: e.to_string(),
                        code: extract_error_code(&e),
                    }),
                    inserted_ids: HashMap::new(),
                });
            }
        }
    }
    
    tx.commit().await?;
    
    Ok(BatchResult {
        success: true,
        executed_count: executed,
        total_statements: batch.statements.len(),
        error: None,
        inserted_ids,
    })
}

async fn execute_statement(
    tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    stmt: &ParameterizedSql,
    inserted_ids: &mut HashMap<String, serde_json::Value>,
) -> Result<(), sqlx::Error> {
    let query = sqlx::query(&stmt.sql);
    
    // Bind parameters...
    for param in &stmt.params {
        // Bind based on value type
    }
    
    // Execute and capture RETURNING for Postgres
    let result = query.execute(&mut **tx).await?;
    
    // For INSERT with RETURNING, capture returned PK
    // (implementation depends on sqlx AnyRow handling)
    
    Ok(())
}
```

**Success criteria:**
- [ ] Batch executes in a single transaction
- [ ] On error, entire batch is rolled back
- [ ] Postgres RETURNING values captured for INSERT
- [ ] MySQL/SQLite re-query for inserted IDs
- [ ] Error messages are user-friendly
- [ ] FK constraint violations detected and reported

**Key files:**
- `src-tauri/src/commands/edits.rs`
- `src-tauri/src/error.rs`

---

### Phase 7: Preview & Execute Modal

**Goal:** Show SQL preview and allow execute/cancel.

**Modal component:**

```svelte
<!-- src/lib/components/SqlPreviewModal.svelte -->
<script lang="ts">
  import { CodeMirror } from 'codemirror';
  import { sql } from '@codemirror/lang-sql';
  
  let { batch, onExecute, onCancel, open } = $props();
  
  let sqlText = $derived(
    batch.statements.map((s, i) => 
      `-- Statement ${i + 1}\n${s.sql}\n-- Params: ${JSON.stringify(s.params)}`
    ).join('\n\n')
  );
</script>

{#if open}
  <div class="modal-backdrop" onclick={onCancel}>
    <div class="modal" onclick={(e) => e.stopPropagation()}>
      <header>
        <h2>Preview SQL Changes</h2>
        <span class="change-count">{batch.statements.length} statements</span>
      </header>
      
      <div class="sql-preview">
        <CodeMirror
          value={sqlText}
          extensions={[sql()]}
          editable={false}
        />
      </div>
      
      <footer>
        <button class="secondary" onclick={onCancel}>Cancel</button>
        <button class="primary" onclick={onExecute}>
          Execute ({batch.statements.length} statements)
        </button>
      </footer>
    </div>
  </div>
{/if}
```

**Keyboard shortcut handling:**

```typescript
// In App.svelte or SqlEditor.svelte

window.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault();
    
    if (editStore.hasChanges && editStore.tableMeta?.isEditable) {
      const batch = sqlGenerator.generateBatch(editStore.changeSet, editStore.tableMeta);
      openPreviewModal(batch);
    }
  }
});
```

**Execution flow:**

```typescript
async function executeBatch(batch: SqlBatch) {
  try {
    setExecuting(true);
    const result = await invoke<BatchResult>('execute_batch', {
      connectionId: connectionStore.activeId,
      batch,
    });
    
    if (result.success) {
      // Clear change tracking
      editStore.discardAllChanges();
      
      // Update inserted IDs in grid
      if (result.insertedIds) {
        gridStore.updateInsertedIds(result.insertedIds);
      }
      
      // Re-run original query to refresh
      await executeQuery(queryStore.lastQuery);
      
      showSuccess(`Executed ${result.executedCount} statements successfully`);
    } else {
      showError(`Execution failed: ${result.error?.message}`);
    }
  } finally {
    setExecuting(false);
    closePreviewModal();
  }
}
```

**Success criteria:**
- [ ] Ctrl/Cmd+S opens preview modal
- [ ] Modal shows syntax-highlighted SQL
- [ ] Each statement shows with its parameters
- [ ] "Cancel" closes modal, keeps changes
- [ ] "Execute" runs batch and closes modal
- [ ] Success clears changes and refreshes grid
- [ ] Error shows message, keeps changes in state

**Key files:**
- `src/lib/components/SqlPreviewModal.svelte`
- `src/lib/stores/modal.svelte.ts`

---

### Phase 8: Error Handling & Edge Cases

**Goal:** Handle all error scenarios gracefully.

**Error scenarios:**

| Scenario | Detection | User Message | Recovery |
|----------|-----------|--------------|----------|
| No primary key detected | Schema fetch returns `primaryKey.exists: false` | "Cannot edit: no primary key detected for this table" | Disable edit/delete, allow view only |
| Multi-table join | Schema fetch returns `isEditable: false` | "Cannot edit: query joins multiple tables" | Disable all editing |
| Concurrent modification | UPDATE affects 0 rows | "Row was modified or deleted by another user. Refresh to see current data." | Keep changes in state, suggest refresh |
| FK constraint violation | Error code 23503 (Postgres), 1451 (MySQL) | "Cannot delete: row is referenced by other records" | Keep changes, show constraint name |
| Unique constraint violation | Error code 23505 (Postgres), 1062 (MySQL) | "Value already exists: duplicate key" | Keep changes, highlight offending cell |
| Check constraint violation | Error code 23514 (Postgres) | "Value violates constraint: {constraint_name}" | Keep changes |
| Type mismatch | Parameter binding fails | "Invalid value for column '{column}': expected {type}" | Validate before sending |
| NULL in non-null column | DB error | "Column '{column}' cannot be null" | Validate before sending |

**Validation before execute:**

```typescript
function validateChangeSet(changeSet: ChangeSet, tableMeta: TableMeta): ValidationError[] {
  const errors: ValidationError[] = [];
  
  // Check for NULL in non-null columns
  for (const [tempId, addedRow] of changeSet.added) {
    for (const col of tableMeta.columns) {
      if (!col.nullable && addedRow.data[col.name] === null && !col.isAutoIncrement) {
        errors.push({ rowId: tempId, column: col.name, message: 'Cannot be null' });
      }
    }
  }
  
  // Check for missing required columns in INSERT
  for (const [tempId, addedRow] of changeSet.added) {
    for (const col of tableMeta.columns) {
      if (!col.nullable && !col.isAutoIncrement && !col.isGenerated && !(col.name in addedRow.data)) {
        errors.push({ rowId: tempId, column: col.name, message: 'Required column not provided' });
      }
    }
  }
  
  return errors;
}
```

**Success criteria:**
- [ ] No PK → edit disabled with tooltip
- [ ] Multi-table join → edit disabled with reason
- [ ] Concurrent modification detected and reported
- [ ] FK/unique/check violations show user-friendly messages
- [ ] NULL validation before execute
- [ ] Type validation before execute

**Key files:**
- `src/lib/services/validator.ts`
- `src/lib/components/ErrorBanner.svelte`
- `src/lib/components/ValidationErrors.svelte`

---

## System-Wide Impact

### Interaction Graph

1. User runs SELECT → `execute_query` → results stream via Channel → **NEW: `fetch_schema_metadata` invoked** → TableMeta stored → grid becomes editable (if PK detected)

2. User double-clicks cell → inline editor opens → user types → Enter → **change tracked in EditStore** → cell shows yellow background

3. User clicks "Add Row" → **temp row added to EditStore** → green border shown → user fills cells → each edit tracked

4. User selects row → presses Delete → **PK added to deleted set** → row shows red strikethrough → row filtered from view

5. User presses Ctrl+S → **SQL generated** → preview modal opens → user clicks Execute → `execute_batch` → transaction starts → statements execute → on success, changes cleared, grid refreshed

### Error Propagation

| Error Type | Origin | Handling |
|-----------|--------|---------|
| Schema fetch failed | `fetch_schema_metadata` | Grid remains read-only, no error shown (graceful degradation) |
| PK not detected | Schema detector | Tooltip on hover: "Cannot edit: no primary key" |
| Validation error | Frontend validator | Inline error on cell, prevent preview modal |
| Batch execution failed | `execute_batch` | Modal shows error, changes remain in state |
| Partial batch failure | Transaction rollback | All changes rolled back, error shown |

### State Lifecycle Risks

- **Orphaned temp IDs:** If batch succeeds but refresh fails, temp IDs remain. Solution: Temp IDs replaced with real PKs after successful INSERT.
- **Stale schema cache:** Schema changes externally. Solution: 5-minute TTL, force refresh on "schema changed" error.
- **Concurrent edits:** User A edits, user B deletes same row. Solution: Detect UPDATE affecting 0 rows, prompt refresh.

### Integration Test Scenarios

1. **Edit + save single cell:** SELECT → edit cell → Ctrl+S → preview shows UPDATE → execute → grid refreshes with new value
2. **Add row with auto-increment:** Add row → fill non-PK columns → Ctrl+S → preview shows INSERT → execute → new row appears with auto-generated ID
3. **Delete multiple rows:** Delete 3 rows → Ctrl+S → preview shows 3 DELETEs → execute → rows removed
4. **Batch with error:** Add row (violates unique constraint) + delete row → Ctrl+S → execute → transaction rolls back → both changes remain in state
5. **No PK detection:** SELECT from view → schema fetch returns no PK → edit disabled → tooltip explains why

---

## Acceptance Criteria

### Functional Requirements

- [ ] **AC-EDIT-01** After successful SELECT, schema metadata is fetched including table name, primary keys, and column types
- [ ] **AC-EDIT-02** When primary key cannot be detected, edit and delete operations are disabled with explanatory tooltip
- [ ] **AC-EDIT-03** Multi-table join results are read-only with reason displayed
- [ ] **AC-EDIT-04** Double-click or Enter starts inline cell editing
- [ ] **AC-EDIT-05** Escape cancels edit and reverts to original value
- [ ] **AC-EDIT-06** Enter or blur commits edit to change tracking
- [ ] **AC-EDIT-07** Type-aware editors: boolean dropdown, date picker, enum dropdown
- [ ] **AC-EDIT-08** Read-only columns (auto-increment, generated) are not editable
- [ ] **AC-EDIT-09** NULL vs empty string distinction supported (context menu option)
- [ ] **AC-EDIT-10** Modified cells show yellow background
- [ ] **AC-EDIT-11** Added rows show green left border
- [ ] **AC-EDIT-12** Deleted rows show red background with strikethrough
- [ ] **AC-EDIT-13** Add row via: empty row at bottom, toolbar button, context menu, Ctrl/Cmd+N
- [ ] **AC-EDIT-14** Delete row via: context menu, Delete key, Backspace key
- [ ] **AC-EDIT-15** "Discard All Changes" button clears all pending modifications
- [ ] **AC-EDIT-16** Ctrl/Cmd+S opens SQL preview modal
- [ ] **AC-EDIT-17** Preview modal shows syntax-highlighted SQL with parameters
- [ ] **AC-EDIT-18** All values sent as parameters (no SQL injection)
- [ ] **AC-EDIT-19** Postgres uses RETURNING clause for INSERT; MySQL/SQLite re-query
- [ ] **AC-EDIT-20** Batch executes in single transaction
- [ ] **AC-EDIT-21** On error, entire batch is rolled back
- [ ] **AC-EDIT-22** Concurrent modification detected (UPDATE affects 0 rows)
- [ ] **AC-EDIT-23** FK/unique/check violations show user-friendly messages
- [ ] **AC-EDIT-24** NULL validation for non-null columns before execute
- [ ] **AC-EDIT-25** Success clears changes and refreshes grid with new data

### Non-Functional Requirements

- [ ] Schema fetch completes in under 500ms for typical tables
- [ ] Cell edit response is immediate (no perceptible lag)
- [ ] SQL preview generation for 50 statements in under 100ms
- [ ] Batch execution UI remains responsive (non-blocking)

### Quality Gates

- [ ] No SQL injection vectors (parameterized queries only)
- [ ] All identifiers properly quoted
- [ ] Transaction rollback tested for all error paths
- [ ] Keyboard-only workflow supported for all operations

---

## Dependencies & Prerequisites

### Requires MVP Phase 4 Complete

- `execute_query` command with Channel streaming
- `QueryEvent` types
- `ResultGrid.svelte` with TanStack Virtual
- `query.svelte.ts` store

### New Rust Dependencies

```toml
# src-tauri/Cargo.toml
[dependencies]
# ... existing deps ...
sqlparser = "0.45"  # For parsing SELECT queries to find table references
```

### New Frontend Dependencies

```json
// package.json (no new deps required)
// CodeMirror already installed for SQL editor
```

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Schema metadata query slow on large schemas | Medium | Medium | Cache per-connection with 5-min TTL; show loading indicator |
| sqlparser can't handle complex queries | Medium | Low | Fallback to "unknown table" → disable editing |
| User expects undo/redo | High | Low | Document that changes are one-shot; "Discard All" is the escape hatch |
| Batch of 1000 INSERTs is slow | Medium | Medium | Show progress indicator; consider batching limit (warn at 100) |
| RETURNING fails for generated columns | Low | Medium | Detect generated columns, exclude from INSERT statement |

---

## Future Considerations (v3+)

- **Multi-row selection** — Select multiple rows for batch delete or bulk update
- **Undo/redo stack** — Per-cell and global undo
- **Column reordering** — Drag columns to reorder
- **Copy/paste rows** — Excel-like copy/paste
- **Import CSV** — Bulk INSERT from CSV file
- **Edit history** — Audit log of all changes with timestamps

---

## File Structure

```
sqlator/
├── core/
│   └── src/
│       ├── schema_cache.rs      # Moved to core
├── tauri-app/
│   ├── src-tauri/
│   │   └── src/
│       ├── commands/
│       │   ├── mod.rs
│       │   ├── connections.rs
│       │   ├── queries.rs
│       │   ├── schema.rs        # NEW: fetch_schema_metadata
│       │   └── edits.rs         # NEW: execute_batch
│       ├── schema_cache.rs      # NEW: TTL cache for schema metadata
│       └── error.rs             # Extended with BatchError
├── src/
│   └── lib/
│       ├── types.ts             # Extended: TableMeta, ChangeSet, SqlBatch
│       ├── stores/
│       │   ├── connections.svelte.ts
│       │   ├── query.svelte.ts
│       │   ├── schema.svelte.ts # NEW: TableMeta state
│       │   ├── edit.svelte.ts   # NEW: ChangeSet tracking
│       │   └── modal.svelte.ts  # NEW: Modal state
│       ├── services/
│       │   ├── schema-fetcher.ts      # NEW
│       │   ├── sql-generator.ts       # NEW
│       │   ├── generators/
│       │   │   ├── postgres.ts        # NEW
│       │   │   ├── mysql.ts           # NEW
│       │   │   └── sqlite.ts          # NEW
│       │   └── validator.ts           # NEW
│       └── components/
│           ├── ResultGrid.svelte      # Enhanced: inline editing
│           ├── GridToolbar.svelte     # NEW: Add Row, Discard buttons
│           ├── SqlPreviewModal.svelte # NEW
│           ├── ContextMenu.svelte     # NEW
│           ├── editors/
│           │   ├── TextEditor.svelte  # NEW
│           │   ├── BooleanEditor.svelte # NEW
│           │   ├── DateEditor.svelte  # NEW
│           │   ├── DateTimeEditor.svelte # NEW
│           │   ├── EnumEditor.svelte  # NEW
│           │   ├── NumberEditor.svelte # NEW
│           │   ├── JsonEditor.svelte  # NEW
│           │   └── TextAreaEditor.svelte # NEW
│           └── ValidationErrors.svelte # NEW
```

---

## Sources & References

### Origin

- **Origin document:** [docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md](docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md)
- Key decisions carried forward: TanStack Virtual grid, sqlx AnyPool, QueryEvent streaming, Channel pattern for large results

### Internal References

- Result grid design: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:295-375`
- Query execution flow: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:118-138`
- IPC command surface: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:107-117`

### External References

- [Tauri 2 `tauri::ipc::Channel`](https://v2.tauri.app/develop/calling-rust/#channels) — Streaming pattern
- [sqlx AnyPool Transactions](https://docs.rs/sqlx/latest/sqlx/struct.Pool.html#method.begin) — Transaction support
- [sqlparser crate](https://docs.rs/sqlparser/latest/sqlparser/) — SQL parsing for table extraction
- [Postgres information_schema](https://www.postgresql.org/docs/current/information-schema.html) — Schema metadata queries
- [TanStack Virtual](https://tanstack.com/virtual/latest) — `createVirtualizer` for Svelte 5

### Key Gotchas

1. **Parameterized queries only** — Never interpolate values into SQL strings
2. **Quote all identifiers** — Use `"` for Postgres/SQLite, `` ` `` for MySQL
3. **RETURNING is Postgres-only** — MySQL/SQLite must re-query for inserted IDs
4. **Composite PKs** — WHERE clause must include all PK columns
5. **Generated columns** — Must be excluded from INSERT/UPDATE
6. **Auto-increment** — Must be excluded from INSERT
7. **Schema cache TTL** — 5 minutes to handle external schema changes
8. **Transaction for batch** — All-or-nothing execution model
