---
title: "fix: Align Tauri table extraction with web, and close the implicit-join hole in both"
type: fix
status: active
date: 2026-07-25
---

# fix: Align Tauri table extraction with web, and close the implicit-join hole in both

## Overview

`extract_table_regex` — the fallback used to decide whether a query's results are editable —
exists in two copies that disagree in three ways. Tauri's copy is the worse one on all three.

But **adopting web's copy verbatim would introduce a data-safety bug**, because web's copy
marks an old-style comma join as editable. The correct fix is neither copy: it is a small
corrected implementation that takes web's permissiveness on the SELECT list and Tauri's
strictness on the FROM clause.

This plan also fixes a silent-wrong-output bug present in **both** copies.

---

## Problem Statement / Motivation

### Where this code sits

`fetch_schema_metadata` (`src-tauri/src/commands.rs:1617`) decides whether the results grid is
editable. It calls `extract_single_table` (`:1530`), which tries `sqlparser` first and falls
back to `extract_table_regex` (`:1583`) when parsing fails. If neither yields a single table,
the grid is returned read-only with the reason *"Cannot edit: query joins multiple tables or
uses a subquery"* (`:1642`).

The sqlparser branch is byte-for-byte equivalent between Tauri and web. **Only the regex
fallback diverges.**

### Measured behavior

Both implementations were extracted and run side by side against the same inputs. Results:

| Input | Tauri today | Web today | Correct |
|---|---|---|---|
| `SELECT a FROM t` | `Some(t)` | `Some(t)` | `Some(t)` |
| `SELECT a, b FROM t` | **`None`** | `Some(t)` | `Some(t)` |
| `SELECT f(a, b) FROM t` | **`None`** | `Some(t)` | `Some(t)` |
| `SELECT a FROM t WHERE x IN (1,2)` | **`None`** | `Some(t)` | `Some(t)` |
| `SELECT a, b FROM t1, t2` | `None` | `None` | `None` |
| `SELECT a, b FROM t1 , t2` | `None` | **`Some(t1)`** | `None` |
| ``SELECT a FROM `"t"` `` | **`Some("\"t\"")`** | `Some(t)` | `Some(t)` |
| `SELECT a FROM s.t` | `Some(t, s)` | `Some(t, s)` | `Some(t, s)` |

Three distinct defects, all in ~30 lines:

1. **Tauri rejects any comma anywhere in the statement** (`commands.rs:1601`:
   `upper2.contains(",")`). Since almost every real SELECT has a comma in its column list,
   this makes the entire fallback path return `None` in practice. Web checks only the table
   token (`handlers.rs:1141-1142`).
2. **Web accepts an implicit comma join when there is a space before the comma.**
   `SELECT a, b FROM t1 , t2` yields `Some("t1")` — the grid would offer to edit `t1` on the
   results of a cross join. Tauri's over-broad check accidentally prevents this. **This is the
   reason the fix cannot simply be "copy web".**
3. **Tauri's identifier unquoting is order-dependent.**
   `.trim_matches('"').trim_matches('`')` strips all double quotes, *then* all backticks — so a
   mixed-quoted `` `"t"` `` comes out as `"t"` with the double quotes still attached, and the
   subsequent catalog lookup fails. Web's single predicate pass handles it.

### A fourth defect, present in both

`from_idx` is computed on the uppercased string and then used to slice the **original**:

```rust
let upper = sql.to_uppercase();
let from_idx = upper.find(" FROM ")?;
let after_from = sql[from_idx + 6..].trim_start();   // byte offset from a different string
```

`to_uppercase()` is not length-preserving. Verified:

```
"SELECT 'ııı' FROM t"  ->  extracts table name "OM"
```

Three dotless-i characters (2 bytes each) uppercase to `I` (1 byte each), shifting every
subsequent offset by 3 bytes. The extracted "table name" is a fragment of the word `FROM`. The
user sees a read-only grid with a misleading reason. A `panic` is also reachable in principle,
when the shifted offset lands inside a multi-byte character rather than between two.

### How much does this actually matter?

Less than the defect count suggests, and the plan should be honest about that. The fallback is
only reached when `sqlparser` fails, and sqlparser 0.54's `GenericDialect` is extremely
permissive. Of 28 dialect-specific constructs tested — Postgres `->>`/`@>`/`~`/`$$…$$`/
`DISTINCT ON`/`FOR UPDATE SKIP LOCKED`, MySQL `LIMIT 1, 2`/`_binary`/`MATCH…AGAINST`,
MSSQL `OPTION (RECOMPILE)`/`WITH (NOLOCK)`/`N'…'`, ClickHouse `PREWHERE`/`ARRAY JOIN`/
`SETTINGS`, Oracle optimizer hints — **27 parsed successfully**. Only the `mysql` CLI's `\G`
terminator failed.

