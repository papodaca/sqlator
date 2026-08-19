//! Fail-hard live wrap/passthrough proofs against Compose engines + SQLite file DB.
//!
//! Gating matches Group C (`--features integration` or `SQLATOR_INTEGRATION=1`).
//! Once enabled, a down required engine is a test failure (not a skip).
//!
//! Seed files: `tests/fixtures/engines/*.sql`. Compose mounts them for first boot
//! of empty volumes; tests also apply them so existing named volumes still work.

use sqlator_core::db::PagedQueryOutcome;
use sqlator_core::models::{QueryEvent, SortSpec};
use sqlator_core::DbManager;

const POSTGRES_URL: &str = "postgresql://sqlator:sqlator@localhost:5454/sqlator";
const MYSQL_URL: &str = "mysql://sqlator:sqlator@localhost:3336/sqlator";
const MARIADB_URL: &str = "mysql://sqlator:sqlator@localhost:3337/sqlator";
const MSSQL_URL: &str = "mssql://sa:Sqlator123!@localhost:1444/master";
const ORACLE_URL: &str = "oracle://system:Sqlator123!@localhost:1522/FREEPDB1";
const CLICKHOUSE_URL: &str = "clickhouse://sqlator:sqlator@localhost:8123/sqlator";

const PG_SEED: &str = include_str!("fixtures/engines/postgres.sql");
const MYSQL_SEED: &str = include_str!("fixtures/engines/mysql.sql");
const MARIADB_SEED: &str = include_str!("fixtures/engines/mariadb.sql");
const MSSQL_SEED: &str = include_str!("fixtures/engines/mssql.sql");
const ORACLE_SEED: &str = include_str!("fixtures/engines/oracle.sql");
const CLICKHOUSE_SEED: &str = include_str!("fixtures/engines/clickhouse.sql");
const SQLITE_SEED: &str = include_str!("fixtures/engines/sqlite.sql");

#[derive(Clone, Copy)]
struct Engine {
    name: &'static str,
    url: &'static str,
    seed: &'static str,
    /// Top-level WITH is passthrough on MSSQL (wrapper nesting).
    wrap_top_level_with: bool,
}

const COMPOSE_ENGINES: &[Engine] = &[
    Engine {
        name: "postgres",
        url: POSTGRES_URL,
        seed: PG_SEED,
        wrap_top_level_with: true,
    },
    Engine {
        name: "mysql",
        url: MYSQL_URL,
        seed: MYSQL_SEED,
        wrap_top_level_with: true,
    },
    Engine {
        name: "mariadb",
        url: MARIADB_URL,
        seed: MARIADB_SEED,
        wrap_top_level_with: true,
    },
    Engine {
        name: "mssql",
        url: MSSQL_URL,
        seed: MSSQL_SEED,
        wrap_top_level_with: false,
    },
    Engine {
        name: "oracle",
        url: ORACLE_URL,
        seed: ORACLE_SEED,
        wrap_top_level_with: true,
    },
    Engine {
        name: "clickhouse",
        url: CLICKHOUSE_URL,
        seed: CLICKHOUSE_SEED,
        wrap_top_level_with: true,
    },
];

fn integration_enabled() -> bool {
    if cfg!(feature = "integration") {
        return true;
    }
    matches!(
        std::env::var("SQLATOR_INTEGRATION").ok().as_deref(),
        Some("1") | Some("true")
    )
}

fn sqlite_url(path: &std::path::Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

async fn collect_events(mgr: &DbManager, id: &str, sql: &str) -> Vec<QueryEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QueryEvent>(64);
    mgr.execute_query(id, sql, tx)
        .await
        .unwrap_or_else(|e| panic!("execute_query failed ({id}): {e:?}"));
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

async fn collect_paged(
    mgr: &DbManager,
    id: &str,
    sql: &str,
    sort: &[SortSpec],
    sortable_columns: &[String],
    limit: usize,
    offset: usize,
) -> (PagedQueryOutcome, Vec<QueryEvent>) {
    // Capacity 1024 > 500-row page + Columns/Done so the driver never blocks.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QueryEvent>(1024);
    let outcome = mgr
        .execute_query_paged(id, sql, sort, sortable_columns, limit, offset, tx)
        .await
        .unwrap_or_else(|e| panic!("execute_query_paged failed ({id}): {e:?}"));
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    (outcome, events)
}

fn without_durations(events: &[QueryEvent]) -> Vec<serde_json::Value> {
    events
        .iter()
        .map(|e| {
            let mut v = serde_json::to_value(e).expect("serialize event");
            if let Some(data) = v.get_mut("data").and_then(|d| d.as_object_mut()) {
                data.remove("duration_ms");
            }
            v
        })
        .collect()
}

fn json_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|n| n as i64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn ids_from(events: &[QueryEvent]) -> Vec<i64> {
    events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Row { values } => values.first().and_then(json_i64),
            _ => None,
        })
        .collect()
}

