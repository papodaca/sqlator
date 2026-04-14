---
title: "feat: Result Set Charting — Bar/Line Charts in a New Tab"
type: feat
status: active
date: 2026-04-14
---

# feat: Result Set Charting — Bar/Line Charts in a New Tab

## Overview

Add an "Open as Chart" action to query results and table browse views. Clicking it opens a dedicated chart tab that holds a bar or line chart built from the result data. The tab tracks its source (query SQL or table name + connection) so a Refresh button can re-execute the source and update the chart with fresh data.

This is a quick data-exploration aid — no BI tool, no export to a separate app.

## Problem Statement / Motivation

Reading raw tabular data is cognitive work. For queries like `SELECT country, revenue FROM ...` or `SELECT month, signups FROM ...`, a chart reveals trends and outliers instantly. Today sqlator has no visualization layer; users copy-paste results into spreadsheets or external tools. A chart tab — scoped to one connection, living alongside query tabs — closes that loop without disrupting the editor workflow.

Opening the chart in its own tab (rather than inline beneath the grid) gives the chart enough vertical space to be readable, avoids splitting attention between the data and the visualization, and allows the user to flip back to the raw results in the same connection context.

## Proposed Solution

### User Flow

1. User runs a query (or opens a table browse). Result loads normally in the results pane.
2. An **"Open as Chart"** button appears in the result toolbar when at least one numeric column is detected.
3. Clicking it opens a **new `QueryTab`** with `chartTab` state set, labeled `Chart: <source hint>`.
4. The chart tab shows: chart type selector (Bar / Line), label column dropdown, value column dropdown, the SVG chart, and a **Refresh** button.
5. Refresh re-executes the original source (query SQL or table fetch) and redraws the chart with fresh data.
6. Deduplication: opening a chart for the same source a second time focuses the existing chart tab rather than creating a duplicate.

### Chart Rendering

- **Plain SVG** — zero new npm dependencies for v1. Themed with CSS custom properties (`--color-accent`, `--color-surface-2`, `--color-border`, `--color-text-muted`) for automatic dark/light support.
- Column type detection scans up to the first 50 non-null values per column to handle sparse/nullable numerics.
- Column order follows `result.columns` array (SQL column order), never `Object.keys(rows[0])`.

> **TUI scope:** Out of scope for v1. `ratatui::widgets::BarChart` is available without new dependencies and can be added as a follow-up.

## Technical Approach

### New Types (`src/lib/types.ts`)

```ts
// Source that a chart tab was created from — used for refresh
export type ChartSource =
  | { kind: "query"; sql: string; connectionId: string }
  | { kind: "table"; tableName: string; schema?: string; connectionId: string };

export type ChartType = "bar" | "line";

export interface ChartPoint {
  label: string;
  value: number;
}

export interface ChartTabState {
  source: ChartSource;
  columns: string[];
  rows: Record<string, unknown>[];
  isTruncated: boolean;
  // User selections — preserved across refreshes
  labelCol: string;
  valueCol: string;
  chartType: ChartType;
  // Refresh lifecycle
  isRefreshing: boolean;
  error: string | null;
}
```

Add `chartTab?: ChartTabState` to the existing `QueryTab` interface (alongside `tableBrowse?` and `schemaDdl?`):

```ts
// src/lib/types.ts — QueryTab (line ~115)
export interface QueryTab {
  id: string;
  label: string;
  sql: string;
  isDirty: boolean;
  result: ResultPaneState;
  isExecuting: boolean;
  tableBrowse?: TableBrowseState;
  schemaDdl?: SchemaDdlState;
  chartTab?: ChartTabState;   // ← new
}
```

### Store Extension (`src/lib/stores/tabs.svelte.ts`)

Add two methods following the `openSchemaDdl` / `updateSchemaDdlState` pattern exactly:

