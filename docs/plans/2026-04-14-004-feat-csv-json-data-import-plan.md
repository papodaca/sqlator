---
title: "feat: CSV/JSON Data Import"
type: feat
status: active
date: 2026-04-14
---

# feat: CSV/JSON Data Import

## Overview

Add a drag-and-drop CSV/JSON import flow to the schema browser. Users drag a file onto any table node, map source columns to target columns, preview the generated INSERT batch, and execute — all without leaving the app. The feature is purely frontend: it reuses the existing `execute_batch` Tauri command, `sql-generator.ts` INSERT logic, and `SqlPreviewModal` for preview, so no new Rust code is required.

## Problem Statement / Motivation

Users frequently need to seed tables or migrate data from spreadsheet exports and JSON fixtures. Currently they must write INSERT statements by hand or use an external tool. Given that the editable grid already supports row-level INSERT/UPDATE/DELETE and the export feature is planned, data import completes the round-trip and makes sqlator viable for lightweight ETL workflows.

## Proposed Solution

### User Flow

1. User drags a `.csv` or `.json` file from the OS onto a table node in the schema browser.
2. A `DataImportModal` opens showing file metadata and a parsed preview (first 10 rows).
3. User maps each source column to a target table column. Auto-matched by name. Unmapped columns can be excluded.
4. The modal generates a `SqlBatch` using existing `sql-generator.ts` INSERT logic and shows the `SqlPreviewModal`.
5. User executes — `execute_batch` runs all statements in a transaction.
6. Modal shows result: N rows inserted (or error details).

### Scope Decisions (v1)

| Concern | v1 Decision |
|---|---|
| JSON shape | Array-of-objects only (`[{...}, ...]`) |
| CSV header | Row 1 is always the header |
| Delimiter detection | Auto-detect comma / tab / semicolon; show detected delimiter with manual override |
| Type coercion | Frontend coerces string values to `CellValue` using `ColumnMeta.dataType` |
| Row limit | Warn (not block) above 10,000 rows; chunk execution into batches of 500 rows per `execute_batch` call |
| Transaction | All-or-nothing per chunk (matching existing `execute_batch` behavior) |
| Partial commit | Not in v1 |
| Encoding | UTF-8 assumed; show decode error if `FileReader` emits replacement characters |

## Technical Approach

### 1. Drop Target & Context Menu — `SchemaNode.svelte`

**Drag-and-drop:** Add file-drop handlers to the `.table-row` button (line 56). Guard: only accept when `table.tableType === 'table'` and `e.dataTransfer.types.includes('Files')` (so existing connection-drag which uses `text/plain` is unaffected). On successful drop, emit an `onimport` event with `{ table: TableInfo, file: File }`.

```svelte
// src/lib/components/SchemaNode.svelte (inside the table button)
let fileDropOver = $state(false);

function handleDragOver(e: DragEvent) {
  if (!e.dataTransfer?.types.includes('Files')) return;
  if (table.tableType !== 'table') return;
  e.preventDefault();
  e.dataTransfer.dropEffect = 'copy';
  fileDropOver = true;
}

function handleDrop(e: DragEvent) {
  fileDropOver = false;
  const file = e.dataTransfer?.files[0];
  if (!file || table.tableType !== 'table') return;
  e.preventDefault();
  onimport?.({ table, file });
}
```

Visual feedback: add `.file-drop-over` class (dashed accent border) when `fileDropOver` is true.

**Context menu fallback:** Also add "Import CSV / JSON…" to the `contextItems` array (line 25–28) so keyboard and right-click users can trigger import without drag-and-drop. Opens a standard `<input type="file" accept=".csv,.json">` hidden picker and fires the same `onimport` event.

```svelte
// src/lib/components/SchemaNode.svelte — contextItems
{ label: 'Import CSV / JSON…', action: () => fileInputEl.click(), disabled: table.tableType !== 'table' }
```

### 2. `DataImportModal.svelte`

New modal component in `src/lib/components/`. Follows the `ImportDialog.svelte` stepper pattern with steps:

