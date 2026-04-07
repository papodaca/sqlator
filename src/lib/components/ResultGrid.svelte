<script lang="ts">
  import { createVirtualizer } from "@tanstack/svelte-virtual";
  import { get } from "svelte/store";
  import type { ColumnMeta, CellValue, TempRowId } from "$lib/types";
  import { editStore, pkToKey } from "$lib/stores/edit.svelte";
  import GridToolbar from "./GridToolbar.svelte";
  import ContextMenu from "./ContextMenu.svelte";
  import type { ContextMenuItem } from "./ContextMenu.svelte";
  import TextEditor from "./editors/TextEditor.svelte";
  import NumberEditor from "./editors/NumberEditor.svelte";
  import BooleanEditor from "./editors/BooleanEditor.svelte";
  import DateEditor from "./editors/DateEditor.svelte";
  import EnumEditor from "./editors/EnumEditor.svelte";
  import JsonEditor from "./editors/JsonEditor.svelte";
  import TextAreaEditor from "./editors/TextAreaEditor.svelte";

  let {
    columns,
    rows,
    onSave,
  }: {
    columns: string[];
    rows: Record<string, unknown>[];
    onSave: () => void;
  } = $props();

  let scrollEl = $state<HTMLDivElement | null>(null);

  // ── Stable row keys ───────────────────────────────────────────────────────────
  // Each displayed row has a __rowKey:
  //   - added rows:    tempId (e.g. "temp_1")
  //   - existing rows: serialized PK if available, else "row_<original-index>"

  type DisplayRow = Record<string, unknown> & {
    __rowKey: string;
    __isAdded?: true;
    __tempId?: TempRowId;
  };

  function rowKey(row: Record<string, unknown>, originalIndex: number): string {
    const tm = editStore.tableMeta;
    if (!tm?.primaryKey.exists) return `row_${originalIndex}`;
    const pkVal = tm.primaryKey.columns.map((c) => row[c] as CellValue);
    return pkToKey(pkVal.length === 1 ? pkVal[0] : pkVal);
  }

  let displayRows = $derived.by((): DisplayRow[] => {
    const tm = editStore.tableMeta;
    const result: DisplayRow[] = [];

    // Added rows first
    for (const [tempId, addedRow] of editStore.changeSet.added) {
      result.push({ ...addedRow.data, __rowKey: tempId, __isAdded: true, __tempId: tempId });
    }

    // Existing rows, minus deleted
    for (let i = 0; i < rows.length; i++) {
      const row = rows[i];
      const key = rowKey(row, i);
      if (tm?.primaryKey.exists && editStore.changeSet.deleted.has(key)) continue;
      result.push({ ...row, __rowKey: key });
    }
    return result;
  });

  // ── Virtualizer ───────────────────────────────────────────────────────────────

  let virtualizer = $derived(
    scrollEl
      ? createVirtualizer({
          count: displayRows.length,
          getScrollElement: () => scrollEl!,
          estimateSize: () => 36,
          overscan: 10,
        })
      : null,
  );
  let virtualItems = $derived(virtualizer ? get(virtualizer).getVirtualItems() : []);
  let totalSize = $derived(virtualizer ? get(virtualizer).getTotalSize() : 0);

  // ── Editing state — identified by stable rowKey, not index ───────────────────

  interface EditingCell {
    rowKey: string;
    colName: string;
  }
  let editingCell = $state<EditingCell | null>(null);

  // ── Multi-row selection ───────────────────────────────────────────────────────

  let selectedKeys = $state(new Set<string>());
  let lastSelectedKey = $state<string | null>(null);

  function handleRowClick(e: MouseEvent, row: DisplayRow) {
    const key = row.__rowKey;
    if (e.shiftKey && lastSelectedKey) {
      // Extend selection from last selected to current
      const keys = displayRows.map((r) => r.__rowKey);
      const a = keys.indexOf(lastSelectedKey);
      const b = keys.indexOf(key);
      const [lo, hi] = a < b ? [a, b] : [b, a];
      const next = new Set(selectedKeys);
      for (let i = lo; i <= hi; i++) next.add(keys[i]);
      selectedKeys = next;
    } else if (e.metaKey || e.ctrlKey) {
      // Toggle individual row
      const next = new Set(selectedKeys);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      selectedKeys = next;
      lastSelectedKey = next.has(key) ? key : null;
    } else {
      // Single select (unless clicking already-selected row with no modifier — deselect)
      if (selectedKeys.size === 1 && selectedKeys.has(key)) {
        selectedKeys = new Set();
        lastSelectedKey = null;
        return;
      }
      selectedKeys = new Set([key]);
      lastSelectedKey = key;
    }
  }

  function deleteSelected() {
    for (const key of selectedKeys) {
      const row = displayRows.find((r) => r.__rowKey === key);
      if (!row) continue;
      if (row.__isAdded && row.__tempId) {
        editStore.deleteAddedRow(row.__tempId);
      } else {
        editStore.deleteRow(row);
      }
    }
    selectedKeys = new Set();
    lastSelectedKey = null;
    editingCell = null;
  }

  // ── Context menu ──────────────────────────────────────────────────────────────

  let contextMenu = $state<{
    x: number;
    y: number;
    items: ContextMenuItem[];
    targetKey: string;
    targetCol: string;
  } | null>(null);

  function handleContextMenu(e: MouseEvent, row: DisplayRow, colName: string) {
    e.preventDefault();
    // Select row if not already selected
    if (!selectedKeys.has(row.__rowKey)) {
      selectedKeys = new Set([row.__rowKey]);
      lastSelectedKey = row.__rowKey;
    }

    const isNewRow = !!row.__isAdded;
    const colMeta = getColumnMeta(colName);
    const items: ContextMenuItem[] = [];

    if (isColumnEditable(colName, isNewRow)) {
      items.push({ label: "Edit Cell", action: "edit" });
    }
    if (colMeta?.nullable && editStore.isEditable) {
      items.push({ label: "Set to NULL", action: "set-null" });
    }
    items.push({ label: "Add Row", action: "add-row", disabled: !editStore.isEditable });
    if (selectedKeys.size > 1) {
      items.push({ label: `Delete ${selectedKeys.size} rows`, action: "delete-selected", danger: true });
    } else {
      items.push({ label: "Delete Row", action: "delete-row", danger: true, disabled: !editStore.isEditable });
    }

    contextMenu = { x: e.clientX, y: e.clientY, items, targetKey: row.__rowKey, targetCol: colName };
  }

  function handleContextMenuSelect(action: string) {
    if (!contextMenu) return;
    const targetRow = displayRows.find((r) => r.__rowKey === contextMenu!.targetKey);

    switch (action) {
      case "edit":
        if (targetRow) {
          startEdit(targetRow.__rowKey, contextMenu.targetCol);
        }
        break;
      case "set-null":
        if (targetRow) {
          const isNewRow = !!targetRow.__isAdded;
          saveCellEdit(targetRow, contextMenu.targetCol, null, isNewRow);
        }
        break;
      case "add-row":
        handleAddRow();
        break;
      case "delete-row":
        if (targetRow) deleteSingleRow(targetRow);
        break;
      case "delete-selected":
        deleteSelected();
        break;
    }
    contextMenu = null;
  }

  // ── Cell helpers ──────────────────────────────────────────────────────────────

  function formatCell(value: unknown): string {
    if (value === null || value === undefined) return "NULL";
    if (typeof value === "object") return JSON.stringify(value);
    return String(value);
  }

  function isNull(value: unknown): boolean {
    return value === null || value === undefined;
  }

  function getColumnMeta(colName: string): ColumnMeta | undefined {
    return editStore.tableMeta?.columns.find((c) => c.name === colName);
  }

  function getEditorType(colName: string): string {
    const meta = getColumnMeta(colName);
    if (!meta) return "text";
    const t = meta.type;
    if (t === "boolean") return "boolean";
    if (t === "date") return "date";
    if (t === "time") return "time";
    if (t === "datetime" || t === "timestamp") return "datetime";
    if (t === "enum") return "enum";
    if (t === "json" || t === "jsonb") return "json";
    if (
      t === "integer" || t === "bigint" || t === "smallint" ||
      t === "decimal" || t === "numeric" || t === "float" || t === "double"
    ) return "number";
    return "text";
  }

  // New rows are always editable (user explicitly added them — no PK guard needed).
  // Existing rows require the table to be editable (has PK) and the column to be updatable.
  function isColumnEditable(colName: string, isNewRow: boolean): boolean {
    if (isNewRow) {
      const meta = getColumnMeta(colName);
      return meta ? !meta.isGenerated : true;
    }
    if (!editStore.isEditable) return false;
    const meta = getColumnMeta(colName);
    return meta ? meta.isUpdatable : true;
  }

  function getDisplayValue(row: DisplayRow, colName: string): CellValue {
    if (row.__isAdded) {
      const tempId = row.__tempId!;
      return (editStore.changeSet.added.get(tempId)?.data[colName] ?? null) as CellValue;
    }
    return editStore.getCellDisplayValue(row, colName);
  }

  // ── Cell editing ──────────────────────────────────────────────────────────────

  function startEdit(rKey: string, colName: string) {
    const row = displayRows.find((r) => r.__rowKey === rKey);
    if (!row) return;
    const isNewRow = !!row.__isAdded;
    if (!isColumnEditable(colName, isNewRow)) return;
    editingCell = { rowKey: rKey, colName };
  }

  function saveCellEdit(row: DisplayRow, colName: string, newValue: CellValue, isNewRow: boolean) {
    if (isNewRow && row.__tempId) {
      editStore.modifyAddedCell(row.__tempId, colName, newValue);
    } else {
      editStore.modifyCell(row, colName, newValue);
    }
    editingCell = null;
  }

  function cancelEdit() {
    editingCell = null;
  }

  // ── Row operations ────────────────────────────────────────────────────────────

  function handleAddRow() {
    const tempId = editStore.addRow();
    // Auto-start editing the first editable column.
    // No tick/setTimeout needed — $derived is lazy so displayRows is current on next read.
    const firstCol = columns.find((c) => isColumnEditable(c, true));
    if (firstCol) startEdit(tempId, firstCol);
  }

  function deleteSingleRow(row: DisplayRow) {
    if (row.__isAdded && row.__tempId) {
      editStore.deleteAddedRow(row.__tempId);
    } else {
      editStore.deleteRow(row);
    }
    selectedKeys = new Set([...selectedKeys].filter((k) => k !== row.__rowKey));
    if (editingCell?.rowKey === row.__rowKey) editingCell = null;
  }

  // ── Keyboard ──────────────────────────────────────────────────────────────────

  function handleGlobalKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === "n") {
      e.preventDefault();
      if (editStore.isEditable) handleAddRow();
    }
    if ((e.ctrlKey || e.metaKey) && e.key === "s") {
      e.preventDefault();
      if (editStore.hasChanges) onSave();
    }
    if ((e.key === "Delete" || e.key === "Backspace") && !editingCell && selectedKeys.size > 0) {
      if (editStore.isEditable) deleteSelected();
    }
    if (e.key === "Escape") {
      selectedKeys = new Set();
      lastSelectedKey = null;
    }
  }

  function handleCellKeydown(e: KeyboardEvent, row: DisplayRow, colName: string) {
    if (e.key === "Enter" && !editingCell) {
      e.preventDefault();
      startEdit(row.__rowKey, colName);
    }
  }

  // ── Row state display ─────────────────────────────────────────────────────────

  function isRowModified(row: DisplayRow): boolean {
    return editStore.changeSet.modified.has(row.__rowKey);
  }
