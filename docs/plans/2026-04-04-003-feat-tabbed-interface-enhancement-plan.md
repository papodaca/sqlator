---
title: "feat: Tabbed Interface Enhancement — VS Code-Style Multi-Level Tabs"
type: feat
status: active
date: 2026-04-04
origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md
---

# Tabbed Interface Enhancement

VS Code-style hierarchical tabbed interface for SQLator, enabling multiple database connections with multiple query tabs per connection.

---

## Overview

Transform SQLator's single-panel UI into a professional multi-tab interface with two levels:
1. **Connection tabs** (top level) — one tab per open database connection
2. **Query tabs** (second level) — multiple SQL editor + result panes per connection

This enhancement builds upon the MVP foundation (see origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md) and moves the "Multiple tabs" feature from Future Considerations (line 617) into active development.

---

## Problem Statement

The MVP architecture (see origin: Connection State Machine, lines 88-101) limits users to a single active connection and one query at a time. This creates friction when:
- Switching between databases requires disconnecting and reconnecting
- Comparing query results across databases requires running queries sequentially
- Working on multiple queries simultaneously isn't possible
- No way to keep multiple result sets visible

Real-world SQL workflows often involve multiple connections and concurrent queries — a constraint the MVP's "One active connection at a time" model (line 103) doesn't support.

---

## Proposed Solution

Implement a hierarchical tab system modeled after VS Code's editor interface:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ ┌─────────────────────┐ ┌─────────────────────┐ ┌───────────────────┐  │
│ │ 🔵 Production DB  × │ │ 🟢 Staging DB    × │ │ + Add Connection │  │  ← Connection Tabs
│ └─────────────────────┘ └─────────────────────┘ └───────────────────┘  │
├─────────────────────────────────────────────────────────────────────────┤
│ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌─────────────┐  │
│ │ Query 1     × │ │ users.sql   × │ │ Query 3     × │ │ + New Query │  │  ← Query Tabs
│ └───────────────┘ └───────────────┘ └───────────────┘ └─────────────┘  │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     SQL Editor (CodeMirror 6)                   │   │
│  │                                                                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     Result Grid (TanStack Virtual)              │   │
│  │                                                                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Key Decisions (from origin plan)

Carried forward from MVP plan:
- **CodeMirror 6** for SQL editor (see origin: lines 432-442, Monaco rejected due to Tauri CSP issues)
- **TanStack Virtual** for result grids (see origin: lines 330-341)
- **`DashMap<String, AnyPool>`** for concurrent connection pools (see origin: lines 237-248, 450-452)
- **tauri-plugin-store** for persisted metadata (see origin: lines 152, 549)
- **keyring** for credential storage (see origin: lines 76-86)

---

## Technical Approach

### Architecture Changes

#### Frontend Component Hierarchy

```
App.svelte
├── Sidebar.svelte              # Connection list (existing)
├── TabbedEditor.svelte         # NEW: Container for both tab levels
│   ├── ConnectionTabBar.svelte # NEW: Top-level connection tabs
│   └── ConnectionTab.svelte    # NEW: Per-connection container
│       ├── QueryTabBar.svelte  # NEW: Query tabs within connection
│       └── QueryTab.svelte     # NEW: SQL editor + results
│           ├── SqlEditor.svelte       # (existing)
│           └── ResultPane.svelte      # (existing)
```

#### Rust State Changes

```rust
// src-tauri/src/state.rs
use dashmap::DashMap;
use sqlx::AnyPool;
use std::collections::HashMap;

pub struct AppState {
    pub pools: DashMap<String, AnyPool>,           // connection_id -> pool
    pub connection_tabs: tokio::sync::Mutex<Vec<ConnectionTab>>,
}

pub struct ConnectionTab {
    pub connection_id: String,
    pub query_tabs: Vec<QueryTab>,
    pub active_query_id: Option<String>,
}

pub struct QueryTab {
    pub id: String,           // UUID
    pub label: String,        // "Query 1", "users.sql", etc.
    pub sql: String,          // Current query text
    pub is_dirty: bool,       // Unsaved changes
}
```

### IPC Command Changes