fn collapse_sql(sql: &str) -> String {
    sql.lines()
        .map(|line| line.split("--").next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn seed_batches(engine: &str, sql: &str) -> Vec<String> {
    if engine == "oracle" {
        // Strip `--` comments and newlines: oracle-rs can close the connection
        // on multiline CREATE/INSERT (temporary LOB bind).
        return collapse_sql(sql)
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
    }
    if engine == "mssql" {
        let mut batches = Vec::new();
        let mut current = String::new();
        for line in sql.lines() {
            if line.trim().eq_ignore_ascii_case("go") {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    batches.push(trimmed.to_string());
                }
                current.clear();
            } else {
                current.push_str(line);
                current.push('\n');
            }
        }
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            batches.push(trimmed.to_string());
        }
        return batches;
    }
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| {
            s.lines()
                .any(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
        })
        .map(|s| s.to_string())
        .collect()
}

fn events_failed(events: &[QueryEvent]) -> Option<String> {
    events.iter().find_map(|e| match e {
        QueryEvent::Error { message, .. } => Some(message.clone()),
        _ => None,
    })
}

fn create_table_name(sql: &str) -> Option<&str> {
    let mut parts = sql.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("CREATE") {
        return None;
    }
    if !parts.next()?.eq_ignore_ascii_case("TABLE") {
        return None;
    }
    Some(parts.next()?)
}

async fn oracle_user_table_exists(mgr: &DbManager, id: &str, table: &str) -> bool {
    let sql = format!(
        "SELECT COUNT(*) FROM user_tables WHERE table_name = '{}'",
        table.to_uppercase()
    );
    let events = collect_events(mgr, id, &sql).await;
    events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Row { values } => values.first().and_then(json_i64),
            _ => None,
        })
        .unwrap_or(0)
        > 0
}

async fn apply_seed(mgr: &DbManager, id: &str, engine: &str, sql: &str) {
    for batch in seed_batches(engine, sql) {
        if engine == "oracle" {
            if let Some(name) = create_table_name(&batch) {
                if oracle_user_table_exists(mgr, id, name).await {
                    continue;
                }
            }
        }
        let events = collect_events(mgr, id, &batch).await;
        if let Some(msg) = events_failed(&events) {
            if engine == "oracle"
                && (msg.contains("ORA-00955")
                    || msg.contains("name is already used")
                    || msg.contains("ORA-02264")
                    || (msg.contains("ORA-00000") && create_table_name(&batch).is_some()))
            {
                continue;
            }
            panic!("seed apply failed on {id} ({engine}): {msg}\nSQL:\n{batch}");
        }
    }
}

async fn count_paged_items(mgr: &DbManager, id: &str) -> i64 {
    let events = collect_events(mgr, id, "SELECT COUNT(*) FROM sqlator_paged_items").await;
    if let Some(msg) = events_failed(&events) {
        panic!("{id}: COUNT(*) on paged_items failed: {msg}");
    }
    events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Row { values } => values.first().and_then(json_i64),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{id}: COUNT(*) returned no row; events={events:?}"))
}

async fn connect_fail_hard(mgr: &DbManager, id: &str, url: &str) {
    DbManager::test_connection(url)
        .await
        .unwrap_or_else(|e| panic!("{id} must be up at {url}: {e:?}"));
    mgr.connect(id, url)
        .await
        .unwrap_or_else(|e| panic!("connect {id} ({url}): {e:?}"));
}

async fn ready_engine(mgr: &DbManager, engine: &Engine) {
    let id = engine.name;
    connect_fail_hard(mgr, id, engine.url).await;
    apply_seed(mgr, id, engine.name, engine.seed).await;
    let n = count_paged_items(mgr, id).await;
    assert_eq!(
        n, 1250,
        "{id}: expected 1250 paged_items after seed, got {n}"
    );
}

async fn ready_sqlite(mgr: &DbManager, id: &str, path: &std::path::Path) {
    let url = sqlite_url(path);
    mgr.connect(id, &url).await.expect("connect sqlite");
    apply_seed(mgr, id, "sqlite", SQLITE_SEED).await;
    let n = count_paged_items(mgr, id).await;
    assert_eq!(n, 1250, "sqlite seed must leave 1250 paged_items, got {n}");
}

