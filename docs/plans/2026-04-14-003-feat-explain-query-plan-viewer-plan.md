---
title: "feat: EXPLAIN / Query Plan Viewer"
type: feat
status: active
date: 2026-04-14
---

# feat: EXPLAIN / Query Plan Viewer

## Overview

Add a query plan viewer that automatically constructs and runs `EXPLAIN ANALYZE` (dialect-specific) on the current query, then renders the result as an annotated, collapsible tree. Both the desktop (Tauri/SvelteKit) and TUI (Ratatui) surfaces are in scope. The goal is to help developers tune queries without leaving the tool or manually constructing EXPLAIN syntax.

---

## Problem Statement

Developers currently must:
1. Manually prepend `EXPLAIN`/`EXPLAIN ANALYZE`/`EXPLAIN FORMAT=JSON` to their query (syntax differs by DB)
2. Read the raw text/JSON/tabular output in the plain results grid — which has `Constraint::Min(15)` column widths unsuitable for long EXPLAIN lines
3. Context-switch to an external tool (pgAdmin, DBeaver, EXPLAIN.depesz.com) to visualise the plan tree

The tool already routes `EXPLAIN` through `execute_query` as a select-like statement (identified at `core/src/db/mod.rs:119`), so raw EXPLAIN output already works. The missing piece is dialect-aware SQL construction, structured parsing, and a dedicated render layer.

---

## Proposed Solution

A dedicated **EXPLAIN tab mode** on the desktop (mirroring the `SchemaDdlState`/`SchemaDdlViewer` pattern) and a **toggle view** on the TUI results pane (no new `AppMode` to avoid conflicts with in-flight plans).

### Dialect-specific EXPLAIN SQL

No new Tauri command or backend route is needed — `execute_query` already handles EXPLAIN. A thin frontend helper constructs the dialect-specific SQL before invoking `execute_query`:

| Database | SQL injected | Output shape |
|---|---|---|
| PostgreSQL | `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS) <query>` | Single row, single `QUERY PLAN` JSON column |
| MySQL 8+ / MariaDB 10.6+ | `EXPLAIN FORMAT=JSON <query>` | Single row, single `EXPLAIN` JSON column |
| SQLite | `EXPLAIN QUERY PLAN <query>` | Tabular: `id`, `parent`, `notused`, `detail` |
| MSSQL | ❌ Deferred — requires `SET SHOWPLAN_XML ON` multi-statement sequence | — |
| Oracle | ❌ Deferred — requires `EXPLAIN PLAN FOR` + `SELECT … DBMS_XPLAN.DISPLAY` | — |
| ClickHouse | 🔜 v1.1 — `EXPLAIN <query>` returns tabular text | — |

### Tree rendering

- **PostgreSQL / MySQL**: parse `Plans` array recursively from JSON → `PlanNode` tree
- **SQLite**: reconstruct tree from `id` / `parent` integer columns
- Both renderers produce a flat `PlanNode[]` list with `depth`, `nodeType`, `cost`, `actualTime`, `rows`, `loops`, `detail` fields consumed by the view layer

### Desktop: dedicated tab

Open a new `QueryTab` with `explainPlan: ExplainPlanState` set, rendered by a new `ExplainPlanViewer.svelte` component. Original query result tab is unaffected.

### TUI: toggle view

In the `FocusPane::Results` pane, add a `p` keybinding that toggles between `ResultView::Table` and `ResultView::Plan`. The plan view reuses the flat `VisibleItem` + `ListState` pattern from the schema tree (`app.rs:398–482`). No new `AppMode` is added.

---

## Technical Approach

### Architecture

```
SqlEditor.svelte
  └─ Mod-Shift-Enter / "Explain" button
       └─ tabs.executeExplain(connectionId, queryTabId, sql, dbType)
            └─ api.executeQuery(connectionId, explainSql)  ← existing command
                 └─ streaming QueryEvents → parseExplainResult(columns, rows, dbType)
                      └─ ExplainPlanState { nodes: PlanNode[], dbType, durationMs }
                           └─ ExplainPlanViewer.svelte
                                └─ collapsible PlanNode tree
```

