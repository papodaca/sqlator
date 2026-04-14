---
title: "feat: Keyboard Shortcut Palette (Cmd/Ctrl+K)"
type: feat
status: active
date: 2026-04-14
---

# feat: Keyboard Shortcut Palette (Cmd/Ctrl+K)

## Overview

Add a Cmd/Ctrl+K command palette that fuzzy-searches across connections, tables, open query tabs, and hard-coded commands. Power users can navigate the entire app without touching the mouse.

## Problem Statement / Motivation

All navigation currently requires mouse clicks — switching connections, opening tables, running commands. The roadmap (`docs/roadmap.txt:33–35`) identifies this as a priority for power-user ergonomics. A command palette is the standard pattern for this class of tool (VS Code, Linear, Raycast).

## Proposed Solution

Mount a `CommandPalette.svelte` overlay at the layout root (`+layout.svelte`) triggered by Cmd/Ctrl+K. The palette renders a search input at the top and a scored, categorized result list below. Selecting a result navigates to the target or executes the command, then closes. Escape or backdrop click closes without action.

The CodeMirror editor intercepts keydown before `svelte:window`, so Cmd/Ctrl+K must **also** be registered inside CodeMirror's keymap (`SqlEditor.svelte`) at `Prec.highest`.

**Scoping decisions:**
- **Tables** — scoped to `tabs.activeConnectionId` (the focused connection tab)
- **Schema fetch on open** — trigger `schemaStore.loadTables(activeConnectionId)` if tables not yet loaded; show a spinner in the tables category while loading
- **Table selection** — opens a table browse tab (same as clicking a table in the schema browser)
- **Saved queries** — not yet implemented in the backend; proxy with open query tabs (`tabs.connectionTabs[*].queryTabs`)
- **Palette suppressed** — while `VaultUnlockPrompt` is visible (check vault lock state before opening)

## Technical Considerations

### Architecture

```
src/lib/components/CommandPalette.svelte   ← new overlay component
src/routes/+layout.svelte                  ← mount palette, add Mod+K handler, add guard
src/lib/components/SqlEditor.svelte        ← add Mod-k to CodeMirror keymap
src/lib/stores/palette.svelte.ts           ← (optional) open/close state, or use layout-local $state
```

**Keyboard trigger path (two separate entry points):**

1. `+layout.svelte` `handleKeydown` — fires when editor is NOT focused. Add `e.key === 'k' && mod` branch, guard with `if (vaultLocked || commandPaletteOpen) return`.
2. `SqlEditor.svelte` CodeMirror keymap — fires when editor IS focused. Add `{ key: 'Mod-k', run: () => { openPalette(); return true; } }` at `Prec.highest`, alongside the existing `Mod-Enter` binding (line 54–62).

The two entry points call the same `openPalette()` function — a simple `$state` boolean lift into `+layout.svelte` or a tiny exported store.

**Guard in `handleKeydown` (critical):**

```ts
// +layout.svelte handleKeydown — add at very top of mod block
if (commandPaletteOpen) return; // prevent Ctrl+T etc. firing behind palette
```

**Overlay anatomy (follow `SqlPreviewModal.svelte` pattern):**

```svelte
<!-- Backdrop -->
<div class="fixed inset-0 bg-black/50 z-[1000]" onclick={close} role="none" />
<!-- Panel -->
<div role="dialog" aria-modal="true" aria-label="Command palette"
     class="fixed top-[20%] left-1/2 -translate-x-1/2 w-[560px] max-h-[60vh]
            bg-[var(--bg-secondary)] rounded-lg shadow-2xl z-[1001] flex flex-col"
     onclick|stopPropagation>
  <input bind:this={inputEl} bind:value={query} placeholder="Search connections, tables, commands…"
         class="..." />
  <ul><!-- result items --></ul>
</div>
<svelte:window onkeydown={handlePaletteKey} />
```

**Data sources (all synchronous after initial load):**

| Category | Source | Notes |
|---|---|---|
| Connections | `connections.list` | Always loaded on mount |
| Tables | `schemaStore.getState(activeConnectionId).tables` | May be empty; trigger load on open |
| Open tabs | `tabs.connectionTabs.flatMap(ct => ct.queryTabs)` | Always in memory |
| Commands | Hard-coded array | ~8–12 items |

**Fuzzy matching:**

No fuzzy library is currently installed. Use a simple inline scorer for MVP: score each item by whether `query` is a contiguous substring of the item label (case-insensitive), then by match position (earlier = higher score). This matches the existing `SchemaTree.svelte` precedent (line 22, `includes()` filter). If `fuse.js` is later desired, it can be swapped in without changing the component interface.

**Z-index:** Use `z-index: 1000` (backdrop) and `z-index: 1001` (panel). Current max is `500` (`SqlPreviewModal`). This clears the entire existing stack.

### Implementation Plan

#### Phase 1 — Trigger wiring

1. Add `let commandPaletteOpen = $state(false)` to `+layout.svelte`
2. Add guard + Mod+K branch to `handleKeydown` (`+layout.svelte:59–92`)
3. Add `Mod-k` keymap entry to CodeMirror in `SqlEditor.svelte:54–62`
4. Mount `{#if commandPaletteOpen}<CommandPalette onclose={...} />{/if}` in `+layout.svelte:107–112`

#### Phase 2 — CommandPalette component (MVP)

5. Create `src/lib/components/CommandPalette.svelte`
6. Render: search input (auto-focused on mount), categorized result list, backdrop
7. Implement inline substring scorer
8. Wire Arrow Up/Down + Enter navigation; Escape + backdrop click to close
9. Trigger `schemaStore.loadTables(activeConnectionId)` on mount if not loaded
10. Wire result actions: `tabs.openConnection`, `tabs.openTableBrowse`, `tabs.setActiveQueryTab`, command dispatch