async fn assert_page_shape(
    mgr: &DbManager,
    id: &str,
    sql: &str,
    sort: &[SortSpec],
    sortable: &[String],
    offset: usize,
    expect: (usize, bool),
) -> Vec<QueryEvent> {
    let (expect_rows, expect_has_more) = expect;
    let (outcome, events) = collect_paged(mgr, id, sql, sort, sortable, 500, offset).await;
    if let Some(msg) = events_failed(&events) {
        panic!("{id} wrapper rejected at offset {offset}: {msg}\n{events:?}");
    }
    assert!(outcome.paged, "{id}: SELECT must run through the wrapper");
    assert_eq!(outcome.row_count, expect_rows, "{id} offset {offset}");
    assert_eq!(outcome.has_more, expect_has_more, "{id} offset {offset}");
    assert!(!outcome.capped, "{id}: small result must not be capped");
    let row_events = events
        .iter()
        .filter(|e| matches!(e, QueryEvent::Row { .. }))
        .count();
    assert_eq!(
        row_events, expect_rows,
        "{id} offset {offset}; got {events:?}"
    );
    assert!(
        matches!(events.last(), Some(QueryEvent::Done { .. })),
        "{id} offset {offset} must end with Done; got {events:?}"
    );
    events
}

#[test]
fn oracle_seed_batches_are_single_line_without_comments() {
    let batches = seed_batches("oracle", ORACLE_SEED);
    assert!(!batches.is_empty());
    for batch in &batches {
        assert!(!batch.contains('\n'), "{batch}");
        assert!(!batch.contains("--"), "{batch}");
        let head = batch.split_whitespace().next().unwrap_or("");
        assert!(
            head.eq_ignore_ascii_case("CREATE") || head.eq_ignore_ascii_case("INSERT"),
            "{batch}"
        );
    }
}

// ── U1 / U2: seed + fail-hard connect ─────────────────────────────────────────

#[tokio::test]
async fn sqlite_seed_file_creates_1250_paged_items_and_is_idempotent() {
    if !integration_enabled() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("seed.db");
    let mgr = DbManager::new();
    ready_sqlite(&mgr, "sqlite-seed", &path).await;
    apply_seed(&mgr, "sqlite-seed", "sqlite", SQLITE_SEED).await;
    assert_eq!(count_paged_items(&mgr, "sqlite-seed").await, 1250);
    mgr.disconnect("sqlite-seed").await;
}

#[tokio::test]
async fn postgres_must_be_up() {
    if !integration_enabled() {
        return;
    }

    let mgr = DbManager::new();
    connect_fail_hard(&mgr, "postgres", POSTGRES_URL).await;
    apply_seed(&mgr, "postgres", "postgres", PG_SEED).await;
    assert_eq!(count_paged_items(&mgr, "postgres").await, 1250);
    mgr.disconnect("postgres").await;
}

// ── U3: wrap matrix ───────────────────────────────────────────────────────────

const PLAIN_SELECT: &str = "SELECT id, n FROM sqlator_paged_items";
const WITH_SELECT: &str = "WITH q AS (SELECT id, n FROM sqlator_paged_items) SELECT id, n FROM q";
const EMPTY_SELECT: &str = "SELECT id, n FROM sqlator_paged_items WHERE 1 = 0";

async fn wrap_pages_and_sort(mgr: &DbManager, id: &str) {
    let sql = PLAIN_SELECT;
    assert_page_shape(mgr, id, sql, &[], &[], 0, (500, true)).await;
    assert_page_shape(mgr, id, sql, &[], &[], 500, (500, true)).await;
    assert_page_shape(mgr, id, sql, &[], &[], 1000, (250, false)).await;

    let sort = [SortSpec {
        column: "id".into(),
        desc: true,
    }];
    let sortable = ["id".to_string(), "n".to_string()];
    let mut seen: Vec<i64> = Vec::new();
    for offset in [0usize, 500, 1000] {
        let events = assert_page_shape(
            mgr,
            id,
            sql,
            &sort,
            &sortable,
            offset,
            (if offset < 1000 { 500 } else { 250 }, offset < 1000),
        )
        .await;
        seen.extend(ids_from(&events));
    }
    let expected: Vec<i64> = (1..=1250).rev().collect();
    assert_eq!(seen, expected, "{id}: sort id DESC must hold across pages");

    let (outcome, events) = collect_paged(mgr, id, EMPTY_SELECT, &[], &[], 500, 0).await;
    if let Some(msg) = events_failed(&events) {
        panic!("{id} empty filter failed: {msg}");
    }
    assert!(outcome.paged, "{id} empty filter still wraps");
    assert_eq!(outcome.row_count, 0);
    assert!(!outcome.has_more);
}