### Implementation Phases

#### Phase 1: SQL Construction + Raw Result (Backend-side, no new Rust code)

All changes are in the frontend SQL construction layer.

- Add `buildExplainSql(sql: string, dbType: DbType): string` to `src/lib/services/explain-builder.ts`
  - PostgreSQL: `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS) ${sql}`
  - MySQL: `EXPLAIN FORMAT=JSON ${sql}`
  - SQLite: `EXPLAIN QUERY PLAN ${sql}`
  - Others: throw `UnsupportedExplainError`
- Add `parseExplainResult(columns: string[], rows: JsonValue[][], dbType: DbType): PlanNode[]` to `src/lib/services/explain-parser.ts`
  - PostgreSQL/MySQL: `JSON.parse(rows[0][0])` → walk recursive `Plans` nodes
  - SQLite: build adjacency list from `id`/`parent` columns, then DFS into `PlanNode[]`
- Add types to `src/lib/types.ts`:
  ```ts
  export interface PlanNode {
    id: number;
    depth: number;
    nodeType: string;
    relation?: string;
    cost?: number;
    actualTime?: number;
    rows?: number;
    loops?: number;
    detail: string;
    children: PlanNode[];
    expanded: boolean;
  }
  export interface ExplainPlanState {
    nodes: PlanNode[];       // top-level nodes only; children nested
    flatVisible: PlanNode[]; // flattened visible list for the renderer
    dbType: DbType;
    durationMs: number;
  }
  ```

#### Phase 2: Desktop Integration

- Extend `QueryTab` in `src/lib/types.ts`:
  ```ts
  explainPlan?: ExplainPlanState;
  ```
- Add `openExplainPlan` and `updateExplainPlanState` methods to `src/lib/stores/tabs.svelte.ts` (mirror `openSchemaDdl` at line 158 / `updateSchemaDdlState` at line 202)
- Add `executeExplain(connectionId, tabId, sql)` to the store — constructs dialect SQL, calls `api.executeQuery`, collects rows, calls `parseExplainResult`, calls `updateExplainPlanState`
- Add `Mod-Shift-Enter` keybinding to `src/lib/components/SqlEditor.svelte` (keymap array at line 55):
  ```ts
  { key: "Mod-Shift-Enter", run: () => { onExplain(); return true; } }
  ```
- Add "Explain" button to `src/lib/components/EditorToolbar.svelte`
- Add `{:else if activeQueryTab.explainPlan}` branch to `src/lib/components/TabbedEditor.svelte` (dispatch chain at line 96)
- Create `src/lib/components/ExplainPlanViewer.svelte`:
  - Receives `ExplainPlanState` as prop
  - Renders flat `flatVisible` list with indentation via `padding-left: {node.depth * 16}px`
  - Click/Enter on a node toggles children (rebuilds `flatVisible` from tree)
  - Node card shows: `nodeType`, `relation`, `cost`, `actualTime`, `rows`, `loops`
  - Color-code nodes by cost percentile (e.g. red > 50% of total cost, yellow > 20%)
  - **Svelte 5 gotcha**: derive `flatVisible` via `$derived` from `nodes` but never write back to `nodes` from within the derived — use plain `let flatVisible` + `$effect` if mutation is needed (see `SchemaDdlViewer` fix in commit `2d27e51`)

#### Phase 3: TUI Integration

- Add `ResultView` enum to `tui-app/src/app.rs`:
  ```rust
  pub enum ResultView { Table, Plan }
  ```
  Add `result_view: ResultView` field to `App` (default `Table`).
