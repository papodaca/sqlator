---
title: "feat: Open / Save .sql Files"
type: feat
status: active
date: 2026-04-14
---

# feat: Open / Save .sql Files

## Overview

Add the ability to open a `.sql` file from disk into a new query tab and save a query tab's content back to a file. The feature spans the full stack: a new `filePath` field on `QueryTab`, two new Tauri commands for file I/O, new `ApiAdapter` methods with Tauri and web implementations, and UI entry points (toolbar buttons + keyboard shortcuts).

## Problem Statement / Motivation

sqlator already has a CodeMirror 6 editor per query tab and persists SQL across sessions, but there is no way to load a pre-existing `.sql` script from disk or save the current query out as a file. This forces users to copy-paste between their file manager and the editor, defeating the purpose of having a first-class SQL editor in a desktop app.

## Proposed Solution

- **Open**: Ctrl+O (or toolbar button) shows a native file picker (Tauri) or `<input type="file">` (web). The chosen file is read and opened into a **new** query tab labeled with the filename. `filePath` is set on the new tab; `isDirty` starts `false`.
- **Save** (Ctrl+S): If the active tab has a `filePath`, write directly. If not (or if the file no longer exists), fall through to Save As.
- **Save As** (Ctrl+Shift+S): Always shows a native save dialog. On confirmation, writes the file and updates `filePath` + `label` on the tab.
- **Web fallback**: `<input type="file">` for Open; `Blob` download for Save/Save As. After a download is triggered, `isDirty` is cleared optimistically (no persistent path concept in web mode).

### Key Decisions

- Always open into a **new** query tab — never replace the active tab's content.
- Tab label = `basename(filePath)` (e.g. `report.sql`, not the full path).
- `*` suffix on the tab label when `filePath` is set **and** `isDirty` (file-backed dirty indicator). The existing dot indicator is kept for non-file tabs.
- Ctrl+S is a no-op on `tableBrowse` and `schemaDdl` tab types.
- File size: warn at 1 MB, reject at 10 MB with a clear error message.
- Encoding: UTF-8 only; strip BOM on read.
- On app restart with a persisted `filePath`: if the file no longer exists on disk, fall through to Save As on next Ctrl+S (do not error on restore).

## Technical Considerations

### Data Model (`types.ts`)

Add `filePath?: string | null` to `QueryTab` and `PersistedQueryTab`:

```ts
// src/lib/types.ts
export interface QueryTab {
  id: string;
  label: string;
  sql: string;
  isDirty: boolean;
  filePath?: string | null;   // ← new
  result: ResultPaneState;
  isExecuting: boolean;
  tableBrowse?: TableBrowseState;
  schemaDdl?: SchemaDdlState;
}
```

`PersistedQueryTab` in `tabs.svelte.ts` must also carry `filePath` so that Save (Ctrl+S) works after app restart.

### Store (`tabs.svelte.ts`)

New / modified methods:

| Method | Description |
|---|---|
| `openFromFile(connectionId, path, content)` | Creates a new query tab via `createQueryTab`, sets `label = basename(path)`, `sql = content`, `filePath = path`, `isDirty = false` |
| `markClean(connectionId, queryTabId, filePath?)` | Sets `isDirty = false`; optionally updates `filePath` and `label` |
| `updateSql()` | No change needed — already sets `isDirty: true` |
| `saveState` / `restoreState` | Must include `filePath` in serialized shape |

### Backend (`src-tauri/`)

Two new Tauri commands in `commands.rs`:

```rust
// src-tauri/src/commands.rs
#[tauri::command]
pub async fn read_sql_file(path: String) -> CmdResult<String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_sql_file(path: String, content: String) -> CmdResult<()> {
    std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())
}
```

Register both in `tauri::generate_handler!` in `lib.rs`.

Add `tauri-plugin-dialog = "2"` to `src-tauri/Cargo.toml` and the permissions to `src-tauri/capabilities/default.json`:

```json
"dialog:allow-open",
"dialog:allow-save"
```

### ApiAdapter (`adapter.ts`, `tauri-adapter.ts`, `web-adapter.ts`)

Two new interface methods:

```ts
// src/lib/api/adapter.ts
openSqlFile(): Promise<{ path: string; content: string } | null>;
saveSqlFile(
  path: string | null,
  content: string,
  suggestedName: string
): Promise<string | null>; // returns chosen path, or null if cancelled
```

**Tauri adapter**: uses `@tauri-apps/plugin-dialog` `open()` / `save()` for the native picker, then calls `invoke("read_sql_file", ...)` / `invoke("write_sql_file", ...)`.

**Web adapter**: for `openSqlFile`, programmatically clicks a hidden `<input type="file" accept=".sql">` and wraps the `change` event in a Promise using `FileReader.readAsText`. For `saveSqlFile`, creates a `Blob` and triggers an `<a download>` click. Returns `null` for `path` (web has no persistent path).

### UI (`EditorToolbar.svelte`, `SqlEditor.svelte`, `QueryTabBar.svelte`)

- **EditorToolbar**: Add "Open File" and "Save" / "Save As" icon buttons.
- **SqlEditor**: Register `Mod-s` (Save) and `Mod-Shift-s` (Save As) in the CodeMirror keymap with `Prec.highest` so the editor consumes them before the browser's native Save Page dialog. Register `Mod-o` on `svelte:window` (safe outside CodeMirror).
- **QueryTabBar**: Tab label already rendered from `tab.label`. Add `*` suffix rendering when `tab.filePath && tab.isDirty`. Existing dot indicator remains for non-file tabs.