| Command | Changes |
|---------|---------|
| `connect_database` | Now creates a new connection tab; connection_id returned |
| `disconnect_database` | New: closes connection tab, removes pool from DashMap |
| `get_open_tabs` | New: returns all connection tabs + query tabs state |
| `save_tab_state` | New: persists tab layout to tauri-plugin-store |
| `create_query_tab` | New: creates query tab under connection |
| `close_query_tab` | New: closes query tab, prompts if dirty |
| `rename_query_tab` | New: renames query tab label |
| `execute_query` | Now takes `connection_id` + `query_id` params |

### Tab State Persistence

```typescript
// Stored in tauri-plugin-store under key "tabState"
interface PersistedTabState {
  connectionTabs: ConnectionTabState[];
  activeConnectionId: string | null;
}

interface ConnectionTabState {
  connectionId: string;
  queryTabs: QueryTabState[];
  activeQueryId: string | null;
}

interface QueryTabState {
  id: string;
  label: string;
  sql: string;
}
```

---

## Implementation Phases

### Phase 1: Query Tab Infrastructure

**Goal:** Multiple query tabs within a single connection.

**Frontend tasks:**
- [ ] Create `src/lib/stores/tabs.svelte.ts` — tab state management with `$state`
- [ ] Create `src/lib/components/QueryTabBar.svelte` — horizontal scrollable tab bar
- [ ] Create `src/lib/components/QueryTab.svelte` — individual tab with close button
- [ ] Update `App.svelte` to include `QueryTabBar` above editor
- [ ] Add "+" button to create new query tabs
- [ ] Implement tab switching with `$derived` for active tab content
- [ ] Add keyboard shortcut: `Ctrl/Cmd+T` to create new query tab
- [ ] Add keyboard shortcut: `Ctrl/Cmd+W` to close current query tab

**Rust tasks:**
- [ ] Add `QueryTab` struct to `state.rs`
- [ ] Implement `create_query_tab` command
- [ ] Implement `close_query_tab` command
- [ ] Implement `get_open_tabs` command
- [ ] Update `execute_query` to accept `query_id`

**UI specifications:**
```typescript
// src/lib/components/QueryTabBar.svelte
interface Props {
  tabs: QueryTab[];
  activeId: string;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, label: string) => void;
}

// Tab styling
const TAB_HEIGHT = '36px';
const TAB_MIN_WIDTH = '100px';
const TAB_MAX_WIDTH = '200px';
const TAB_PADDING = '12px';
```

**Success criteria:**
- [ ] Can create multiple query tabs
- [ ] Each tab maintains independent SQL text
- [ ] Switching tabs preserves query text
- [ ] Close button works (middle-click also supported)
- [ ] Ctrl+T creates new tab
- [ ] Tab bar horizontally scrolls when overflowing

---

### Phase 2: Connection Tab Infrastructure

**Goal:** Multiple connection tabs, each with their own query tabs.

**Frontend tasks:**
- [ ] Create `src/lib/components/TabbedEditor.svelte` — main container
- [ ] Create `src/lib/components/ConnectionTabBar.svelte` — top-level tabs
- [ ] Create `src/lib/components/ConnectionTab.svelte` — per-connection container
- [ ] Update `Sidebar.svelte` click behavior: creates new connection tab (not replaces)
- [ ] Implement connection tab close with disconnect
- [ ] Add "+" button in connection tab bar for new connection (opens connection form)
- [ ] Update `tauri.conf.json` window title to show active connection name

**Rust tasks:**
- [ ] Update `AppState` to track `connection_tabs`
- [ ] Implement `disconnect_database` command (closes pool, removes from DashMap)
- [ ] Update `connect_database` to create connection tab entry
- [ ] Implement connection-level query tab management

**State model:**
```rust
// When user clicks connection in sidebar:
// 1. Check if connection tab already open
// 2. If yes: focus existing tab
// 3. If no: connect, create tab, focus

// When closing connection tab:
// 1. Check for dirty query tabs
// 2. Prompt if any dirty: "Close without saving?"
// 3. Close pool (DashMap.remove)
// 4. Remove tab from state
```

**Success criteria:**
- [ ] Clicking connection in sidebar opens new tab or focuses existing
- [ ] Each connection tab shows connection name with color indicator
- [ ] Closing connection tab disconnects from database
- [ ] Multiple connections can be open simultaneously
- [ ] Switching connection tabs switches active pool
- [ ] Window title reflects active connection