- Add `explain_nodes: Vec<ExplainNode>` + `explain_list_state: ListState` fields to `App`
- Add `struct ExplainNode { depth: usize, label: String, detail: String, node_id: i64, expanded: bool }`
- Add `fn build_explain_sql(sql: &str, db_type: &DatabaseType) -> Result<String>` in `tui-app/src/app.rs`
- Add `fn parse_explain_result(columns: &[String], rows: &[Vec<Value>], db_type: &DatabaseType) -> Vec<ExplainNode>` — follow the same adjacency-list DFS as the desktop parser
- Add `fn rebuild_explain_visible(&mut self)` — mirror `rebuild_visible_items` (line 398) to flatten the tree respecting `expanded` flags
- Keybinding `p` in `FocusPane::Results` → toggle `result_view`; if switching to `Plan` and `explain_nodes` is empty, execute EXPLAIN automatically
- Render via `tui-app/src/ui.rs`: when `result_view == Plan`, render `List::new(explain_items)` with `ListState` instead of `Table` widget (use `Constraint::Min(15)` is wrong here — use full-width `List` items with `[{depth spaces}{node_type}] {detail}` formatting)
- `Enter` on a plan node toggles its children (`expand/collapse`) and calls `rebuild_explain_visible`

---

## Alternative Approaches Considered

### Use a new `explain_query` Tauri command