```ts
openChartTab(connectionId: string, initialState: ChartTabState) {
  // Deduplicate: if a chart tab with the same source already exists, focus it
  const existing = ct.queryTabs.find(t =>
    t.chartTab && sourceKey(t.chartTab.source) === sourceKey(initialState.source)
  );
  if (existing) {
    // Focus existing tab and update its data (data may be stale)
    connectionTabs = connectionTabs.map(t =>
      t.connectionId === connectionId
        ? { ...t, activeQueryTabId: existing.id,
            queryTabs: t.queryTabs.map(qt =>
              qt.id === existing.id
                ? { ...qt, chartTab: { ...qt.chartTab!, ...initialState } }
                : qt
            )}
        : t
    );
    return;
  }

  const newTab: QueryTab = {
    id: crypto.randomUUID(),
    label: chartTabLabel(initialState.source),
    sql: "",
    isDirty: false,
    result: { kind: "idle" },
    isExecuting: false,
    chartTab: initialState,
  };
  connectionTabs = connectionTabs.map(t =>
    t.connectionId === connectionId
      ? { ...t, queryTabs: [...t.queryTabs, newTab], activeQueryTabId: newTab.id }
      : t
  );
},

updateChartTabState(connectionId: string, queryTabId: string, patch: Partial<ChartTabState>) {
  connectionTabs = connectionTabs.map(t =>
    t.connectionId === connectionId
      ? { ...t, queryTabs: t.queryTabs.map(qt =>
            qt.id === queryTabId && qt.chartTab
              ? { ...qt, chartTab: { ...qt.chartTab, ...patch } }
              : qt
          )}
      : t
  );
},
```

Helper (module-private):
```ts
function sourceKey(s: ChartSource): string {
  return s.kind === "query"
    ? `query:${s.connectionId}:${s.sql}`
    : `table:${s.connectionId}:${s.schema ?? ""}:${s.tableName}`;
}

function chartTabLabel(s: ChartSource): string {
  if (s.kind === "query") {
    const snippet = s.sql.replace(/\s+/g, " ").trim().slice(0, 40);
    return `Chart: ${snippet}${s.sql.length > 40 ? "…" : ""}`;
  }
  return `Chart: ${s.schema ? `${s.schema}.` : ""}${s.tableName}`;
}
```

### Tab View Routing (`src/lib/components/TabbedEditor.svelte`)

Add a new branch in the active-tab renderer (after the existing `schemaDdl` branch, around line 148):

```svelte
{:else if activeQueryTab.chartTab}
  <ChartTabView
    connectionId={activeConnectionTab.connectionId}
    queryTabId={activeQueryTab.id}
    state={activeQueryTab.chartTab}
  />
```

### Chart Tab View (`src/lib/components/ChartTabView.svelte`)

Full-panel component. Receives the `ChartTabState` and handles:
- Column picker UI (label + value dropdowns, chart type selector)
- SVG chart rendering via `<ChartSvg>`
- **Refresh**: re-runs source query/table fetch → calls `tabs.updateChartTabState(..., { isRefreshing: true })` → on completion patches `rows`, `columns`, `isTruncated`, `isRefreshing: false`

Refresh for a query source:
```ts
async function refresh() {
  tabs.updateChartTabState(connectionId, queryTabId, { isRefreshing: true, error: null });
  try {
    const rows: Record<string, unknown>[] = [];
    let columns: string[] = [];
    // Re-use existing executeQueryStream API
    await api.executeQueryStream(state.source.sql, state.source.connectionId, {
      onColumns(cols) { columns = cols; },
      onRow(row) { rows.push(row); },
    });
    tabs.updateChartTabState(connectionId, queryTabId, {
      rows, columns,
      isTruncated: rows.length >= 1000,
      isRefreshing: false,
    });
  } catch (e) {
    tabs.updateChartTabState(connectionId, queryTabId, {
      isRefreshing: false,
      error: String(e),
    });
  }
}
```

For a table source, call the existing table-browse fetch utility with the same `tableName`/`schema`/`connectionId`.

### "Open as Chart" Button Placement

- **`src/lib/components/ResultPane.svelte`** — add button to the results toolbar (visible when `result.kind === "results"` and numeric columns are detected). On click: `tabs.openChartTab(connectionId, buildInitialChartState("query", sql, result))`.
- **`src/lib/components/EnhancedGrid.svelte`** (table browse) — add button to the existing grid toolbar. On click: `tabs.openChartTab(connectionId, buildInitialChartState("table", tableBrowse))`.