---

### Phase 3: Tab Persistence & Restoration

**Goal:** Tab state persists across app restarts.

**Frontend tasks:**
- [ ] Implement `save_tab_state()` on tab changes (debounced)
- [ ] Load persisted state on app mount
- [ ] Reconnect to databases for persisted connection tabs
- [ ] Restore query tab SQL text

**Rust tasks:**
- [ ] Implement `save_tab_state` command
- [ ] Implement `restore_tab_state` command
- [ ] Store tab state in `tauri-plugin-store` under key `tabState`

**Persistence behavior:**
```typescript
// On tab change (debounced 500ms):
// 1. Serialize current tab state
// 2. Save to tauri-plugin-store

// On app startup:
// 1. Load persisted tab state
// 2. For each connection tab:
//    a. Attempt to reconnect (async, non-blocking)
//    b. Show "Reconnecting..." state
//    c. On failure: show error, keep tab with disconnected state
// 3. Restore query tabs with SQL text
```

**Error handling:**
- Connection no longer exists → Show error dialog, remove tab
- Auth failure → Prompt for updated credentials
- Network timeout → Show "Connection failed" with retry button

**Success criteria:**
- [ ] Tab state saved on changes
- [ ] App restart restores previous session
- [ ] Failed reconnections show clear error state
- [ ] Dirty tabs retain SQL text even if connection fails

---

### Phase 4: Tab UI Polish & Keyboard Navigation

**Goal:** Professional tab UX matching VS Code patterns.

**UI tasks:**
- [ ] Implement horizontal scroll with scroll buttons for overflow
- [ ] Add "..." overflow menu showing all tabs when many open
- [ ] Show close button on hover for inactive tabs
- [ ] Show close button always for active tab
- [ ] Middle-click to close tabs
- [ ] Right-click context menu: "Close", "Close Others", "Close All"
- [ ] Drag-and-drop tab reordering (optional: Phase 4 stretch)
- [ ] Dirty indicator (dot) for unsaved query changes
- [ ] Tab tooltips showing full label + connection info

**Keyboard navigation:**
- [ ] `Ctrl/Cmd+Tab` — cycle through connection tabs
- [ ] `Ctrl/Cmd+Shift+Tab` — reverse cycle connection tabs
- [ ] `Ctrl/Cmd+1-9` — jump to nth connection tab
- [ ] `Ctrl/Cmd+PageUp/Down` — cycle query tabs within connection
- [ ] `Ctrl/Cmd+Shift+[` / `]` — previous/next query tab (VS Code style)
- [ ] Arrow keys navigate within tab bar when focused
- [ ] Home/End jump to first/last tab

**Accessibility:**
- [ ] WAI-ARIA `tablist`, `tab`, `tabpanel` roles
- [ ] `aria-selected` state on active tab
- [ ] `aria-controls` linking tabs to panels
- [ ] Focus indicator with high contrast
- [ ] Screen reader announcements for tab changes

**Success criteria:**
- [ ] All keyboard shortcuts functional
- [ ] Tab overflow handled gracefully
- [ ] Middle-click closes tabs
- [ ] Right-click menu works
- [ ] Screen reader navigates tabs correctly

---

## System-Wide Impact

### Interaction Graph

1. **New connection from sidebar:**
   - Click connection → `connect_database` command → `AnyPool::connect()` → pool stored in `DashMap` → new `ConnectionTab` created → first `QueryTab` auto-created → state saved to `tauri-plugin-store`

2. **Query execution in multi-tab context:**
   - `execute_query(connection_id, query_id, sql)` → pool from `DashMap.get(connection_id)` → query runs → `QueryEvent` streamed via Channel → results stored by `query_id` → active tab shows results

3. **Tab close with dirty state:**
   - Close clicked → check `is_dirty` flag → if true, show confirmation dialog → on confirm: `close_query_tab(query_id)` → SQL text discarded → state saved

4. **App restart restoration:**
   - App loads → `get_tab_state()` from store → for each `ConnectionTab` → `connect_database()` in background → `ConnectionTab` shows loading state → on connect: restore `QueryTab` SQL text → on fail: show error banner with retry

### Error Propagation