**Rejected**: `execute_query` already routes `EXPLAIN` correctly (it's classified as `is_select = true` at `core/src/db/mod.rs:119`). Adding a new command and DB driver path duplicates ~200 lines of code for no benefit. The only advantage would be pre-parsing on the Rust side, but JSON/tabular parsing is simpler in TypeScript and doesn't require a Rust recompile when format details change.

### Inline EXPLAIN result in `ResultPane` (add `kind: "explainPlan"` to `ResultPaneState`)

**Rejected**: The dedicated-tab approach (mirroring `SchemaDdlState`) keeps the original query result intact while viewing the plan — a developer can compare actual rows and the plan side-by-side in two tabs. Replacing the result with the plan would destroy that workflow.

### Show MSSQL / Oracle in v1

**Rejected**: Both require multi-statement execution not supported by the current single-statement executor. MSSQL needs `SET SHOWPLAN_XML ON` / query / `SET SHOWPLAN_XML OFF`. Oracle needs `EXPLAIN PLAN FOR` + `SELECT … FROM TABLE(DBMS_XPLAN.DISPLAY)`. Forcing this through `execute_batch` (which exists) would require parsing XML/text output of a different shape — high complexity for a v1.

---

## System-Wide Impact

### Interaction Graph

`Mod-Shift-Enter` → `SqlEditor.svelte:onExplain` → `tabs.executeExplain` (store) → `api.executeQuery` (existing Tauri IPC) → `DbManager::execute_query` (Rust, routes via `is_select` heuristic) → streams `QueryEvent::Columns` + `QueryEvent::Row` + `QueryEvent::Done` → `tabs.executeExplain` accumulates rows → `parseExplainResult` → `updateExplainPlanState` → `TabbedEditor.svelte` re-renders `ExplainPlanViewer.svelte`.

The in-flight unified-grid plan (`2026-04-13-002`) introduces `sql_classify.rs` with `QueryClass::MetaCommand` for `EXPLAIN`. This plan's `tabs.executeExplain` must call `api.executeQuery` (not `api.executeQueryPaged`) to avoid CTE wrapping — which is already the correct path since `MetaCommand` falls back to `execute_query` directly.

### Error & Failure Propagation

- `buildExplainSql` throws `UnsupportedExplainError` for MSSQL/Oracle → caught in `executeExplain` → sets `result: { kind: "error", message: "EXPLAIN not supported for this database in v1" }` on the plan tab
- JSON parse failure (malformed Postgres/MySQL output) → `parseExplainResult` returns `[]` nodes with an error flag → `ExplainPlanViewer` shows "Could not parse plan"
- Query execution error (e.g. syntax error in the user's SQL) → `QueryEvent::Error` → existing error handling in `executeExplain`, displayed in the plan tab

### State Lifecycle Risks

- Explain tab opened but connection dropped mid-stream → `executeExplain` catches the error and marks the tab `kind: "error"`. No orphaned state since `ExplainPlanState` is only set on `Done`.
- User closes explain tab while streaming → existing tab-close logic disposes the store state; the streaming callback checks `tabId` existence before writing (same pattern as existing `executeQuery`).

### API Surface Parity

- `api/tauri-adapter.ts` and `api/web-adapter.ts` both expose `executeQuery` — no changes needed there since EXPLAIN goes through the same path.
- TUI has its own execution path (`tui-app/src/app.rs`) — plan parser and SQL builder must be re-implemented in Rust (not shared with the TypeScript layer).

### Integration Test Scenarios

1. User runs `SELECT * FROM users` then presses `Mod-Shift-Enter` on PostgreSQL → explain tab opens showing JSON plan tree with cost annotations
2. User runs a query with a syntax error then triggers EXPLAIN → error propagates correctly, no crash, error shown in explain tab
3. User triggers EXPLAIN on SQLite → `EXPLAIN QUERY PLAN` constructs correct SQL, parent/child rows reconstruct into a tree
4. User triggers EXPLAIN on MSSQL → `UnsupportedExplainError` shown with actionable message
5. User expands/collapses nodes in `ExplainPlanViewer` → `flatVisible` updates correctly, no reactive loop

---

## Acceptance Criteria

### Functional Requirements

- [ ] `Mod-Shift-Enter` (desktop) and `p` (TUI results pane) trigger EXPLAIN on the current query
- [ ] EXPLAIN tab opens alongside the original query results tab (desktop); TUI toggles in-place
- [ ] PostgreSQL: `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS)` is used; plan tree is parsed from the `QUERY PLAN` JSON column
- [ ] MySQL 8+ / MariaDB 10.6+: `EXPLAIN FORMAT=JSON` is used; plan tree is parsed from the JSON column
- [ ] SQLite: `EXPLAIN QUERY PLAN` is used; tree is reconstructed from `id`/`parent` columns
- [ ] MSSQL and Oracle show a clear "not supported in v1" message — no crash
- [ ] Plan tree is collapsible/expandable (click or `Enter`)
- [ ] Each node shows: node type, relation/index name, estimated cost, actual time (if ANALYZE), row estimate vs actual
- [ ] High-cost nodes are visually highlighted (desktop: color-coded; TUI: bold or `>` marker)
- [ ] EXPLAIN SQL is never CTE-wrapped or paginated
- [ ] Original query result tab is unaffected when explain tab is opened

### Non-Functional Requirements

- [ ] Plan viewer loads within the same latency as a normal query (no extra round trips)
- [ ] No Svelte 5 `$derived` reactive loop (write-back guard)
- [ ] TUI plan view does not use `Table` widget with `Min(15)` constraints — uses `List`

### Quality Gates

- [ ] TypeScript types added for `PlanNode`, `ExplainPlanState`
- [ ] `buildExplainSql` and `parseExplainResult` have unit tests covering all three supported dialects and the unsupported-dialect error case
- [ ] Manual QA on each supported database type

---

## Success Metrics

- Developer can go from query to annotated plan in one keystroke without leaving the tool
- Plan tree clearly communicates which nodes are most expensive (no need to read raw cost numbers)
- Zero regressions in existing query execution flow

---

## Dependencies & Prerequisites

- In-flight plan `2026-04-13-002-feat-unified-grid-infinite-scroll-cte-plan.md` introduces `sql_classify.rs` / `QueryClass::MetaCommand`. If merged before this, verify that `executeExplain` uses `executeQuery` (not `executeQueryPaged`) — classification already handles CTE exclusion.
- In-flight plans `2026-04-13-001` (TUI URL connection) and `2026-04-13-003` (TUI query history) both modify `AppMode` in `tui-app/src/app.rs`. This plan deliberately avoids a new `AppMode` to sidestep those conflicts — but rebase after both merge anyway to pick up any `handle_key` restructuring.

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Mitigation |
|---|---|---|
| Postgres JSON plan shape changes between versions | Low | Parse defensively — use optional chaining, fallback to raw JSON display if known fields are absent |
| MySQL EXPLAIN FORMAT=JSON not available on older versions | Medium | Detect MySQL < 8.0 / MariaDB < 10.6 and fall back to `EXPLAIN` tabular output with a notice |
| Svelte 5 reactive loop in ExplainPlanViewer | Medium | Follow the `$derived`-is-read-only rule from `SchemaDdlViewer` fix; unit-test expand/collapse state |
| TUI plan view merge conflict with in-flight AppMode plans | Medium | Keep all EXPLAIN TUI code in a new `result_view` field + `FocusPane::Results` toggle; no `AppMode` changes |
| User triggers EXPLAIN on a very large query with huge plan | Low | Plan output is typically < 100 rows; 1,000-row cap is not a practical concern |

---

## Future Considerations

- **v1.1**: ClickHouse `EXPLAIN` support (tabular text output)
- **v1.2**: MSSQL `SET SHOWPLAN_XML ON` multi-statement support once batch executor is stable
- **v1.3**: Oracle `EXPLAIN PLAN FOR` + `DBMS_XPLAN.DISPLAY` two-statement sequence
- **v2**: Visual graph layout (SVG node-link diagram) for desktop — rendered inside the Tauri webview; file export via Rust command (blob-URL downloads do not work in Tauri)
- **v2**: Side-by-side diff of two plans (before/after optimization)
- **v2**: "Explain on save" auto-run mode for a dedicated tuning workflow

---

## Implementation Checklist

### New files to create

- `src/lib/services/explain-builder.ts` — `buildExplainSql(sql, dbType)`
- `src/lib/services/explain-parser.ts` — `parseExplainResult(columns, rows, dbType)`
- `src/lib/components/ExplainPlanViewer.svelte` — collapsible plan tree component

### Existing files to modify

| File | Change |
|---|---|
| `src/lib/types.ts:94` | Add `PlanNode`, `ExplainPlanState`; extend `QueryTab` with `explainPlan?: ExplainPlanState` |
| `src/lib/stores/tabs.svelte.ts:158` | Add `openExplainPlan`, `updateExplainPlanState`, `executeExplain` methods |
| `src/lib/components/TabbedEditor.svelte:96` | Add `{:else if activeQueryTab.explainPlan}` branch |
| `src/lib/components/SqlEditor.svelte:55` | Add `Mod-Shift-Enter` keymap entry |
| `src/lib/components/EditorToolbar.svelte` | Add "Explain" button + keybinding hint |
| `tui-app/src/app.rs:16` | Add `ResultView` enum, `result_view` field, `explain_nodes`, `explain_list_state` |
| `tui-app/src/app.rs:398` | Add `rebuild_explain_visible`, `parse_explain_result`, `build_explain_sql` |
| `tui-app/src/ui.rs:290` | Add `ResultView::Plan` branch using `List` widget |

---

## Sources & References

### Internal References

- Query execution entry point: `core/src/db/mod.rs:94–175`
- `is_select` heuristic (EXPLAIN already included): `core/src/db/mod.rs:119`
- `QueryEvent` streaming protocol: `core/src/models.rs:131–138`
- `ResultPaneState` discriminated union: `src/lib/types.ts:94–106`
- `QueryTab` with optional mode fields: `src/lib/types.ts:115–124`
- `openSchemaDdl` / `updateSchemaDdlState` pattern: `src/lib/stores/tabs.svelte.ts:158,202`
- `TabbedEditor.svelte` dispatch chain: `src/lib/components/TabbedEditor.svelte:96–152`
- `SqlEditor.svelte` keymap: `src/lib/components/SqlEditor.svelte:51–65`
- Schema tree flat-list pattern (TUI template): `tui-app/src/app.rs:398–482`
- TUI results Table widget (to replace for plan view): `tui-app/src/ui.rs:290–296`
- Svelte 5 `$derived` reactive-loop fix: commit `2d27e51` (`SchemaDdlViewer`)

### Related Plans

- `docs/plans/2026-04-13-002-feat-unified-grid-infinite-scroll-cte-plan.md` — introduces `QueryClass::MetaCommand`; EXPLAIN must use `executeQuery` not `executeQueryPaged`
- `docs/plans/2026-04-13-001-feat-tui-url-connection-add-plan.md` — modifies `AppMode`; rebase after merge
- `docs/plans/2026-04-13-003-feat-query-history-panel-plan.md` — modifies `AppMode`; rebase after merge
- `docs/plans/2026-04-14-001-feat-export-results-plan.md` — confirms blob-URL Tauri constraint; applies to future SVG export
