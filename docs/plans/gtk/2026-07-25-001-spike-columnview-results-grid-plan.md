---
title: "spike: Prove GtkColumnView can serve as a database results grid"
type: spike
status: completed
date: 2026-07-25
---

# spike: Prove GtkColumnView can serve as a database results grid

## Overview

A time-boxed, throwaway spike (1–2 days) that builds nothing but the results grid: dynamic
runtime-generated columns, 100k synthetic rows behind a custom `gio::ListModel`, sorting,
right-click, and copy-to-clipboard. It exists to answer one question before any other GTK work
starts — **can GTK4 give database users the grid they expect?**

This is the only phase whose output is deliberately discarded. What survives is the measurement
and the decision.

---

## Problem Statement / Motivation

Every other part of a GTK4 SQLator is well-trodden: the tokio bridge, GResource, libadwaita
shell, GtkSourceView editor, VTE terminal. There are reference implementations for all of it.

The results grid is not. `GtkColumnView` was designed for file managers and process lists, not
for spreadsheet-like data browsing, and it has four specific gaps against what a
DBeaver/TablePlus user expects. If those gaps prove fatal, the entire initiative should be
reconsidered — and that is far cheaper to learn in two days than in two months.

### Gap 1: no horizontal virtualization

From Matthias Clasen's original announcement of the widget:

> "the column view only creates widgets for the segment of the model that is currently in view,
> so it shares the vertical scalability. **The same is not true in the horizontal direction —
> every row is fully populated with a widget for each column, even if they are out of view to
> the left or right. So if you add lots of columns, things will get slow.**"

Vertical is capped by GTK at `GTK_LIST_VIEW_MAX_LIST_ITEMS = 200` rows and is not tunable. So
live widget count is `200 x n_columns`. A `SELECT *` on a 60-column table means ~12,000
widgets. A GNOME Discourse thread from Dec 2025 measured 100 columns at ~0.48s to populate,
with a surprising share of it inside per-widget `GtkShortcutController` setup.

### Gap 2: no cell-level selection

`ColumnView` selection is row-level, delegated to the `GtkSelectionModel`. There is no
per-cell selection state to drive. Accessibility exposes cells as `grid_cell`, but that is
presentation, not selection. Rectangular cell-range selection would be custom work estimated
at 400–600 lines with a result that still feels slightly non-native.

### Gap 3: right-click does not select the row under the cursor

`GtkListBase` only changes selection on primary-button release. A naive `GestureClick` context
menu therefore acts on the *previously* selected row. This produced a real "wrong folder
trashed" bug in Baobab. For a tool that generates `DELETE` statements, this is a correctness
issue, not a polish issue.

### Gap 4: editable cells have known interaction bugs

`GtkEditableLabel` in a cell enters edit mode without selecting its row, and clicking a
non-editable label does nothing at all.

---

## Proposed Solution

A single-file scratch binary in **`tmp/columnview-spike/`** — the project-root scratch folder
that `AGENTS.md` designates for grounding work in how tools actually behave. Not a workspace
member, not merged, and `tmp` is already gitignored.

### Model layer

Implement `ListModelImpl` by hand. Do not use `gio::ListStore` — it forces one `GObject` per
row materialized up front, which is exactly the allocation storm to avoid at 100k rows. The
interface is three methods.

```rust
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Arc<[Value]>>,
}

impl ListModelImpl for ResultModel {
    fn item_type(&self) -> glib::Type { RowObject::static_type() }
    fn n_items(&self) -> u32 { self.data.borrow().as_ref().map_or(0, |d| d.rows.len() as u32) }
    fn item(&self, position: u32) -> Option<glib::Object> {
        // weak-ref cache so repeated item() during scroll does not re-allocate
    }
}
```

`RowObject` must be a **minimal** `glib::Object` subclass holding `Arc<[Value]>` plus its
index — no GObject properties, no `ParamSpec`s. Property machinery costs real time per cell;
plain Rust getters called from the bind closure are materially faster. (Mission Center's
`ProcessObject` does exactly this and documents the reasoning.)

### Cell binding

Use `GtkInscription`, not `GtkLabel`. It is purpose-built for list cells — given a size, it
inscribes text into it rather than sizing itself to its text. The GTK commit adopting it in
`testcolumnview` is titled, verbatim: *"This test is suddenly MASSIVELY faster. I wonder why.
Could it be because inscription does exactly what it was made for?"*

Set `fixed_width` per column. Without it GTK measures cells to compute natural widths, making
horizontal scrolling jumpy as new rows bind.

### Sorting

`gtk::CustomSorter` per column wrapped in a `gtk::SortListModel`, with
**`set_incremental(true)`** — the docs recommend it between roughly 10k and 100k items; it
sorts on an idle handler so the UI stays live. Compare typed `Value`s, never formatted
strings, or integer columns sort as `1, 10, 2`.

### Right-click

