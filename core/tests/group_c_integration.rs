//! Group C characterization tests (plan 000).
//!
//! Pin current behavior of load-bearing DB/SSH paths before the service extraction.
//!
//! Gating (either is enough to run; otherwise each test returns early):
//! - Cargo feature: `cargo test -p sqlator-core --features integration`
//! - Env var: `SQLATOR_INTEGRATION=1 cargo test -p sqlator-core`
//!
//! Bare `cargo test --workspace` must pass with no docker (tests skip).

use sqlator_core::models::{
    ParameterizedStatement, QueryEvent, SqlBatch, TableQueryParams,
};
use sqlator_core::{DbManager, SshAuthConfig, SshHostConfig, SshTunnel};
use std::time::Duration;

const MYSQL_URL: &str = "mysql://sqlator:sqlator@localhost:3336/sqlator";
const POSTGRES_URL: &str = "postgresql://sqlator:sqlator@localhost:5454/sqlator";
const MSSQL_URL: &str = "mssql://sa:Sqlator123!@localhost:1444/master";
const ORACLE_URL: &str = "oracle://system:Sqlator123!@localhost:1522/FREEPDB1";
const CLICKHOUSE_URL: &str = "clickhouse://sqlator:sqlator@localhost:8123/sqlator";

const SSH_HOST: &str = "127.0.0.1";
const SSH_PORT: u16 = 2222;
const SSH_USER: &str = "sqlator";
const SSH_PASSWORD: &str = "sqlator";
/// Compose-network hostname for postgres as seen from the openssh service.
const TUNNEL_TARGET_HOST: &str = "postgres";
const TUNNEL_TARGET_PORT: u16 = 5432;

fn integration_enabled() -> bool {
    if cfg!(feature = "integration") {
        return true;
    }
    matches!(
        std::env::var("SQLATOR_INTEGRATION").ok().as_deref(),
        Some("1") | Some("true")
    )
}

fn skip(reason: &str) {
    eprintln!("skipping Group C test: {reason}");
}

