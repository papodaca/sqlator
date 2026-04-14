---
title: Schema-Aware SQL Autocomplete
type: feat
status: active
date: 2026-04-14
---

# feat: Schema-Aware SQL Autocomplete

## Overview

Wire the already-fetched schema tree (tables + columns) into CodeMirror 6's built-in `sql()` extension `schema` option so the editor completes table names, column names, and SQL keywords as the user types. The `@codemirror/lang-sql` package is already installed and the schema store already holds the data — the gap is simply connecting the two.

## Problem Statement / Motivation

Users must type table and column names from memory. The schema sidebar shows all available objects, but the editor has no awareness of them. Wiring the two together eliminates a major UX friction point with minimal new infrastructure.

## Proposed Solution

1. Build a `schemaToCompletions` helper that maps `ConnectionSchemaState` → `Record<string, string[]>` (table name → column names), which is the exact format `sql({ schema })` expects.
2. Hold the `sql()` extension in a `Compartment` so it can be reconfigured in-place without destroying and recreating the `EditorView`.
3. Add a `$effect` in `SqlEditor.svelte` that reacts to changes in `schemaStore.getState(connectionId)` and dispatches a compartment reconfiguration.
4. Eagerly bulk-load all columns for the active schema's tables when the editor first opens (via `schemaStore.loadColumns`), so completions don't depend on the user having expanded tree nodes.

## Technical Considerations

### Architecture impacts

- Change is confined to `SqlEditor.svelte` plus a small helper. No new stores, no new IPC commands, no new packages.
- `schemaStore` is already a plain importable module — no prop threading needed.
- The editor is currently recreated on every `${connectionId}:${queryTabId}:${theme.isDark}` change. With a `Compartment`, the sql extension can also be reconfigured independently when schema data updates mid-session.

### The `sql()` schema option

`@codemirror/lang-sql` accepts:

```typescript
// SqlEditor.svelte — inside createEditor()
sql({
  dialect: dialectMap[dbType] ?? PostgreSQL,
  schema: {
    "users": ["id", "name", "email"],
    "public.users": ["id", "name", "email"],  // schema-qualified name
  },
  defaultSchema: "public",  // optional
})
```

Both `table.name` (short) and `table.fullName` (schema-qualified, e.g. `"public.users"`) should be included as keys so completions fire for both typing styles.

### `Compartment` reconfiguration

```typescript
// SqlEditor.svelte
import { Compartment } from "@codemirror/state";

const sqlCompartment = new Compartment();

// In createEditor():
extensions: [
  basicSetup,
  sqlCompartment.of(sql({ dialect, schema: buildSchemaMap(connectionId) })),
  // ...other extensions
]

// In $effect watching schema:
view.dispatch({
  effects: sqlCompartment.reconfigure(
    sql({ dialect, schema: buildSchemaMap(connectionId) })
  )
});
```

### Column loading strategy

`schemaStore.getState(connectionId).columns` is a `Record<string, SchemaColumnInfo[]>` keyed by table name — only populated for tables the user has expanded in the tree. For autocomplete this is insufficient for first-time users.

**Strategy:** when `state.tables` is populated and `state.columns` is empty (or sparse), call `schemaStore.loadColumns(connectionId, tableName)` for each table in a batched `Promise.all`. Cap concurrency at 10 parallel fetches to avoid flooding the IPC bridge.

```typescript
// Eagerly load columns for all tables in the active schema
async function prefetchColumns(connectionId: string) {
  const state = schemaStore.getState(connectionId);
  const unloaded = state.tables.filter(t => !state.columns[t.name]);
  // Batch in chunks of 10
  for (let i = 0; i < unloaded.length; i += 10) {
    await Promise.all(
      unloaded.slice(i, i + 10).map(t =>
        schemaStore.loadColumns(connectionId, t.name, t.schema ?? undefined)
      )
    );
  }
}
```

Call `prefetchColumns` once when the editor mounts and when `connectionId` changes (not on every schema reactivity tick).

### Performance

- `buildSchemaMap` runs synchronously on already-loaded state — O(tables × columns), negligible.
- Compartment reconfiguration is incremental; CM6 does not re-parse the document.
- Autocomplete popup is triggered on demand by CM6 — no per-keystroke work from our side beyond what `basicSetup` already does.

### Dialect fallthrough

`"mssql"`, `"oracle"`, `"clickhouse"` all fall back to `PostgreSQL`. Consider adding `MSSQL` to the dialect map while touching this file (exported by `@codemirror/lang-sql`). Oracle/ClickHouse stay on the PostgreSQL fallback.

### Security considerations

Schema and column names come from the user's own database connection — no sanitization required for completions. Names are never evaluated, only displayed.

## System-Wide Impact