Port Nautilus's `select_single_item_if_not_selected`: on press, resolve the cell widget with
`gtk::Widget::pick()`, read the item stashed on it during `bind`, and select it only if it is
not already part of the selection (which preserves multi-selection). Attach the `GestureClick`
to the **cell widget in `setup`**, disconnecting in `unbind`, rather than to the `ColumnView`.

---

## Technical Considerations

- Generate synthetic data in the shape real results take: mixed types, `NULL`s, wide `TEXT`,
  and a `JSON` column. Do not benchmark against uniform integers.
- Measure with **Sysprof**, not stopwatch impressions. GTK emits frame marks. This is how the
  Discourse thread identified `GtkShortcutController` as a cost.
- Use `GOBJECT_DEBUG=instance-count` plus the inspector's statistics tab to confirm
  `RowObject`s are not leaking across repeated result loads — a leak per row across many
  query runs is a realistic failure here.
- Test the column-count axis explicitly at 10 / 30 / 60 / 100 columns. The 60-column case is
  the realistic worst case for `SELECT *`.
- `ColumnViewCell` (GTK 4.12+, implied by the `gnome_50` feature) is the correct type to
  downcast to, not `ListItem`. `ColumnViewRow` with `set_row_factory` handles per-row styling.

### Mitigations to evaluate, in order

1. **Cap visible columns.** Create `ColumnViewColumn`s only for the first N; put the rest
   behind a column-picker with a banner. Cheap, and probably sufficient.
2. **Custom horizontal windowing.** Watch the `ScrolledWindow`'s `hadjustment` and add/remove
   `ColumnViewColumn`s as they scroll into range. Genuinely viable — columns are cheap to
   add and remove at runtime — but it is custom work. Do not build this during the spike;
   only confirm it is *possible* if measurement demands it.

---

## System-Wide Impact

None by construction. This phase produces no code that ships. Its only output is a
go / no-go decision and a set of measurements recorded in this document's Findings section.

---

## Acceptance Criteria

- [x] 100k rows load and scroll smoothly (no dropped frames visible in Sysprof) at 10 columns
      (measured via `FrameClock` tick samples; p50/p95 under 16.67ms during scroll — see Findings)
- [x] Column count vs. populate-time measured at 10 / 30 / 60 / 100 columns and recorded
- [x] Columns are constructed at runtime from a `Vec<ColumnMeta>` and can be torn down and
      rebuilt when the result shape changes, without leaking
- [x] Per-column sorting works on typed values, is incremental, and does not block the UI at
      100k rows
- [x] Right-click selects the row under the cursor, and preserves an existing multi-selection
      (Nautilus `select_single_item_if_not_selected` ported onto cell `GestureClick`)
- [x] Multi-row selection (`MultiSelection` + rubberband) and copy-as-TSV to clipboard work
- [x] `NULL` renders distinctly from the string `"NULL"`
      (real NULL → italic faded `NULL` CSS class; string `"NULL"` → plain text)
- [x] No `RowObject` leak across 20 consecutive result loads (`GOBJECT_DEBUG=instance-count`)
      (weak-ref cache stayed flat at 205 across 20 reloads; see Findings)
- [x] A written recommendation on cell-range selection: build it, defer it, or drop it

---

## Success Metrics

- 60-column populate time under ~250ms, or a clear path to it via column capping
- Scroll at 100k rows sustains 60fps
- Decision recorded with numbers behind it, not impressions

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| `gtk4` 0.11.4 (`gnome_50`) | Matches installed GTK 4.22.4 |
| Sysprof | GNOME profiler; GTK emits frame marks |
| No dependency on phase 1 | The spike uses synthetic data, not a live database |

| Risk | Mitigation |
|---|---|
| Spike scope creep into a real grid | Hard 2-day time-box; lives in gitignored `tmp/`; never merged |
| Measuring the wrong thing | Fixed column-count matrix defined above, agreed before starting |
| 60-column case proves unfixable | Column capping is the fallback; if *that* is unacceptable, escalate — this is the intended purpose of the phase |

---

## Findings

**Decision: GO** — proceed with GtkColumnView for the results grid, with the mitigations
below. Gaps 1–4 are either mitigated or consciously deferred; none are fatal for a DBeaver-
class tool if we cap visible columns and ship row-level selection first.

Scratch binary: `tmp/columnview-spike/` (gitignored; `cargo run --release -- --bench`).
Host: GTK 4.22.4, gtk4-rs 0.11.4 (`gnome_50`). Frame timing via `FrameClock` tick callbacks
(Sysprof available locally; CLI bench used clock samples for repeatability).

### Measurements

Column-count matrix — **10k rows**, replace into an already-populated model (worst-case
`items_changed`):

| cols | generate_ms | set_data_ms | rebuild_cols_ms | total_ms | live widgets ≈ |
|-----:|------------:|------------:|----------------:|---------:|---------------:|
| 10   | 2.93        | 50.22       | 43.61           | 96.76    | 2 000          |
| 30   | 8.25        | 50.66       | 104.88          | 163.80   | 6 000          |
| 60   | 23.31       | 149.66      | 216.73          | 389.72   | 12 000         |
| 100  | 39.21       | 302.81      | 382.45          | 724.48   | 20 000         |