So this is a latent-landmine fix, not a firefight. It is worth doing now for three reasons:
the code is about to be moved into a shared crate where the wrong copy could win by accident;
the implicit-join hole is a data-safety issue whenever it *is* reached; and the whole thing is
about 30 lines.

---

## Proposed Solution

Replace both copies with one corrected implementation.

### Behavioral specification

1. **Locate `FROM` without a cross-string byte offset.** Scan the original bytes
   case-insensitively rather than searching an uppercased copy:

   ```rust
   let idx = sql.as_bytes()
       .windows(6)
       .position(|w| w.eq_ignore_ascii_case(b" from "))?;
   ```

   This yields an offset valid for `sql` itself, eliminating defect 4.

2. **Delimit the FROM clause.** Take the region from the end of `" FROM "` up to the first
   following clause keyword (`WHERE`, `GROUP`, `HAVING`, `ORDER`, `LIMIT`, `OFFSET`, `FETCH`,
   `WINDOW`, `UNION`, `INTERSECT`, `EXCEPT`, `FOR`, `INTO`) or a `;`, whichever comes first.

3. **Reject multi-table FROM clauses using that region only.** A comma *or* ` JOIN ` inside the
   FROM region means multiple tables — return `None`. A comma outside it (in the SELECT list, a
   function call, an `IN (…)` list) is irrelevant. This fixes defects 1 and 2 together: it is
   as permissive as web on the SELECT list and stricter than web on the FROM clause.

4. **Take the first whitespace-delimited token** of the FROM region as the table reference.

5. **Unquote with a single predicate pass**, `trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')`,
   fixing defect 3 and additionally handling MSSQL bracket quoting, which neither copy does today.

6. **Split `schema.table` on the first unquoted dot.** Both copies currently use
   `splitn(2, '.')` on the raw token, which mis-splits a quoted identifier containing a dot
   (`"my.schema".t`). Fixing this is optional; if deferred, record it as a known limitation
   rather than leaving it undocumented.

### Where the fixed copy lives

Two options, depending on sequencing against the
[service extraction](gtk/2026-07-25-002-refactor-sqlator-service-extraction-plan.md):

- **If this lands first (recommended):** fix `src-tauri/src/commands.rs` and
  `web-server/src/handlers.rs` identically, with the same tests in both. The extraction then
  moves one already-agreed implementation instead of adjudicating a divergence mid-refactor.
- **If the extraction lands first:** apply the fix once in `sqlator-service`.

Landing this first is preferable. It is small, independently verifiable, and it removes one
decision from a much larger refactor.

### Do not switch to dialect-specific parsers

An obvious-looking improvement is to select `PostgreSqlDialect` / `MySqlDialect` / etc. from
the connection's `db_type` instead of `GenericDialect`. The measurement above argues against
it: `GenericDialect` already accepts 27 of 28 dialect-specific constructs, and the specific
dialects are *stricter* — `PostgreSqlDialect` would reject MySQL backtick quoting that
`GenericDialect` accepts today. Switching would shrink coverage and push more queries into the
fallback, which is the opposite of the goal. Keep `GenericDialect`.

---

## Technical Considerations

- **Editability is a data-safety boundary.** A false negative is a mild annoyance (read-only
  grid). A false positive lets the user edit a grid whose rows do not map to one table, and the
  generated `UPDATE`/`DELETE` may hit the wrong rows. When in doubt the function must return
  `None`. The spec above is deliberately asymmetric for this reason.
- **`core` already provides a second safety net**: `is_editable` is additionally gated on a
  primary key existing (`core/src/db/mod.rs:1281`, `:1364`, `:1429`). The implicit-join hole is
  therefore only exploitable when the first table in the comma join happens to have a PK — which
  is the common case, so this does not defuse it.
- **The read-only reason string is wrong for fallback failures.** `commands.rs:1642` always says
  *"query joins multiple tables or uses a subquery"*, even when the real cause was an unparseable
  statement. Worth distinguishing: *"Cannot determine a single source table for this query"* is
  accurate for the fallback path and less confusing.
- **Keep the fallback.** With sqlparser handling 27/28 cases, deleting the fallback entirely is
  tempting. It still catches the `\G` case and any future sqlparser regression, and it is cheap
  once correct.

---

## System-Wide Impact

- **Interaction graph:** unchanged. One private function is replaced in two files.
- **Error propagation:** unchanged, except the read-only reason string becomes accurate for the
  fallback path.
- **State lifecycle risks:** the schema cache is keyed on the extracted table name
  (`commands.rs:1648`). A previously-wrong extraction (`"OM"`, or `"t"` with quotes) may be
  cached; entries expire after 5 minutes so no migration is needed.
- **API surface parity:** Tauri and web converge. Web loses the implicit-join hole; Tauri gains
  a working fallback. No frontend change required — `TableMeta` is unchanged.