#### Phase 3 — Commands list

11. Define hard-coded command registry (array of `{ label, action }` objects):
    - New query tab
    - New connection
    - Refresh schema
    - Close active tab
    - Export results (if grid has data)

#### Phase 4 — TUI support (separate)

12. Add `AppMode::CommandPalette` to `tui-app/src/app.rs` enum
13. Add `previous_mode: AppMode` field to `App` struct for restore-on-escape
14. Add `CONTROL + k` branch in `handle_workspace_key` and `handle_connection_list_key`
15. Implement TUI palette panel using `ratatui` `Block` + `List` widgets with inline filter

> **Coordinate** TUI `AppMode` changes with in-flight plans:
> `2026-04-13-001` (`AppMode::NewConnection`) and `2026-04-13-003` (`AppMode::History`)
> all modify the same `match self.mode` block. Rebase carefully.

## System-Wide Impact

- **Interaction graph**: Cmd+K → `handleKeydown` OR CodeMirror keymap → sets `commandPaletteOpen = true` → `CommandPalette` mounts → on mount: checks vault lock, triggers schema load, focuses input. On close: sets `commandPaletteOpen = false` → previous focus lost unless explicitly restored.
- **Error propagation**: Schema load failure in `schemaStore.loadTables` is currently silenced; palette should show "Could not load tables" inline in the table category if the store exposes an error state.
- **State lifecycle risks**: Palette open state is transient (`$state` boolean in layout). No persistence risk. Schema load is idempotent (`schemaStore` guards duplicate loads internally).
- **API surface parity**: No new Tauri commands needed for MVP — all data comes from existing frontend stores.
- **Integration test scenarios**:
  - Open palette while CodeMirror editor is focused → palette opens (not swallowed by CodeMirror)
  - Open palette while VaultUnlockPrompt is visible → palette does NOT open
  - Press Ctrl+T while palette is open → new tab is NOT created (guard active)
  - Select table from palette → table browse tab opens on correct connection
  - Escape from palette → focus returns to previously focused element

## Acceptance Criteria

- [ ] Cmd/Ctrl+K opens the palette from any app state, including when a CodeMirror editor tab is focused
- [ ] The palette does NOT open while `VaultUnlockPrompt` is visible
- [ ] Global shortcuts (Ctrl+T, Ctrl+W, etc.) do NOT fire while the palette is open
- [ ] Search input is auto-focused on open
- [ ] Results are filtered in real-time as the user types (no submit required)
- [ ] Results are categorized: Connections, Tables, Open Tabs, Commands
- [ ] Keyboard navigation: Arrow Up/Down moves selection, Enter executes, Escape closes
- [ ] Backdrop click closes the palette
- [ ] Selecting a connection navigates to (or focuses) that connection tab
- [ ] Selecting a table opens a table browse tab for that table on the active connection
- [ ] Selecting an open tab switches to that tab
- [ ] Executing a command performs the action and closes the palette
- [ ] Focus is restored to the previously focused element on close
- [ ] Schema is loaded for the active connection if not already available when the palette opens
- [ ] A loading indicator appears in the tables category while schema is loading
- [ ] Z-index is 1000/1001 — palette renders above all existing overlays
- [ ] TUI: Ctrl+K opens a command palette mode; Escape returns to previous mode

## Success Metrics

- Palette opens reliably from all app states including the SQL editor
- No regression in existing global keyboard shortcuts
- Fuzzy filter returns relevant results within one keystroke latency

## Dependencies & Risks

| Risk | Severity | Mitigation |
|---|---|---|
| CodeMirror swallows Cmd+K | Critical | Add `Mod-k` to CodeMirror keymap at `Prec.highest` in `SqlEditor.svelte:54–62` |
| Other Ctrl shortcuts fire through palette | High | Guard at top of `handleKeydown` with `if (commandPaletteOpen) return` |
| Tables empty for unvisited connections | Medium | Trigger `schemaStore.loadTables(activeConnectionId)` on palette open; show spinner |
| TUI AppMode merge conflict | Medium | Coordinate with plans `2026-04-13-001` and `2026-04-13-003` before touching `app.rs` |
| Focus not restored on close | Low | Capture `document.activeElement` before palette opens; call `.focus()` on close |

## Sources & References

### Internal References

- Global keydown handler: `src/routes/+layout.svelte:59–92`
- App shell (palette mount point): `src/routes/+layout.svelte:107–112`
- CodeMirror keymap (Mod-Enter precedent): `src/lib/components/SqlEditor.svelte:54–62`
- Modal overlay pattern: `src/lib/components/SqlPreviewModal.svelte`
- Backdrop + z-index pattern: `src/lib/components/ContextMenu.svelte`
- Connections store: `src/lib/stores/connections.svelte.ts:8,30`
- Schema/table store: `src/lib/stores/schema.svelte.ts:45–130`
- Tab state store: `src/lib/stores/tabs.svelte.ts:27–520`
- TypeScript types (ConnectionInfo, QueryTab, TableInfo): `src/lib/types.ts:19–34,113–133,234–239`
- TUI AppMode enum: `tui-app/src/app.rs:17–21`
- Roadmap entry: `docs/roadmap.txt:33–35`

### Related Plans

- `docs/plans/2026-04-13-001-feat-tui-url-connection-add-plan.md` — adds `AppMode::NewConnection` (coordinate TUI changes)
- `docs/plans/2026-04-13-003-feat-query-history-panel-plan.md` — adds `AppMode::History` (coordinate TUI changes)
- `docs/plans/2026-04-14-002-feat-schema-aware-sql-autocomplete-plan.md` — shares schema store loading patterns