#[tokio::test]
async fn paged_wrap_sqlite_from_seed_file() {
    if !integration_enabled() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let mgr = DbManager::new();
    ready_sqlite(&mgr, "sqlite-wrap", &dir.path().join("paged.db")).await;
    wrap_pages_and_sort(&mgr, "sqlite-wrap").await;
    let (outcome, events) = collect_paged(&mgr, "sqlite-wrap", WITH_SELECT, &[], &[], 500, 0).await;
    assert!(events_failed(&events).is_none(), "{events:?}");
    assert!(outcome.paged);
    assert_eq!(outcome.row_count, 500);
    mgr.disconnect("sqlite-wrap").await;
}

#[tokio::test]
async fn paged_wrap_compose_engines() {
    if !integration_enabled() {
        return;
    }

    let mgr = DbManager::new();
    for engine in COMPOSE_ENGINES {
        ready_engine(&mgr, engine).await;
        wrap_pages_and_sort(&mgr, engine.name).await;
        if engine.wrap_top_level_with {
            let (outcome, events) =
                collect_paged(&mgr, engine.name, WITH_SELECT, &[], &[], 500, 0).await;
            if let Some(msg) = events_failed(&events) {
                panic!("{} WITH wrap failed: {msg}", engine.name);
            }
            assert!(outcome.paged, "{} WITH must wrap", engine.name);
            assert_eq!(outcome.row_count, 500, "{}", engine.name);
        }
        mgr.disconnect(engine.name).await;
    }
}

// ── U4: passthrough ───────────────────────────────────────────────────────────

#[tokio::test]
async fn paged_passthrough_dml_and_errors_sqlite() {
    if !integration_enabled() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let mgr = DbManager::new();
    ready_sqlite(&mgr, "sqlite-pt", &dir.path().join("pt.db")).await;

    let dml = "UPDATE sqlator_persons SET last_name = last_name WHERE person_id = 1";
    let today = collect_events(&mgr, "sqlite-pt", dml).await;
    let (outcome, paged_events) = collect_paged(&mgr, "sqlite-pt", dml, &[], &[], 500, 0).await;
    assert!(!outcome.paged, "DML must passthrough");
    assert_eq!(without_durations(&today), without_durations(&paged_events));

    let multi = "SELECT 1; SELECT 2";
    let (outcome, events) = collect_paged(&mgr, "sqlite-pt", multi, &[], &[], 500, 0).await;
    assert!(!outcome.paged, "multi-statement must not wrap");
    assert!(
        events_failed(&events).is_some()
            || events.iter().any(|e| matches!(e, QueryEvent::Row { .. })),
        "multi-statement passthrough must error or return rows, not a silent wrap; {events:?}"
    );

    let bad = "SELEC definitely not sql";
    let (outcome, events) = collect_paged(&mgr, "sqlite-pt", bad, &[], &[], 500, 0).await;
    assert!(!outcome.paged);
    assert!(
        events_failed(&events).is_some()
            || matches!(
                collect_events(&mgr, "sqlite-pt", bad).await.first(),
                Some(QueryEvent::Error { .. })
            ),
        "invalid SQL passthrough must surface Error, not paged=true; {events:?}"
    );

    mgr.disconnect("sqlite-pt").await;
}

#[tokio::test]
async fn paged_passthrough_mssql_top_level_with_and_dml() {
    if !integration_enabled() {
        return;
    }

    let mgr = DbManager::new();
    let engine = COMPOSE_ENGINES
        .iter()
        .find(|e| e.name == "mssql")
        .expect("mssql spec");
    ready_engine(&mgr, engine).await;

    let (outcome, events) = collect_paged(&mgr, "mssql", WITH_SELECT, &[], &[], 500, 0).await;
    if let Some(msg) = events_failed(&events) {
        panic!("MSSQL top-level WITH passthrough errored: {msg}");
    }
    assert!(!outcome.paged, "MSSQL WITH must passthrough (AE2)");
    let rows = events
        .iter()
        .filter(|e| matches!(e, QueryEvent::Row { .. }))
        .count();
    assert!(
        rows >= 1,
        "MSSQL WITH passthrough must still return rows; got {events:?}"
    );

    let dml = "UPDATE sqlator_persons SET last_name = last_name WHERE person_id = 1";
    let (outcome, events) = collect_paged(&mgr, "mssql", dml, &[], &[], 500, 0).await;
    if let Some(msg) = events_failed(&events) {
        panic!("MSSQL DML passthrough errored: {msg}");
    }
    assert!(!outcome.paged, "DML must passthrough");

    mgr.disconnect("mssql").await;
}