---

## Acceptance Criteria

- [x] `extract_table_regex` is behaviorally identical in `commands.rs` and `handlers.rs`
- [x] Every row of the measured-behavior table produces the "Correct" column in both frontends
- [x] `SELECT a, b FROM t1 , t2` returns `None` (implicit-join hole closed in web)
- [x] `SELECT a, b FROM t` returns `Some(t)` (comma-in-SELECT-list unblocked in Tauri)
- [x] `` SELECT a FROM `"t"` `` returns `Some(t)` with no residual quote characters
- [x] `SELECT a FROM [dbo].[t]` returns `Some(t, dbo)` (MSSQL bracket quoting)
- [x] `"SELECT 'ııı' FROM t"` returns `Some(t)`, not `Some("OM")`
- [x] No input in the test corpus panics, including non-ASCII before `FROM`
- [x] Comma and `JOIN` detection consider only the FROM region, verified by a test with a comma
      in the SELECT list, in a function call, and in an `IN (…)` list
- [x] The fallback-path read-only reason no longer claims the query "joins multiple tables"
- [x] Tests live next to both copies and move with the code in the service extraction
- [x] Manual check: a MySQL query using `\G` (the one confirmed parse failure) yields a correctly
      editable grid

### Implementation notes (vs plan)

- Optional quoted-dot split (`"my.schema".t`) **was** implemented via `split_schema_table`.
- Table tokens also stop at `\` so `SELECT a FROM t\G` extracts `t` (needed for the `\G` case;
  neither original copy did this — they would have kept `t\G` as the token).

---

## Success Metrics

- One implementation, two call sites, zero divergence
- No query that is safely editable is marked read-only by the fallback
- No query that is unsafe to edit is marked editable

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| None | Self-contained; no new crates, no schema or API change |
| Sequences before [service extraction](gtk/2026-07-25-002-refactor-sqlator-service-extraction-plan.md) | Recommended, so the refactor moves an agreed implementation |
| Overlaps [characterization tests](gtk/2026-07-25-000-chore-characterization-tests-before-refactor-plan.md) | That plan pins current behavior including these bugs; if it lands first, its `extract_table_regex` tests are rewritten here with the ledger rows as justification |

| Risk | Mitigation |
|---|---|
| Making extraction more permissive enables an unsafe edit | The FROM-region rule is *stricter* than web on exactly the dimension that matters; PK gate remains as a second net |
| FROM-region delimiting misses a clause keyword | Keyword list in the spec; a token not in the list simply extends the region, which fails safe (more chance of finding a comma → `None`) |
| Divergence reappears while two copies exist | Identical tests in both files; the extraction collapses them shortly after |
| Effort spent on a rarely-reached path | Acknowledged explicitly above; justified by the imminent code move and the data-safety hole |

---

## Sources & References

### Internal References

- `src-tauri/src/commands.rs:1530` — `extract_single_table` (sqlparser branch; identical to web)
- `src-tauri/src/commands.rs:1583` — `extract_table_regex` (Tauri copy; defects 1, 3, 4)
- `src-tauri/src/commands.rs:1595` — table-token comma check (shared, correct)
- `src-tauri/src/commands.rs:1601` — `upper2.contains(",")`, defect 1
- `src-tauri/src/commands.rs:1608-1612` — sequential `trim_matches`, defect 3
- `src-tauri/src/commands.rs:1617` — `fetch_schema_metadata`, the only caller
- `src-tauri/src/commands.rs:1642` — the misleading read-only reason string
- `src-tauri/src/commands.rs:1648` — schema cache key
- `web-server/src/handlers.rs:1093` — `extract_single_table` (web copy)
- `web-server/src/handlers.rs:1133` — `extract_table_regex` (web copy; defects 2, 4)
- `web-server/src/handlers.rs:1142` — `" JOIN "`-only check, defect 2
- `core/src/db/mod.rs:1281`, `:1364`, `:1429` — `is_editable` gated on primary key
- `core/src/models.rs:237-238` — `TableMeta.is_editable` / `editability_reason`
- `src/lib/components/ResultGrid.svelte` — consumer; renders the read-only badge and reason
- `sqlparser = "0.54"` — `src-tauri/Cargo.toml:29`, `web-server/Cargo.toml:26`

### Measurements

Both implementations were extracted verbatim into a scratch crate and run against the input
table above; the parse-failure survey covered 28 dialect-specific constructs against
`GenericDialect` with 1 failure (`\G`). The non-ASCII offset bug was reproduced with
`"SELECT 'ııı' FROM t"` yielding table name `"OM"`.

### New Files

None. Modifies `src-tauri/src/commands.rs` and `web-server/src/handlers.rs`, plus inline
`#[cfg(test)] mod tests` in each.