- **Interaction graph:** `schemaStore.loadColumns` → Tauri `get_columns` IPC → Rust backend. No new code paths; re-uses the existing IPC command that `SchemaTree` already calls on expand.
- **Error propagation:** `loadColumns` sets `state.error` on failure. `prefetchColumns` should ignore per-table errors gracefully (catch and continue) so a single inaccessible table doesn't block completions for the rest.
- **State lifecycle risks:** `schemaStore` caches columns indefinitely per session. If a user runs `ALTER TABLE` during a session, the cached columns will be stale until they refresh the schema. This is acceptable for v1.
- **API surface parity:** The `SchemaTree` already loads columns on expand — `loadColumns` is a shared call. No duplication.
- **Integration test scenarios:** Editor mounted before schema loads, schema loads mid-session, user switches active schema, user switches connection tabs, theme toggle (editor recreated).

## Acceptance Criteria

- [ ] Typing a partial table name shows matching table names from the connected schema as completions
- [ ] After typing `tableName.` (or `schema.tableName.`), column names for that table are suggested
- [ ] Schema-qualified names (e.g. `public.users`) also complete correctly
- [ ] Completions update when the user switches the active schema in the sidebar
- [ ] Completions update when the user switches to a different connection tab
- [ ] SQL keywords (SELECT, WHERE, JOIN, etc.) continue to work as before — no regression
- [ ] Completions work for all four mapped dialects (postgres, mysql, mariadb, sqlite)
- [ ] No visible lag when the autocomplete popup opens on large schemas (≥50 tables)
- [ ] Editor does not crash or lose state during schema prefetch
- [ ] `prefetchColumns` errors per-table are swallowed gracefully; remaining tables still complete

## Success Metrics

- Autocomplete popup appears for table names within one keystroke of a known prefix
- Column completions available for all tables (not only tree-expanded ones) within ~2 seconds of editor mount on a typical schema

## Dependencies & Risks

| Item | Detail |
|------|--------|
| `@codemirror/lang-sql ^6.10.0` | Already installed; `schema` option available since v6.0 |
| `@codemirror/state ^6.6.0` | Already installed; `Compartment` used here |
| Lazy column loading | Bulk fetch on mount may add ~500ms–2s on first open for large schemas; non-blocking |
| Editor recreation on theme toggle | Compartment is defined per-editor instance — safe to recreate; just rebuild the schema map in `createEditor()` |
| Very large schemas (500+ tables) | `buildSchemaMap` stays fast; CM6 completion filtering is done internally by the package |

## Implementation Notes

### Files to change

| File | Change |
|------|--------|
| `src/lib/components/SqlEditor.svelte` | Main change: import `schemaStore`, add `Compartment`, `buildSchemaMap`, `prefetchColumns`, schema `$effect` |

### Helper: `buildSchemaMap`

```typescript
// SqlEditor.svelte — module scope or inline
function buildSchemaMap(connectionId: string): Record<string, string[]> {
  const state = schemaStore.getState(connectionId);
  const map: Record<string, string[]> = {};
  for (const table of state.tables) {
    const cols = (state.columns[table.name] ?? []).map(c => c.name);
    map[table.name] = cols;
    if (table.fullName && table.fullName !== table.name) {
      map[table.fullName] = cols;  // also register schema-qualified name
    }
  }
  return map;
}
```

### Reactive schema update effect

```typescript
$effect(() => {
  const state = schemaStore.getState(connectionId);
  // Track tables and column keys so the effect re-runs when either changes
  const _tables = state.tables.length;
  const _cols = Object.keys(state.columns).length;

  if (!view) return;
  view.dispatch({
    effects: sqlCompartment.reconfigure(
      sql({ dialect: dialectMap[dbType] ?? PostgreSQL, schema: buildSchemaMap(connectionId) })
    )
  });
});
```

### Prefetch on mount

Call `prefetchColumns(connectionId)` inside the existing `$effect` that creates the editor, after `view` is assigned. It is async fire-and-forget — do not await it, as column data arriving reactively will trigger the schema update effect above.

## Sources & References

### Internal References

- Editor component: `src/lib/components/SqlEditor.svelte` (lines 51–89 — extensions, lines 108–132 — lifecycle)
- Schema store: `src/lib/stores/schema.svelte.ts` (lines 6–13 state shape, line 47 `getState`, line 94 `loadColumns`)
- Schema types: `src/lib/types.ts` (lines 229–251 — `TableInfo`, `SchemaColumnInfo`)
- Dialect map: `src/lib/components/SqlEditor.svelte` (lines 23–28)
- Parent component: `src/lib/components/TabbedEditor.svelte` (lines 135–140 — `dbType` derivation)

### External References

- `@codemirror/lang-sql` schema option: https://codemirror.net/docs/ref/#lang-sql.SQLConfig.schema
- CM6 `Compartment` for runtime reconfiguration: https://codemirror.net/examples/config/