</script>

<svelte:window onkeydown={handleGlobalKeydown} />

<GridToolbar
  onAddRow={handleAddRow}
  {onSave}
  selectedCount={selectedKeys.size}
  onDeleteSelected={deleteSelected}
  onDeselectAll={() => { selectedKeys = new Set(); lastSelectedKey = null; }}
/>

<div class="grid-wrapper" bind:this={scrollEl}>
  <table>
    <thead>
      <tr>
        {#each columns as col}
          <th>{col}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#if (virtualItems[0]?.start ?? 0) > 0}
        <tr class="spacer" style="height: {virtualItems[0].start}px;">
          <td colspan={columns.length}></td>
        </tr>
      {/if}

      {#each virtualItems as item (item.key)}
        {@const row = displayRows[item.index]}
        {@const isNewRow = !!row?.__isAdded}
        {@const isDeleted = !isNewRow && editStore.changeSet.deleted.has(row.__rowKey)}
        {@const isSelected = selectedKeys.has(row?.__rowKey)}
        {@const modified = !isNewRow && isRowModified(row)}
        <tr
          class:alt={item.index % 2 === 1}
          class:added-row={isNewRow}
          class:deleted-row={isDeleted}
          class:selected-row={isSelected}
          onclick={(e) => handleRowClick(e, row)}
        >
          <!-- Data cells -->
          {#each columns as col}
            {@const isEditing = editingCell?.rowKey === row.__rowKey && editingCell?.colName === col}
            {@const cellModified = !isNewRow && editStore.isCellModified(row, col)}
            {@const editable = isColumnEditable(col, isNewRow)}
            {@const displayVal = getDisplayValue(row, col)}
            <td
              class:null-cell={isNull(displayVal) && !isEditing}
              class:modified-cell={cellModified}
              class:editable-cell={editable && !isDeleted}
              class:editing={isEditing}
              title={isEditing ? undefined : formatCell(displayVal)}
              tabindex={editable ? 0 : undefined}
              ondblclick={(e) => {
                e.stopPropagation();
                startEdit(row.__rowKey, col);
              }}
              onkeydown={(e) => handleCellKeydown(e, row, col)}
              oncontextmenu={(e) => {
                e.stopPropagation();
                handleContextMenu(e, row, col);
              }}
            >
              {#if isEditing}
                {@const editorType = getEditorType(col)}
                {@const colMeta = getColumnMeta(col)}
                {#if editorType === "boolean"}
                  <BooleanEditor
                    value={displayVal}
                    nullable={colMeta?.nullable ?? true}
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else if editorType === "date"}
                  <DateEditor
                    value={displayVal}
                    type="date"
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else if editorType === "time"}
                  <DateEditor
                    value={displayVal}
                    type="time"
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else if editorType === "datetime"}
                  <DateEditor
                    value={displayVal}
                    type="datetime-local"
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else if editorType === "enum"}
                  <EnumEditor
                    value={displayVal}
                    enumValues={colMeta?.enumValues ?? []}
                    nullable={colMeta?.nullable ?? true}
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else if editorType === "json"}
                  <JsonEditor
                    value={displayVal}
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else if editorType === "number"}
                  <NumberEditor
                    value={displayVal}
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {:else}
                  <TextEditor
                    value={displayVal}
                    onSave={(v) => saveCellEdit(row, col, v, isNewRow)}
                    onCancel={cancelEdit}
                  />
                {/if}
              {:else}
                {formatCell(displayVal)}
              {/if}
            </td>
          {/each}
        </tr>
      {/each}

      <!-- Bottom spacer -->
      {#if totalSize > 0 && virtualItems.length > 0}
        {@const lastItem = virtualItems[virtualItems.length - 1]}
        {#if lastItem && lastItem.end < totalSize}
          <tr class="spacer" style="height: {totalSize - lastItem.end}px;">
            <td colspan={columns.length}></td>
          </tr>
        {/if}
      {/if}
    </tbody>
  </table>
</div>

{#if contextMenu}
  <ContextMenu
    x={contextMenu.x}
    y={contextMenu.y}
    items={contextMenu.items}
    onselect={handleContextMenuSelect}
    onclose={() => (contextMenu = null)}
  />
{/if}

<style>
  .grid-wrapper {
    flex: 1;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 13px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  thead {
    position: sticky;
    top: 0;
    z-index: 2;
  }

  th {
    background: var(--color-surface-2);
    text-align: left;
    padding: 0 12px;
    height: 36px;
    line-height: 36px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    color: var(--color-text-muted);
    font-weight: 600;
    border-bottom: 2px solid var(--color-border);
    white-space: nowrap;
    position: relative;
  }

  th:not(:last-child)::after {
    content: "";
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--color-border);
  }

  .spacer td {
    padding: 0;
    border: none;
  }

  td {
    padding: 0 12px;
    height: 36px;
    line-height: 36px;
    border-bottom: 1px solid var(--color-border);
    border-right: 1px solid var(--color-border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 300px;
    position: relative;
  }

  td:last-child {
    border-right: none;
  }

  tr.alt td {
    background: var(--color-surface);
  }

  .null-cell {
    color: var(--color-text-muted);
    font-style: italic;
    opacity: 0.7;
  }

  /* Added row: green tint */
  tr.added-row td {
    background: oklch(0.97 0.03 145) !important;
  }

  :global(.dark) tr.added-row td {
    background: oklch(0.2 0.04 145) !important;
  }

  /* Deleted row: red tint + strikethrough */
  tr.deleted-row td {
    background: oklch(0.97 0.03 25) !important;
    text-decoration: line-through;
    opacity: 0.6;
  }

  :global(.dark) tr.deleted-row td {
    background: oklch(0.2 0.04 25) !important;
  }

  /* Modified cell */
  td.modified-cell {
    background: oklch(0.97 0.08 80) !important;
  }

  :global(.dark) td.modified-cell {
    background: oklch(0.22 0.06 80) !important;
  }

  /* Editable cell */
  td.editable-cell {
    cursor: text;
  }

  td.editable-cell:hover {
    outline: 1px solid var(--color-accent);
    outline-offset: -1px;
  }

  td.editing {
    padding: 0;
    overflow: visible;
  }

  /* Selected row */
  tr.selected-row > td {
    background: oklch(0.93 0.03 250) !important;
  }

  :global(.dark) tr.selected-row > td {
    background: oklch(0.22 0.03 250) !important;
  }

</style>