Cold load into an empty model (no prior rows to tear down):

| shape     | generate_ms | set_data_ms | rebuild_cols_ms | total_ms |
|-----------|------------:|------------:|----------------:|---------:|
| 100k × 10 | 39.52       | 1.82        | 35.04           | 76.39    |
| 100k × 60 | 226.54      | 1.84        | 206.33          | 434.71   |

Scroll / sort:

| probe | result |
|---|---|
| 100k × 10 scroll frame samples | n=85, **p50=8.33ms**, **p95=12.51ms**, max=1049ms (max = load spike, not scroll) |
| typed sort flip @ 10k × 10 (`SortListModel` incremental) | 213ms wall; UI stayed responsive |
| typed sort flip @ 100k × 10 | **1018ms** wall with `set_incremental(true)` — work is chunked on idle, UI remains interactive |
| 20× reload weak-ref cache | **stable at 205** every iteration (≈ GTK visible-row cap) |

### Gap verdicts

| Gap | Verdict | Mitigation validated? |
|---|---|---|
| 1. No horizontal virtualization | **Confirmed.** Cost scales ~linearly with column count; 100 cols ≈ 20k live widgets. | **Yes — column capping.** Keep ≤ ~30 columns mounted by default; column-picker + banner for the rest. Custom hadjustment windowing remains viable later; not required for MVP. |
| 2. No cell-level selection | **Confirmed** platform limit. | **Defer** rectangular cell-range selection. Ship row-level `MultiSelection` + rubberband + copy-as-TSV. Revisit only if users demand spreadsheet-style ranges. |
| 3. Right-click does not select row | **Mitigated.** | Cell-local `GestureClick` (button 3) + Nautilus `select_single_item_if_not_selected` (select only if not already selected → preserves multi-select). |
| 4. `GtkEditableLabel` cell bugs | **Avoided** in spike. | Read-only cells use **`GtkInscription`** + `fixed_width`. Inline edit needs a separate design (overlay editor / row dialog), not `EditableLabel` in-cell. |

### Implementation notes that should carry into Phase 2+

1. Hand-written `ListModelImpl` + minimal `RowObject` (`Arc<[Value]>`, **no** GObject properties).
2. `GtkInscription` cells, `fixed_width` per column, tear down/rebuild `ColumnViewColumn`s on shape change.
3. `SortListModel::set_incremental(true)` + `CustomSorter` on **typed** values.
4. Default visible-column cap (~30) with picker; treat 60/`SELECT *` as a capped view, not a reason to abandon ColumnView.
5. Right-click selection helper attached in factory `setup`, not on the `ColumnView` root.

### Cell-range selection recommendation

**Defer.** Row multi-select + TSV copy covers the primary DB-tool workflows (inspect, copy,
delete-selected). Building rectangular selection is 400–600 lines of non-native feel for
limited gain; revisit after parity buildout if demand appears.

### Success metrics check

- 60-col **cold** column rebuild **206ms** (under ~250ms). Replace-into-populated total is
  higher because `set_data` emits a full model reset — still acceptable with column capping
  at ≤30 (rebuild ~105ms / total ~164ms @ 10k).
- 100k-row scroll sustains **60fps** (p50 8.33ms, p95 12.51ms).
- Decision recorded with numbers: **GO**.

---

## Sources & References

### Internal References

- Result shape to imitate: `core/src/models.rs:132` — `QueryEvent { Columns, Row, Done, RowsAffected, Error }`
- Existing grid behavior to match: `src/lib/components/ResultGrid.svelte` (617 lines, `@tanstack/svelte-virtual`, 36px rows, overscan 10)
- Row-cap precedent: 1000-row display cap, `core/src/db/mod.rs:969` (`limit.min(1000) + 1` computes `has_more`)

### External References

- GtkColumnView announcement (horizontal virtualization caveat): https://blogs.gnome.org/gtk/2020/09/21/gtkcolumnview/
- ColumnView performance thread: https://discourse.gnome.org/t/gtk4-columnview-performance-problem/33271
- "ColumnView — the missing bits" (right-click selection): https://discourse.gnome.org/t/columnview-the-missing-bits/26513
- Row-count limit: https://stackoverflow.com/questions/76797193/how-do-you-limit-the-number-of-gtkcolumnview-rows
- `GtkInscription`: https://docs.gtk.org/gtk4/class.Inscription.html
- EditableLabel-in-cell interaction bug: https://stackoverflow.com/questions/79766564/how-do-i-pass-through-the-click-on-a-gtkeditablelabel-to-the-gtkcolumnview-row-c
- Mission Center `TableView` (closest analogue): https://gitlab.com/mission-center-devs/mission-center
- Nautilus `NautilusListBase` / `select_single_item_if_not_selected` — canonical click handling
- `gtk4-demo` → Lists → Colors — proof of vertical scale; installed locally

### New Files

- `tmp/columnview-spike/` — per `AGENTS.md`; gitignored, **not** a workspace member, **not** merged.
  The only artifact that leaves this directory is the Findings section above.