fn sqlite_url(path: &std::path::Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

async fn collect_events(mgr: &DbManager, id: &str, sql: &str) -> Vec<QueryEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QueryEvent>(64);
    mgr.execute_query(id, sql, tx)
        .await
        .unwrap_or_else(|e| panic!("execute_query failed: {e:?}"));
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

async fn mysql_reachable() -> bool {
    DbManager::test_connection(MYSQL_URL).await.is_ok()
}

async fn postgres_reachable() -> bool {
    DbManager::test_connection(POSTGRES_URL).await.is_ok()
}

async fn ssh_port_open() -> bool {
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((SSH_HOST, SSH_PORT)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

// ── 1. MySQL VARBINARY workaround ─────────────────────────────────────────────

/// `get_schemas` against MySQL 8 must decode SCHEMA_NAME as String.
/// Production uses `CAST(SCHEMA_NAME AS CHAR)` (core/src/db/mod.rs) because
/// MySQL 8 returns information_schema strings as VARBINARY via prepared stmts.
#[tokio::test]
async fn mysql_get_schemas_decodes_schema_name_as_string() {
    if !integration_enabled() {
        skip("set SQLATOR_INTEGRATION=1 or --features integration");
        return;
    }
    if !mysql_reachable().await {
        skip("MySQL not reachable at localhost:3336");
        return;
    }

    let mgr = DbManager::new();
    mgr.connect("mysql-schemas", MYSQL_URL)
        .await
        .expect("connect mysql");

    let schemas = mgr
        .get_schemas("mysql-schemas")
        .await
        .expect("get_schemas must succeed with CAST AS CHAR workaround");

    assert!(
        !schemas.is_empty(),
        "expected at least the sqlator schema; got {schemas:?}"
    );
    for s in &schemas {
        assert!(
            !s.name.is_empty(),
            "schema name must decode as non-empty String"
        );
        // Names are printable identifiers, not opaque binary
        assert!(
            s.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "unexpected schema name bytes: {:?}",
            s.name
        );
    }
    assert!(
        schemas.iter().any(|s| s.name == "sqlator"),
        "expected schema 'sqlator' in {schemas:?}"
    );

    mgr.disconnect("mysql-schemas").await;
}

// ── 2. Pagination has_more ────────────────────────────────────────────────────

#[tokio::test]
async fn query_table_has_more_clamping_sqlite() {
    if !integration_enabled() {
        skip("set SQLATOR_INTEGRATION=1 or --features integration");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let url = sqlite_url(&dir.path().join("has_more.db"));
    let mgr = DbManager::new();
    mgr.connect("hm", &url).await.expect("connect");

    collect_events(
        &mgr,
        "hm",
        "CREATE TABLE items (id INTEGER PRIMARY KEY, n INTEGER NOT NULL)",
    )
    .await;

    // Seed 15 rows
    for i in 1..=15 {
        collect_events(
            &mgr,
            "hm",
            &format!("INSERT INTO items (id, n) VALUES ({i}, {i})"),
        )
        .await;
    }

    let base = |limit: i64| TableQueryParams {
        connection_id: "hm".into(),
        table_name: "items".into(),
        schema: None,
        sort: vec![],
        filters: vec![],
        limit,
        offset: 0,
    };

    // Exactly `limit` rows available → has_more false
    let r = mgr.query_table("hm", &base(15)).await.expect("limit=15");
    assert_eq!(r.total_returned, 15);
    assert!(!r.has_more, "exactly limit rows → has_more false; got {r:?}");

    // limit+1 available → has_more true, returns `limit` rows
    let r = mgr.query_table("hm", &base(10)).await.expect("limit=10");
    assert_eq!(r.total_returned, 10);
    assert!(r.has_more, "limit+1 available → has_more true; got {r:?}");

    // Request limit > 1000 is clamped: fetch uses min(limit,1000)+1
    for i in 16..=1005 {
        collect_events(
            &mgr,
            "hm",
            &format!("INSERT INTO items (id, n) VALUES ({i}, {i})"),
        )
        .await;
    }
    let r = mgr.query_table("hm", &base(2000)).await.expect("limit=2000");
    assert_eq!(
        r.total_returned, 1000,
        "limit>1000 clamped to 1000 rows returned"
    );
    assert!(
        r.has_more,
        "1005 rows with clamped limit 1000 → has_more true"
    );

    mgr.disconnect("hm").await;
}

// ── 3. QueryEvent sequencing ──────────────────────────────────────────────────

#[tokio::test]
async fn query_event_sequencing_select_nonsel_failure() {
    if !integration_enabled() {
        skip("set SQLATOR_INTEGRATION=1 or --features integration");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let url = sqlite_url(&dir.path().join("events.db"));
    let mgr = DbManager::new();
    mgr.connect("ev", &url).await.expect("connect");

    collect_events(&mgr, "ev", "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)").await;
    collect_events(&mgr, "ev", "INSERT INTO t (id, v) VALUES (1, 'a')").await;
    collect_events(&mgr, "ev", "INSERT INTO t (id, v) VALUES (2, 'b')").await;

    // SELECT: Columns → Row* → Done
    let events = collect_events(&mgr, "ev", "SELECT id, v FROM t ORDER BY id").await;
    assert!(
        matches!(events.first(), Some(QueryEvent::Columns { .. })),
        "SELECT must start with Columns; got {events:?}"
    );
    let row_count = events
        .iter()
        .filter(|e| matches!(e, QueryEvent::Row { .. }))
        .count();
    assert_eq!(row_count, 2, "expected 2 Row events; got {events:?}");
    assert!(
        matches!(events.last(), Some(QueryEvent::Done { row_count: 2, .. })),
        "SELECT must end with Done; got {events:?}"
    );
    assert_eq!(events.len(), 4, "Columns + 2 Rows + Done; got {events:?}");

    // non-SELECT: RowsAffected (and nothing else)
    let events = collect_events(&mgr, "ev", "UPDATE t SET v = 'x' WHERE id = 1").await;
    assert_eq!(events.len(), 1, "non-SELECT → single event; got {events:?}");
    assert!(
        matches!(events[0], QueryEvent::RowsAffected { count: 1, .. }),
        "expected RowsAffected; got {events:?}"
    );

    // failure: Error and nothing after
    let events = collect_events(&mgr, "ev", "SELECT * FROM definitely_missing").await;
    assert_eq!(events.len(), 1, "failure → only Error; got {events:?}");
    assert!(
        matches!(events[0], QueryEvent::Error { .. }),
        "expected Error; got {events:?}"
    );

    let events = collect_events(&mgr, "ev", "DELETE FROM definitely_missing").await;
    assert_eq!(events.len(), 1, "non-SELECT failure → only Error; got {events:?}");
    assert!(
        matches!(events[0], QueryEvent::Error { .. }),
        "expected Error; got {events:?}"
    );

    mgr.disconnect("ev").await;
}

// ── 4. execute_batch transactionality ─────────────────────────────────────────

fn stmt(sql: &str, params: Vec<serde_json::Value>) -> ParameterizedStatement {
    ParameterizedStatement {
        sql: sql.into(),
        params,
        temp_id: None,
    }
}

#[tokio::test]
async fn execute_batch_ordering_and_rollback_sqlite() {
    if !integration_enabled() {
        skip("set SQLATOR_INTEGRATION=1 or --features integration");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let url = sqlite_url(&dir.path().join("batch.db"));
    let mgr = DbManager::new();
    mgr.connect("batch", &url).await.expect("connect");

    collect_events(
        &mgr,
        "batch",
        "CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)",
    )
    .await;
    collect_events(&mgr, "batch", "INSERT INTO kv (k, v) VALUES ('a', '1')").await;
    collect_events(&mgr, "batch", "INSERT INTO kv (k, v) VALUES ('b', '2')").await;

    // DELETE → UPDATE → INSERT ordering (characterization of statement order)
    let batch = SqlBatch {
        statements: vec![
            stmt("DELETE FROM kv WHERE k = ?", vec![serde_json::json!("a")]),
            stmt(
                "UPDATE kv SET v = ? WHERE k = ?",
                vec![serde_json::json!("2b"), serde_json::json!("b")],
            ),
            stmt(
                "INSERT INTO kv (k, v) VALUES (?, ?)",
                vec![serde_json::json!("c"), serde_json::json!("3")],
            ),
        ],
        use_transaction: true,
    };
    let result = mgr
        .execute_batch("batch", &batch)
        .await
        .expect("batch ok");
    assert!(result.success, "batch should succeed: {result:?}");
    assert_eq!(result.executed_count, 3);

    let events = collect_events(&mgr, "batch", "SELECT k, v FROM kv ORDER BY k").await;
    let rows: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Row { values } => Some(values.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("b"), serde_json::json!("2b")],
            vec![serde_json::json!("c"), serde_json::json!("3")],
        ],
        "DELETE→UPDATE→INSERT order must leave b=2b and c=3; events={events:?}"
    );

    // Mid-batch failure rolls back (current code always begins a transaction)
    let batch = SqlBatch {
        statements: vec![
            stmt(
                "INSERT INTO kv (k, v) VALUES (?, ?)",
                vec![serde_json::json!("d"), serde_json::json!("4")],
            ),
            stmt("INSERT INTO kv (k, v) VALUES (?, ?)", vec![
                serde_json::json!("b"), // PK conflict
                serde_json::json!("nope"),
            ]),
            stmt(
                "INSERT INTO kv (k, v) VALUES (?, ?)",
                vec![serde_json::json!("e"), serde_json::json!("5")],
            ),
        ],
        use_transaction: true,
    };
    let result = mgr
        .execute_batch("batch", &batch)
        .await
        .expect("batch returns Ok with success=false on stmt error");
    assert!(!result.success, "mid-batch failure → success=false: {result:?}");
    assert_eq!(result.executed_count, 1);
    assert!(result.error.is_some());

    let events = collect_events(&mgr, "batch", "SELECT k FROM kv ORDER BY k").await;
    let keys: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Row { values } => values.first().cloned(),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        vec![serde_json::json!("b"), serde_json::json!("c")],
        "rollback must drop the mid-batch insert of 'd'; keys={keys:?}"
    );

    mgr.disconnect("batch").await;
}

#[tokio::test]
async fn execute_batch_unsupported_engines() {
    if !integration_enabled() {
        skip("set SQLATOR_INTEGRATION=1 or --features integration");
        return;
    }

    // Characterization: Mssql / Oracle / ClickHouse return CoreError code UNSUPPORTED.
    // (Plan text says "four"; production match has three arms — pin those three.)
    let cases = [
        ("mssql", MSSQL_URL),
        ("oracle", ORACLE_URL),
        ("clickhouse", CLICKHOUSE_URL),
    ];

    let mgr = DbManager::new();
    let mut exercised = 0usize;

    for (id, url) in cases {
        match tokio::time::timeout(Duration::from_secs(5), mgr.connect(id, url)).await {
            Ok(Ok(())) => {
                let err = mgr
                    .execute_batch(
                        id,
                        &SqlBatch {
                            statements: vec![stmt("SELECT 1", vec![])],
                            use_transaction: true,
                        },
                    )
                    .await
                    .expect_err("unsupported engine must Err, not Ok");
                assert_eq!(
                    err.code, "UNSUPPORTED",
                    "{id} batch must be UNSUPPORTED; got {err:?}"
                );
                mgr.disconnect(id).await;
                exercised += 1;
            }
            Ok(Err(e)) => {
                eprintln!("skipping unsupported-engine case {id}: connect failed: {e:?}");
            }
            Err(_) => {
                eprintln!("skipping unsupported-engine case {id}: connect timed out");
            }
        }
    }

    if exercised == 0 {
        skip("no mssql/oracle/clickhouse reachable — soft-skip UNSUPPORTED cases");
    }
}

// ── 5. SSH tunnel concurrency (deadlock regression) ───────────────────────────

#[tokio::test]
async fn ssh_tunnel_two_simultaneous_streams_complete() {
    if !integration_enabled() {
        skip("set SQLATOR_INTEGRATION=1 or --features integration");
        return;
    }
    if !ssh_port_open().await {
        skip("OpenSSH not reachable at localhost:2222 — start compose service `openssh`");
        return;
    }
    if !postgres_reachable().await {
        skip("Postgres not reachable at localhost:5454 (needed to verify tunnel target)");
        return;
    }

    let auth = SshAuthConfig::with_password(SSH_USER, SSH_PASSWORD);
    let host = SshHostConfig::new(SSH_HOST, SSH_PORT, &auth);

    let tunnel = SshTunnel::create(
        "group-c-concurrency".into(),
        &host,
        &auth,
        TUNNEL_TARGET_HOST.into(),
        TUNNEL_TARGET_PORT,
        &[],
    )
    .await
    .expect("SSH tunnel create");

    SshTunnel::start_forwarding(&tunnel)
        .await
        .expect("start_forwarding");

    // Brief settle for listener task
    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!(
        "postgresql://sqlator:sqlator@127.0.0.1:{}/sqlator",
        tunnel.local_port
    );

    // Two simultaneous streams through one tunnel — both must complete.
    // The old Arc<Mutex<Channel>> design deadlocked on bidirectional traffic;
    // postgres startup is a classic client-first protocol that triggers it.
    let run = async {
        let (a, b) = tokio::join!(
            async {
                let mgr = DbManager::new();
                mgr.connect("t1", &url).await.expect("connect t1");
                let events = collect_events(&mgr, "t1", "SELECT 1 AS n").await;
                mgr.disconnect("t1").await;
                events
            },
            async {
                let mgr = DbManager::new();
                mgr.connect("t2", &url).await.expect("connect t2");
                let events = collect_events(&mgr, "t2", "SELECT 2 AS n").await;
                mgr.disconnect("t2").await;
                events
            },
        );
        (a, b)
    };

    let (events_a, events_b) = tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("tunnel concurrency timed out — possible deadlock regression");

    assert!(
        events_a.iter().any(|e| matches!(e, QueryEvent::Done { .. })),
        "stream A must complete with Done; got {events_a:?}"
    );
    assert!(
        events_b.iter().any(|e| matches!(e, QueryEvent::Done { .. })),
        "stream B must complete with Done; got {events_b:?}"
    );
    assert!(
        events_a.iter().all(|e| !matches!(e, QueryEvent::Error { .. })),
        "stream A must not Error; got {events_a:?}"
    );
    assert!(
        events_b.iter().all(|e| !matches!(e, QueryEvent::Error { .. })),
        "stream B must not Error; got {events_b:?}"
    );

    let _ = SshTunnel::close(tunnel).await;
}