| Error Type | Origin | Handling |
|-----------|--------|---------|
| Connection already open | Frontend check | Focus existing tab instead of creating new |
| Query tab close with dirty | Frontend modal | Confirmation dialog, save prompt |
| Pool creation fails | `sqlx::Error` | Show error in connection tab, keep tab with disconnected state |
| Reconnection fails on restore | `sqlx::Error` | Show "Connection failed" banner with Retry button |
| Tab state save fails | `tauri-plugin-store` | Log warning, continue (non-critical) |
| Too many connections | Memory pressure | Warn when >10 connection tabs, suggest closing |

### State Lifecycle Risks

- **Orphaned query results:** When closing a query tab, any in-flight query should be cancelled. Track `CancellationToken` per query tab.
- **Memory leak with many tabs:** Each `QueryTab` holds SQL text. Limit max query tabs to 20 per connection. Implement LRU eviction with warning.
- **Race condition on rapid tab switches:** If user switches tabs while query executing, ensure results go to correct tab. Include `query_id` in `QueryEvent` responses.
- **Partial restoration:** If only some connections restore successfully, show connection tabs for all, with error state for failed ones.

### Integration Test Scenarios

1. **Multi-connection workflow:** Open connection A → create query tab → run query → open connection B → create query tab → run query → switch back to A → verify first query results still visible
2. **Tab persistence:** Open 3 connections with 2 queries each → close app → reopen → verify all 6 query tabs restored with SQL text
3. **Dirty tab warning:** Edit SQL in query tab → attempt close → verify confirmation dialog → cancel → verify tab remains open
4. **Connection failure restoration:** Open connection → close app → delete database → reopen → verify connection tab shows error state with retry option
5. **Concurrent queries:** In connection A, run long query → switch to connection B → run quick query → verify results display correctly for B without waiting for A

---

## Acceptance Criteria

### Functional Requirements