```typescript
type ImportStep = 'parse' | 'map' | 'preview' | 'done';
```

**Step: `parse`**
- Show file name, detected format, row count, first 10 rows as a read-only preview table.
- Use `FileReader.readAsText` (same as `ImportDialog.svelte` line 34).
- CSV parser: inline implementation using auto-delimiter detection (no new dependency needed).
- JSON parser: `JSON.parse` with try/catch, validate result is an array.
- Expose a delimiter picker (auto / comma / tab / semicolon) for CSV.

**Step: `map`**
- Use `columns: SchemaColumnInfo[]` already passed through the `onimport` event (available as a prop on `SchemaNode` — no additional Tauri call needed for column names/types). Fall back to `api.invoke<TableMeta>('get_table_meta', ...)` only if `columns` is null (schema not yet loaded).
- Render a two-column mapper: source column (from file) → target column (dropdown of `columns`).
- Auto-match by normalized name (`toLowerCase().replace(/\s/g, '_')`).
- Highlight unmapped non-nullable columns without defaults as warnings (⚠️) — block "Next" until resolved.
- Skip auto-increment / generated columns automatically.
- Show target column type beside each dropdown for guidance.

**Step: `preview`**
- Build `AddedRow[]` from mapped + coerced data.
- Call `generateBatch(changeSet, tableMeta, dbType)` from existing `sql-generator.ts`.
- Pass resulting `SqlBatch` to `SqlPreviewModal` (or an inline CodeMirror view for the large-file case).
- If row count > 10,000: show a warning and a "50 rows shown" truncation notice. Full batch executes unchanged.

**Step: `done`**
- Show inserted row count per chunk.
- "Close" and optional "Open Table" button to navigate to the table in the grid.

### 3. Type Coercion Utility — `src/lib/services/import-coerce.ts`

New pure utility file (no side effects, easily testable).

```typescript
// src/lib/services/import-coerce.ts
export function coerceValue(raw: string, dataType: string): CellValue;
```

Rules by `ColumnMeta.dataType` category:
- **integer / bigint / numeric / float**: `Number(raw)` — null if `raw === ''`
- **boolean / bit**: `'true'|'1'|'yes'` → `true`; `'false'|'0'|'no'` → `false`; null if empty
- **date / timestamp**: leave as string (DB parses ISO 8601 fine across all dialects)
- **json / jsonb**: `JSON.parse(raw)` with fallback to raw string
- **default (text, varchar, etc.)**: raw string; `''` → `null` only if column `nullable`

### 4. Chunked Execution

For imports exceeding 500 rows, split the row array into 500-row chunks, generate a `SqlBatch` per chunk, and call `execute_batch` sequentially. Show a progress bar during execution.

### 5. Schema-Qualified Table Names

Use `table.fullName` (already set on `TableInfo`) when building the INSERT SQL. The existing `sql-generator.ts` `pgGenerateInsert` and `myGenerateInsert` accept `tableMeta.tableName` — pass `table.fullName` when constructing the synthetic `TableMeta` for the import path.

## Files to Create / Modify

| File | Change |
|---|---|
| `src/lib/components/SchemaNode.svelte` | Add `ondragover`, `ondragleave`, `ondrop`, `onimport` prop, `file-drop-over` visual state; add "Import CSV / JSON…" to `contextItems` (line 25–28) |
| `src/lib/components/SchemaBrowser.svelte` | Thread `onimport` event (with `columns`) up to parent |
| `src/lib/components/DataImportModal.svelte` | **New** — full stepper modal |
| `src/lib/components/ColumnMapper.svelte` | **New** — reusable column mapping table |
| `src/lib/services/import-coerce.ts` | **New** — type coercion utility |
| `src/lib/services/csv-parser.ts` | **New** — minimal CSV parser (delimiter auto-detect, quoted fields, CRLF) |
| `src/routes/+page.svelte` or `TabbedEditor.svelte` | Mount `DataImportModal` and handle `onimport` event |

No Rust changes required. No new npm packages required.

## System-Wide Impact

