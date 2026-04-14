---
title: "feat: Export Results (CSV, JSON, Markdown)"
type: feat
status: active
date: 2026-04-14
---

# feat: Export Results (CSV, JSON, Markdown)

## Overview

Add an **Export** control to result panes that lets users export loaded query rows as CSV, JSON, or Markdown — delivered either as a clipboard copy or a file saved to `~/Downloads`.

This is a pure serialization feature: no re-fetch, no new DB query. It operates on whatever rows are currently in memory.

## Problem Statement / Motivation

Exporting query results is one of the most frequently requested features in SQL clients. Without it, users must manually copy cell values or screenshot the grid. This is a high-value, low-risk addition.

## Proposed Solution

- **Copy to clipboard**: Format rows client-side in Svelte using `navigator.clipboard.writeText()` (no Rust required — pattern already used in `SchemaDdlViewer.svelte:63`).
- **Save to file**: Add a new Tauri command `export_query_results` in Rust that serializes rows and writes to `~/Downloads/results-<timestamp>.<ext>`, following the established `export_connections` pattern (`commands.rs:1081`).
- **UI**: An Export button/dropdown in `ResultPane.svelte` and `EnhancedGrid.svelte`, visible only when there are rows to export.

## Technical Considerations

### Architecture Impacts

- New Tauri command `export_query_results` in `src-tauri/src/commands.rs`
- Registered in `src-tauri/src/lib.rs` `invoke_handler!` macro
- No new Cargo or npm dependencies needed
- Frontend serialization for clipboard; Rust serialization for file save (avoids sending potentially large row arrays over IPC)

### Critical Gotcha: Blob-URL downloads do NOT work in Tauri's webview

`export_connections` has a comment confirming this: blob URLs and `<a download>` are inert in the Tauri webview. All file writes must go through a Rust `#[tauri::command]`. The frontend only receives the path string back.

### Data Shape

Both result contexts expose the same shape:
```ts
{ columns: string[], rows: Record<string, unknown>[] }
```
Column order must be driven by the `columns: string[]` array — never `Object.keys(rows[0])`.

### NULL Serialization Rules

| Format   | NULL value becomes |
|----------|--------------------|
| CSV      | empty field (not the string `"NULL"`) |
| JSON     | `null` literal |
| Markdown | empty cell |

Do **not** reuse `formatCell` from `ResultGrid.svelte:208` for serialization — it renders `"NULL"` as a display string and calls `JSON.stringify(value)` for objects without CSV-quoting.

### CSV RFC 4180 Compliance

Fields containing `,`, `"`, or newlines must be wrapped in double-quotes; internal double-quotes must be escaped as `""`.

### Row Scope

Export operates on loaded rows (`rows: Record<string, unknown>[]` in state). When `rowCount > rows.length`, the export confirmation message must state: _"Exported N of M total rows — only loaded rows are included."_

### Clipboard API

`navigator.clipboard.writeText()` already works in the Tauri webview (`SchemaDdlViewer.svelte:63`). No `tauri-plugin-clipboard-manager` is needed. Verify that `capabilities/default.json` does not block it — if it does, add `clipboard-manager:allow-write-text`.

### Filename Collision Prevention

`export_connections` uses date-only granularity (`YYYY-MM-DD`), which collides on multiple exports per day. Use second-precision timestamps: `results-2026-04-14T15-30-00.csv`.

### TUI Mode

Out of scope. No `arboard` dependency, no file write changes in `tui-app/`. TUI has no Tauri webview or clipboard API.

## System-Wide Impact

- **API surface parity**: Export applies to both query result panes (`ResultPane.svelte`) and table-browse panes (`EnhancedGrid.svelte`). Both expose `{ columns, rows }` — implementation must cover both entry points.
- **State lifecycle risks**: Export is read-only and stateless — no orphaned state risk.
- **Error propagation**: Rust file write errors must be propagated as a `Result<String, String>` and surfaced as a toast in the frontend (same pattern as other commands).
- **Integration test scenarios**: Export with a row containing NULL, a row with a JSON column (nested object), a row with a comma in a string value (CSV quoting), and an empty result set (0 rows, button should be disabled).

## Acceptance Criteria

- [ ] Export button is visible and enabled only when `result.kind === "results"` with `rows.length > 0`
- [ ] Export button is hidden/disabled for `idle`, `loading`, `empty`, `rowsAffected`, and `error` states
- [ ] CSV output is RFC 4180 compliant: header row, comma-separated, fields with `,`/`"`/newlines double-quoted, `""` for escaped quotes
- [ ] CSV NULLs produce an empty field (not `"NULL"`)
- [ ] JSON output is a JSON array of objects with SQL NULLs serialized as `null`
- [ ] Markdown output is a GFM pipe table with header and separator rows
- [ ] Copy to clipboard places formatted string in system clipboard and shows a 2-second transient confirmation (matching `SchemaDdlViewer` pattern)
- [ ] Save to file writes to `~/Downloads/results-<YYYY-MM-DD-HH-MM-SS>.<ext>` and reveals the file via `api.openPath()`
- [ ] When `rowCount > rows.length`, confirmation message states "Exported N of M rows — only loaded rows included"
- [ ] Export works in both query result panes (`ResultPane`) and table-browse panes (`EnhancedGrid`)
- [ ] Column order in export matches the `columns: string[]` array order

## Key Files

| File | Change |
|------|--------|
| `src/lib/components/ResultPane.svelte` | Add Export button/dropdown; call clipboard or invoke Rust command |
| `src/lib/components/EnhancedGrid.svelte` | Same Export button for table-browse panes |
| `src-tauri/src/commands.rs` | Add `export_query_results(columns, rows, format)` command |
| `src-tauri/src/lib.rs` | Register `export_query_results` in `invoke_handler!` |
| `src/lib/types.ts` | Add `ExportFormat = "csv" \| "json" \| "markdown"` type if needed |

## Dependencies & Risks

- **No new dependencies** needed in either Cargo or npm
- **Clipboard availability**: `navigator.clipboard.writeText()` requires a secure context (HTTPS or `localhost`). In Tauri this is satisfied. Verify capabilities if clipboard is blocked.
- **Large row sets**: Serializing 50k+ rows in JS (for clipboard) may block the main thread briefly. For clipboard path, consider a microtask yield. File export via Rust avoids this entirely.
- **Dependency on unified grid work** (`docs/plans/2026-04-13-002-feat-unified-grid-infinite-scroll-cte-plan.md`): if that plan changes the data model for results, `EnhancedGrid.svelte` export should be coordinated with it.

## Sources & References

### Internal References

- Export file pattern: `src-tauri/src/commands.rs:1081` (`export_connections`)
- Clipboard pattern: `src/lib/components/SchemaDdlViewer.svelte:63`
- Result data type: `src/lib/types.ts:94` (`ResultPaneState`)
- Table browse type: `src/lib/types.ts:286` (`TableQueryResult`)
- `openPath` abstraction: `src/lib/api/adapter.ts:31`
- `formatCell` (do not reuse for serialization): `src/lib/components/ResultGrid.svelte:208`
- Row-limit notice (for "N of M rows" language): `src/lib/components/ResultPane.svelte:74`

### Related Plans

- Unified grid / infinite scroll: `docs/plans/2026-04-13-002-feat-unified-grid-infinite-scroll-cte-plan.md`