- [ ] **AC-01** User can open multiple database connections simultaneously, each in its own tab
- [ ] **AC-02** Each connection tab shows the connection name with its color indicator
- [ ] **AC-03** Clicking an already-open connection in sidebar focuses that tab (doesn't create duplicate)
- [ ] **AC-04** User can create multiple query tabs within each connection
- [ ] **AC-05** Each query tab has its own SQL editor and result pane, independent of others
- [ ] **AC-06** `Ctrl/Cmd+T` creates a new query tab in the current connection
- [ ] **AC-07** `Ctrl/Cmd+W` closes the current query tab
- [ ] **AC-08** Middle-click on tab closes it
- [ ] **AC-09** Close button (×) appears on hover for inactive tabs, always visible on active tab
- [ ] **AC-10** Right-click on tab shows context menu: Close, Close Others, Close All
- [ ] **AC-11** Tab bars horizontally scroll when tabs overflow the viewport
- [ ] **AC-12** "+" button in connection tab bar opens connection form dialog
- [ ] **AC-13** "+" button in query tab bar creates new query tab
- [ ] **AC-14** Switching connection tabs changes the active database connection
- [ ] **AC-15** Switching query tabs within a connection shows different SQL/results
- [ ] **AC-16** Closing a connection tab disconnects from the database
- [ ] **AC-17** Dirty indicator (dot) shows on query tabs with unsaved SQL changes
- [ ] **AC-18** Closing a dirty query tab prompts for confirmation
- [ ] **AC-19** Tab state (connections, queries, SQL text) persists across app restarts
- [ ] **AC-20** On restoration, connections attempt to reconnect; failures show error state with retry
- [ ] **AC-21** `Ctrl/Cmd+Tab` cycles through connection tabs
- [ ] **AC-22** `Ctrl/Cmd+PageUp/PageDown` cycles through query tabs within connection
- [ ] **AC-23** Arrow keys navigate tabs when tab bar has focus
- [ ] **AC-24** Tab components use WAI-ARIA roles for accessibility

### Non-Functional Requirements

- [ ] Tab switching renders within 50ms for smooth UX
- [ ] App supports at least 10 open connections with 20 query tabs each without degradation
- [ ] Tab persistence write completes within 100ms (debounced)
- [ ] Memory usage increase per query tab is under 1MB (SQL text + metadata only)

### Quality Gates

- [ ] Keyboard-only navigation covers all tab operations
- [ ] Screen reader can announce active tab and tab count
- [ ] No memory leaks when opening/closing many tabs in a session

---

## Dependencies & Prerequisites

### Frontend (additions to package.json)

```json
{
  "devDependencies": {
    "@tailwindcss/vite": "^4"
  }
}
```

Note: No additional npm dependencies needed for tabs — Svelte 5 runes handle state, Tailwind handles styling.

### Rust (additions to Cargo.toml)

No new dependencies — existing `dashmap`, `tauri-plugin-store`, `sqlx` cover all needs.

### Component Library Decision

**Recommendation: Custom components** over Bits UI or shadcn-svelte for this feature:
- Tight integration with Tauri state management
- Full control over horizontal scroll behavior
- No headless library overhead for simple tab UI
- Svelte 5 runes make custom state management straightforward

If future need arises for more complex components (menus, dialogs), consider adding Bits UI then.

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Memory pressure with many open tabs | Medium | Medium | Limit max query tabs per connection (20); implement warning at 15 tabs |
| Reconnection failures on app restore | High | Medium | Show clear error state; provide retry button; don't block other tabs |
| Race condition: query results to wrong tab | Low | High | Include `query_id` in all `QueryEvent` responses; validate on frontend |
| Horizontal scroll UX on narrow windows | Medium | Low | Show scroll buttons; ensure minimum tab width; overflow menu for 10+ tabs |
| Drag-and-drop complexity | Low | Medium | Defer to Phase 4 stretch; not required for v1 |
| Tab state file grows too large | Low | Low | Implement max saved queries; trim SQL text over 10KB |

---

## Future Considerations

- **Drag-and-drop tab reordering** — move tabs within and between connections
- **Tab splitting** — side-by-side query editors (VS Code style)
- **Pinned tabs** — protect certain queries from accidental close
- **Tab groups** — save/load named tab layouts
- **Export tab state** — share query sets between team members

---

## File Structure (Additions)

```
sqlator/
├── src-tauri/
│   └── src/
│       ├── state.rs              # Updated: ConnectionTab, QueryTab structs
│       └── commands/
│           └── tabs.rs           # NEW: tab management commands
├── src/
│   ├── lib/
│   │   ├── stores/
│   │   │   └── tabs.svelte.ts    # NEW: tab state management
│   │   └── components/
│   │       ├── TabbedEditor.svelte       # NEW
│   │       ├── ConnectionTabBar.svelte   # NEW
│   │       ├── ConnectionTab.svelte      # NEW
│   │       ├── QueryTabBar.svelte        # NEW
│   │       └── QueryTab.svelte           # NEW
│   └── App.svelte               # Updated: include TabbedEditor
```

---

## Sources & References

### Origin Document

- **Origin:** [docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md](docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md)
  - Connection State Machine (lines 88-101) — single connection constraint
  - `DashMap<String, AnyPool>` pattern (lines 237-248, 450-452)
  - Future Considerations: Multiple tabs (line 617)
  - Key gotchas for Svelte 5 + Tauri (lines 693-700)

### External References

- [VS Code User Interface](https://code.visualstudio.com/docs/getstarted/userinterface) — tab system reference
- [WAI-ARIA Tabs Pattern](https://www.w3.org/WAI/ARIA/apg/patterns/tabs/) — accessibility guidelines
- [PatternFly Tabs Design Guidelines](https://www.patternfly.org/components/tabs/design-guidelines) — hierarchical tabs
- [NN/G Tabs Used Right](https://www.nngroup.com/articles/tabs-used-right/) — UX best practices
- [Bits UI Tabs](https://bits-ui.com/docs/components/tabs) — Svelte 5 headless component reference
- [Svelte 5 Runes](https://svelte-5-preview.vercel.app/docs/runes) — `$state`, `$derived`, `$effect`

### Key Gotchas (from research)

1. **Horizontal scroll:** Show ~20% of next tab edge to indicate overflow (NN/G pattern)
2. **Max tab depth:** Never exceed 2 levels (PatternFly guideline)
3. **Tab activation:** Use automatic activation for fast-loading content, manual for async (WAI-ARIA)
4. **Memory:** Each query tab holds SQL text in memory — implement limits
5. **State sync:** Include identifiers in all async responses to prevent race conditions