- **Interaction graph:** Drop on `SchemaNode` → `onimport` event bubbles to page → `DataImportModal` opens → on execute, calls `api.invoke('execute_batch')` → same Rust path as editable grid saves → `execute_batch` at `commands.rs:1721`.
- **Existing drag-and-drop:** The `e.dataTransfer.types.includes('Files')` guard in `SchemaNode` ensures no interference with the connection re-grouping drag (which uses `text/plain`, not `Files`).
- **State isolation:** Import is self-contained in the modal; it does not touch `editStore` or `queryStore`. After success it can optionally refresh `schemaStore` (if row count changes matter) — not required.
- **Error propagation:** Chunk failures surface as an error message in the `done` step. No partial rollback of already-committed chunks — this is consistent with the v1 all-or-nothing decision per chunk.
- **Integration test scenarios:**
  1. CSV with header names matching table columns — all rows inserted correctly.
  2. CSV with extra columns — mapper shows them as "unmapped", can be excluded.
  3. Non-nullable column unmapped — mapper blocks "Next" with a warning.
  4. JSON array with 600 rows — chunked into 2 batches of 500/100, both committed.
  5. Drag onto a view node — drop rejected with a tooltip "Cannot import into a view".

## Acceptance Criteria

- [ ] Dragging a `.csv` or `.json` file onto a table node in the schema browser opens `DataImportModal`
- [ ] Dragging onto a view node, column row, or schema header is rejected (no modal opens)
- [ ] File drag does not interfere with existing connection re-grouping drag
- [ ] CSV parser handles comma, tab, and semicolon delimiters with auto-detection
- [ ] CSV parser handles quoted fields (RFC 4180), CRLF line endings, and empty fields
- [ ] JSON parser accepts `[{...}]` array-of-objects; shows a clear error for other shapes
- [ ] Column mapper auto-matches by normalized name
- [ ] Unmapped non-nullable columns without defaults block advancing to preview
- [ ] Auto-increment / generated columns are excluded from the mapper
- [ ] Type coercion converts CSV strings to appropriate `CellValue` types per `ColumnMeta.dataType`
- [ ] INSERT batch uses `table.fullName` (schema-qualified) for all databases
- [ ] Imports > 10,000 rows show a warning; user can still proceed
- [ ] Execution is chunked at 500 rows per `execute_batch` call with a visible progress bar
- [ ] Success step shows total rows inserted
- [ ] Failure shows which chunk failed and the DB error message
- [ ] Works across Postgres, MySQL, SQLite (tested); MSSQL/Oracle/ClickHouse use existing dialect handling in `sql-generator.ts`
- [ ] Keyboard: Escape closes modal at any step; Enter advances from parse → map → preview

## Dependencies & Risks

| Risk | Mitigation |
|---|---|
| CSV edge cases (nested quotes, multiline fields) | Implement RFC 4180-compliant parser; add unit tests for edge cases |
| Large file UI freeze | Parse in a microtask queue or Web Worker if >50k rows detected |
| ClickHouse / Oracle type strictness | Rely on `ColumnMeta.dataType` coercion; surface DB error clearly |
| `get_table_meta` not available on all connection types | Check existing editable grid — if it works there, import works the same way |
| Schema-qualified INSERT on MySQL (no schema prefix) | `table.fullName` for MySQL is just `table.name`; `TableInfo` already handles this |

## Sources & References

### Internal References

- Stepper modal pattern: `src/lib/components/ImportDialog.svelte:10–55`
- SQL INSERT generation: `src/lib/services/sql-generator.ts:39–107`
- Batch execution: `src/lib/stores/edit.svelte.ts:200–215`
- Batch Tauri command: `src-tauri/src/commands.rs:1721`
- Drag-and-drop pattern: `src/lib/components/GroupItem.svelte:40–59`
- Schema node template: `src/lib/components/SchemaNode.svelte:55–103`
- Type definitions: `src/lib/types.ts:147–170`
- SQL preview modal: `src/lib/components/SqlPreviewModal.svelte`
- TableMeta / ColumnMeta models: `core/src/models.rs:209–239`