`buildInitialChartState` is a utility function (in `src/lib/services/chart-utils.ts` or inline) that:
1. Detects numeric columns (scan up to 50 rows)
2. Pre-selects `valueCol` = first numeric column, `labelCol` = first non-numeric column (or first column)
3. Returns a `ChartTabState` with `chartType: "bar"`, `isRefreshing: false`, `error: null`

### SVG Chart (`src/lib/components/ChartSvg.svelte`)

Pure renderer — no state, no side effects. Props: `{ points: ChartPoint[], type: ChartType, width: number, height: number }`. Uses CSS custom properties for colors. Falls back gracefully for empty `points` with a "No data" message.

## System-Wide Impact

- **Interaction graph**: "Open as Chart" button click → `tabs.openChartTab()` mutates `connectionTabs` `$state` → `TabbedEditor` re-renders via `$derived` `activeQueryTab` → `ChartTabView` mounts. Refresh button → async stream → `updateChartTabState` patches → `ChartTabView` re-renders. No Rust IPC changes.
- **Error propagation**: Refresh errors are stored in `chartTab.error` and displayed in the chart panel. A failed refresh does not close the tab or clear the previous data. A rendering error in `ChartSvg` must not crash `ChartTabView` — wrap in `{#if}` guard with fallback.
- **State lifecycle risks**: `ChartTabState` is persisted in `tabs.svelte.ts` store (in-memory, not persisted to `localStorage` — same as other tab modes). Closing the chart tab drops all chart state; this is expected. Source is stored by value (SQL string / table name), so there is no dangling reference if the source query tab is closed.
- **Persistence**: The existing `PersistedQueryTab` shape in `tabs.svelte.ts` (lines 8–14) must be updated to include `chartTab` source + selections (but not rows/columns — those are re-fetched on restore). Add `chartTab?: { source: ChartSource; labelCol: string; valueCol: string; chartType: ChartType }` to `PersistedQueryTab`.
- **API surface parity**: The Refresh action in the chart tab uses the same `api.executeQueryStream` path that query execution uses. No new Tauri commands needed.
- **Integration test scenarios**:
  1. Open chart from query → chart tab appears with correct source SQL stored.
  2. Open chart from same query twice → second click focuses existing tab, updates data.
  3. Refresh after source data changes → chart updates without losing column selections.
  4. Close chart tab → no orphaned state in store.
  5. Query result with no numeric columns → "Open as Chart" button hidden.

## Acceptance Criteria

- [ ] "Open as Chart" button appears in the result toolbar when `result.kind === "results"` and ≥1 numeric column is detected; hidden otherwise
- [ ] "Open as Chart" button appears in the `EnhancedGrid` toolbar for table browse views when ≥1 numeric column is detected
- [ ] Clicking "Open as Chart" opens a new `QueryTab` labeled `Chart: <source hint>` and activates it
- [ ] Clicking "Open as Chart" for the same source a second time focuses the existing chart tab rather than creating a duplicate
- [ ] Chart tab displays: chart type selector (Bar / Line), label column dropdown (all columns), value column dropdown (numeric columns only)
- [ ] Column order in dropdowns reflects `result.columns` array order (not `Object.keys` order)
- [ ] Changing any selector immediately re-renders the chart without re-fetching data
- [ ] Chart is rendered as SVG using CSS custom property color tokens; renders correctly in light and dark modes
- [ ] Chart tab has a **Refresh** button that re-executes the source query (or table fetch) and updates the chart
- [ ] During refresh, the button shows a loading state; errors are shown inline without closing the tab
- [ ] Numeric column detection scans up to 50 rows (not just `rows[0]`) to handle sparse/nullable columns
- [ ] If the result was truncated to 1,000 rows, the chart tab shows a notice: "Showing first 1,000 rows"
- [ ] `ChartTabState` source + selections are preserved in the `PersistedQueryTab` shape (restored on session reload without data — data is re-fetched)
- [ ] No new npm dependency is added for v1 (plain SVG implementation)
- [ ] Closing the chart tab removes its state from the store with no orphaned data