## System-Wide Impact

- **Interaction graph**: Ctrl+S → `SqlEditor` keymap handler → `api.saveSqlFile()` → Tauri `write_sql_file` command → `std::fs::write` → on success → `tabs.markClean(connectionId, queryTabId, path)` → `isDirty: false` → tab label re-renders without `*`.
- **Error propagation**: `write_sql_file` returns a `CmdResult` (`Result<(), String>`). The adapter surfaces the error string; `TabbedEditor` or `EditorToolbar` shows a toast/alert. `isDirty` stays `true` on failure.
- **State lifecycle risks**: If the app crashes mid-write, `filePath` is already persisted but the file may be corrupt or missing. On next restart, `filePath` is rehydrated; if the file is gone, Ctrl+S falls through to Save As — no data loss of in-editor content since `sql` is also persisted.
- **API surface parity**: `closeQueryTab` in `tabs.svelte.ts` closes immediately. It must be updated to check `isDirty && filePath` before closing and surface a confirm dialog. `closeOtherQueryTabs` and `closeAllQueryTabs` need a batch-close confirmation (single dialog listing filenames of all dirty file-backed tabs with Save All / Discard All / Cancel options).
- **Tab type guard**: Ctrl+S handler must check that the active query tab is not a `tableBrowse` or `schemaDdl` tab before attempting to read `sql` or invoke the save command.

## Acceptance Criteria

- [ ] Ctrl+O (or toolbar button) opens a native file picker filtered to `.sql` files; selecting a file creates a new query tab labeled `<filename>.sql` with the file content loaded, `isDirty = false`, and `filePath` set
- [ ] Ctrl+O is a no-op when no connection tab is active
- [ ] Ctrl+S on a file-backed tab writes to the existing `filePath` without a dialog; tab clears dirty indicator on success
- [ ] Ctrl+S on a tab with no `filePath` (or whose `filePath` no longer exists on disk) shows a Save As dialog
- [ ] Ctrl+Shift+S always shows a Save As dialog; on confirm, `filePath` and `label` are updated on the tab
- [ ] File-backed dirty tabs show a `*` suffix in the tab label; non-file tabs keep the existing dot indicator
- [ ] Closing a query tab that has `filePath && isDirty` shows a "Save / Discard / Cancel" confirmation dialog
- [ ] "Close Others" / "Close All" with multiple dirty file-backed tabs shows a single batch confirmation listing filenames
- [ ] Closing a connection tab that contains dirty file-backed query tabs prompts for confirmation
- [ ] Files > 10 MB are rejected with a clear error; files between 1 MB and 10 MB show a warning before opening
- [ ] BOM is stripped from read content; non-UTF-8 files show an encoding error (not silent corruption)
- [ ] `filePath` is persisted across app restarts; if the file no longer exists, the tab reopens with the last saved SQL and Ctrl+S triggers Save As
- [ ] Web mode: Open uses `<input type="file">`; Save triggers a Blob download; `isDirty` clears optimistically after download
- [ ] Ctrl+S on a `tableBrowse` or `schemaDdl` tab is a no-op (no write attempted)
- [ ] Tab label uses only the `basename` of the path, not the full path

## Dependencies & Risks

- **`tauri-plugin-dialog` is not yet in the project** — must be added to `Cargo.toml` and `capabilities/default.json`. This is a compile-time and permission-time dependency; missing either will cause silent runtime failures.
- **CodeMirror Ctrl+S interception** — must be registered with `Prec.highest` inside the CodeMirror keymap in `SqlEditor.svelte`. A `svelte:window` handler alone is insufficient: a focused CodeMirror editor will consume the event before it bubbles, and some browsers will show a "Save Page As" dialog.
- **`closeOtherQueryTabs` / `closeAllQueryTabs` are synchronous** — making them async to support confirmation dialogs is a non-trivial refactor of the tab store and the QueryTabBar context menu. This may be scoped to a follow-up if batch-close UX is deferred.
- **Web adapter `saveSqlFile` path** — always returns `null` since the browser has no persistent file path. The tab's `filePath` must not be set in web mode, or Ctrl+S will call the backend write command with a null path and crash.

## Sources & References

- Tab store: `src/lib/stores/tabs.svelte.ts` — `makeQueryTab`, `updateSql`, `renameQueryTab`, `createQueryTab`, `saveState`, `restoreState`
- QueryTab type: `src/lib/types.ts:115–132`
- Editor wiring: `src/lib/components/SqlEditor.svelte` — `createEditor`, `EditorView.updateListener`
- Toolbar: `src/lib/components/EditorToolbar.svelte`
- Tab bar: `src/lib/components/QueryTabBar.svelte`
- ApiAdapter interface: `src/lib/api/adapter.ts`
- Tauri adapter: `src/lib/api/tauri-adapter.ts`
- Web adapter: `src/lib/api/web-adapter.ts`
- Tauri commands: `src-tauri/src/commands.rs`
- Command registration: `src-tauri/src/lib.rs`
- Capabilities: `src-tauri/capabilities/default.json`
- Precedent for web file read: `src/lib/components/ImportDialog.svelte:28–54` (uses `<input type="file">` + `FileReader`)
- Precedent for Tauri file write: `src-tauri/src/commands.rs:1083–1098` (export connections uses `std::fs::write`)