## Design Notes

**Tab label:** `Chart: SELECT country, rev…` for query sources; `Chart: public.sales` for table sources. Keep it short enough to fit in the tab bar.

**Pre-selection heuristics:** On chart tab open, auto-select first numeric column as `valueCol` and first non-numeric column as `labelCol`. The user can override via dropdowns. These selections persist for the lifetime of the tab (including across refreshes).

**No aggregation in v1.** The chart plots raw rows. A one-line hint at the top of the chart panel ("For aggregated charts, use GROUP BY in your query") sets expectations.

**Column picker placement:** Horizontal strip at the top of the chart panel — type selector + label col dropdown + value col dropdown + Refresh button, all in one row. Chart SVG fills the remaining space below.

## Dependencies & Risks

| Risk | Mitigation |
|------|-----------|
| SVG rendering of 1,000 bar chart entries is unusably dense | Cap visible bars/points at 200 with a "downsampled" badge; bar chart is self-limiting via label legibility |
| CSS custom property values not available at SVG render time | Read via `getComputedStyle(document.documentElement)` in `onMount` / `$effect`; re-read on dark-mode change |
| `$derived` reactive loop when computing chart data | Use `$effect` + `$state` pattern — never `$derived` for computed chart data (see commit `2d27e51`) |
| Duplicate chart tabs if source key hashing is incorrect | Unit-test `sourceKey()` with identical SQL strings to verify deduplication |
| `PersistedQueryTab` shape change breaks existing persisted sessions | Make `chartTab` field optional with a fallback; old sessions without it restore normally |

## Success Metrics

- Charts open correctly from both query results and table browse, on all supported DB types
- Refresh re-executes the source and updates the chart without losing column selections
- No regressions: existing `tableBrowse`, `schemaDdl`, and plain query tabs are unaffected
- "Open as Chart" button hidden when no numeric columns present

## Files to Create / Modify

### New

- `src/lib/components/ChartTabView.svelte` — full-panel chart tab (column pickers, refresh button, chart mount)
- `src/lib/components/ChartSvg.svelte` — pure SVG bar/line renderer `{ points, type, width, height }`

### Modified

- `src/lib/types.ts` — add `ChartSource`, `ChartType`, `ChartPoint`, `ChartTabState`; add `chartTab?: ChartTabState` to `QueryTab`
- `src/lib/stores/tabs.svelte.ts` — add `openChartTab()`, `updateChartTabState()`, `sourceKey()`, `chartTabLabel()`; update `PersistedQueryTab` to include `chartTab` source + selections
- `src/lib/components/TabbedEditor.svelte` — add `{:else if activeQueryTab.chartTab}` branch rendering `<ChartTabView>`
- `src/lib/components/ResultPane.svelte` — add "Open as Chart" button to results toolbar (visible when numeric columns exist)
- `src/lib/components/EnhancedGrid.svelte` — add "Open as Chart" button to table browse toolbar

### Unchanged

- `core/` (Rust) — no backend changes; chart is client-side only
- `tui-app/` — TUI charting deferred to a follow-up

## Sources & References

- **QueryTab interface:** `src/lib/types.ts:115–124`
- **Tab extension pattern (schemaDdl):** `src/lib/stores/tabs.svelte.ts:158–215`
- **ResultPane results block:** `src/lib/components/ResultPane.svelte:73–90`
- **EnhancedGrid toolbar:** `src/lib/components/EnhancedGrid.svelte` (getColTypeCategory at line 149)
- **TabbedEditor tab routing:** `src/lib/components/TabbedEditor.svelte:96–152`
- **CSS custom properties:** `src/app.css` (`--color-accent`, `--color-surface-2`, `--color-border`, etc.)
- **Reactive loop gotcha:** commit `2d27e51` — use `$effect` + `$state`, not `$derived`
- **Blob-URL / file-export in Tauri:** `docs/plans/2026-04-14-001-feat-export-results-plan.md:35–37` — if chart image export ever added, must use Rust IPC
- **Related feature — map view:** `docs/plans/2026-04-14-008-feat-geo-map-view-plan.md` — same tab-open pattern for geographic data
