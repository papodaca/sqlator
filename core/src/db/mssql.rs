use crate::error::CoreError;
use crate::models::{QueryEvent, SchemaColumnInfo, SchemaInfo, TableInfo};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

pub type MssqlClient = tiberius::Client<tokio_util::compat::Compat<TcpStream>>;
pub type MssqlPool = Arc<Mutex<MssqlClient>>;

pub async fn create_pool(url: &str) -> Result<MssqlPool, CoreError> {
    let config = parse_url(url)?;
    let addr = config.get_addr().to_string();
    let tcp = TcpStream::connect(&addr).await.map_err(|e| CoreError {
        message: format!("TCP connect to {}: {}", addr, e),
        code: "CONNECTION_FAILED".into(),
    })?;
    tcp.set_nodelay(true).ok();
    let client = tiberius::Client::connect(config, tcp.compat_write())
        .await
        .map_err(|e| CoreError {
            message: e.to_string(),
            code: "CONNECTION_FAILED".into(),
        })?;
    Ok(Arc::new(Mutex::new(client)))
}

fn parse_url(url: &str) -> Result<tiberius::Config, CoreError> {
    let parsed = url::Url::parse(url).map_err(|e| CoreError {
        message: format!("Invalid MSSQL URL: {}", e),
        code: "INVALID_URL".into(),
    })?;

    let mut config = tiberius::Config::new();

    if let Some(host) = parsed.host_str() {
        config.host(host);
    }
    config.port(parsed.port().unwrap_or(1433));

    let database = parsed.path().trim_start_matches('/');
    if !database.is_empty() {
        config.database(database);
    }

    if !parsed.username().is_empty() {
        let password = parsed.password().unwrap_or("");
        config.authentication(tiberius::AuthMethod::sql_server(parsed.username(), password));
    }

    // Trust server certificate by default — users can configure TLS later
    config.trust_cert();

    Ok(config)
}

pub async fn execute_select(
    pool: &MssqlPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    let mut client = pool.lock().await;
    let rows = match client.query(sql, &[]).await {
        Ok(q) => match q.into_first_result().await {
            Ok(rows) => rows,
            Err(e) => {
                let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
                return Ok(());
            }
        },
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
            return Ok(());
        }
    };

    let mut row_count: usize = 0;
    let mut columns_sent = false;
    let max_rows: usize = 1000;

    for row in &rows {
        if !columns_sent {
            let names: Vec<String> = row.columns().iter().map(|c| c.name().to_string()).collect();
            let _ = sender.send(QueryEvent::Columns { names }).await;
            columns_sent = true;
        }
        if row_count < max_rows {
            let values: Vec<serde_json::Value> = row
                .columns()
                .iter()
                .enumerate()
                .map(|(i, _)| mssql_value_to_json(row, i))
                .collect();
            let _ = sender.send(QueryEvent::Row { values }).await;
        }
        row_count += 1;
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let _ = sender.send(QueryEvent::Done { row_count, duration_ms }).await;
    Ok(())
}

pub async fn execute_statement(
    pool: &MssqlPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    let mut client = pool.lock().await;
    match client.execute(sql, &[]).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected {
                    count: result.rows_affected().iter().sum(),
                    duration_ms,
                })
                .await;
        }
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
        }
    }
    Ok(())
}

pub async fn get_schemas(pool: &MssqlPool) -> Result<Vec<SchemaInfo>, CoreError> {
    let mut client = pool.lock().await;
    let rows = client
        .query(
            "SELECT name FROM sys.schemas \
             WHERE name NOT IN ('sys','INFORMATION_SCHEMA','guest','db_owner',\
             'db_accessadmin','db_securityadmin','db_ddladmin','db_backupoperator',\
             'db_datareader','db_datawriter','db_denydatareader','db_denydatawriter') \
             ORDER BY name",
            &[],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
        .into_first_result()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: &str = r.try_get(0).ok().flatten().unwrap_or("unknown");
            SchemaInfo { is_default: name == "dbo", name: name.to_string() }
        })
        .collect())
}

pub async fn get_tables(
    pool: &MssqlPool,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, CoreError> {
    let schema = schema.unwrap_or("dbo");
    let mut client = pool.lock().await;
    let rows = client
        .query(
            "SELECT t.name, t.type_desc \
             FROM sys.tables t \
             JOIN sys.schemas s ON t.schema_id = s.schema_id \
             WHERE s.name = @P1 \
             UNION ALL \
             SELECT v.name, 'VIEW' \
             FROM sys.views v \
             JOIN sys.schemas s ON v.schema_id = s.schema_id \
             WHERE s.name = @P1 \
             ORDER BY name",
            &[&schema],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
        .into_first_result()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: &str = r.try_get(0).ok().flatten().unwrap_or("unknown");
            let type_desc: &str = r.try_get(1).ok().flatten().unwrap_or("USER_TABLE");
            let table_type = if type_desc.contains("VIEW") { "view" } else { "table" };
            TableInfo {
                full_name: format!("{}.{}", schema, name),
                name: name.to_string(),
                schema: Some(schema.to_string()),
                table_type: table_type.to_string(),
            }
        })
        .collect())
}

pub async fn get_columns(
    pool: &MssqlPool,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let schema = schema.unwrap_or("dbo");
    let mut client = pool.lock().await;

    // Fetch primary key columns for this table
    let pk_rows = client
        .query(
            "SELECT k.name \
             FROM sys.key_constraints kc \
             JOIN sys.index_columns ic ON kc.parent_object_id = ic.object_id \
                 AND kc.unique_index_id = ic.index_id \
             JOIN sys.columns k ON ic.object_id = k.object_id AND ic.column_id = k.column_id \
             JOIN sys.schemas s ON kc.schema_id = s.schema_id \
             WHERE kc.type = 'PK' AND kc.parent_object_id = OBJECT_ID(@P1) \
                 AND s.name = @P2",
            &[&format!("{}.{}", schema, table_name), &schema],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
        .into_first_result()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let pk_columns: std::collections::HashSet<String> = pk_rows
        .iter()
        .filter_map(|r| r.try_get::<&str, _>(0).ok().flatten().map(String::from))
        .collect();

    // Fetch foreign key columns
    let fk_rows = client
        .query(
            "SELECT \
                 c.name AS column_name, \
                 rs.name AS ref_schema, \
                 rt.name AS ref_table, \
                 rc.name AS ref_column \
             FROM sys.foreign_key_columns fkc \
             JOIN sys.foreign_keys fk ON fkc.constraint_object_id = fk.object_id \
             JOIN sys.columns c ON fkc.parent_object_id = c.object_id \
                 AND fkc.parent_column_id = c.column_id \
             JOIN sys.tables t ON c.object_id = t.object_id \
             JOIN sys.schemas s ON t.schema_id = s.schema_id \
             JOIN sys.tables rt ON fkc.referenced_object_id = rt.object_id \
             JOIN sys.schemas rs ON rt.schema_id = rs.schema_id \
             JOIN sys.columns rc ON fkc.referenced_object_id = rc.object_id \
                 AND fkc.referenced_column_id = rc.column_id \
             WHERE t.name = @P1 AND s.name = @P2",
            &[&table_name, &schema],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
        .into_first_result()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let mut fk_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for r in &fk_rows {
        if let (Some(col), Some(ref_schema), Some(ref_table), Some(ref_col)) = (
            r.try_get::<&str, _>(0).ok().flatten(),
            r.try_get::<&str, _>(1).ok().flatten(),
            r.try_get::<&str, _>(2).ok().flatten(),
            r.try_get::<&str, _>(3).ok().flatten(),
        ) {
            fk_map.insert(
                col.to_string(),
                (format!("{}.{}", ref_schema, ref_table), ref_col.to_string()),
            );
        }
    }

    // Fetch column metadata
    let col_rows = client
        .query(
            "SELECT c.name, tp.name AS data_type, c.is_nullable, \
                 dc.definition AS default_value \
             FROM sys.columns c \
             JOIN sys.types tp ON c.user_type_id = tp.user_type_id \
             JOIN sys.tables t ON c.object_id = t.object_id \
             JOIN sys.schemas s ON t.schema_id = s.schema_id \
             LEFT JOIN sys.default_constraints dc ON c.default_object_id = dc.object_id \
             WHERE t.name = @P1 AND s.name = @P2 \
             ORDER BY c.column_id",
            &[&table_name, &schema],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
        .into_first_result()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(col_rows
        .iter()
        .enumerate()
        .map(|(ordinal, r)| {
            let name: &str = r.try_get(0).ok().flatten().unwrap_or("unknown");
            let data_type: &str = r.try_get(1).ok().flatten().unwrap_or("unknown");
            let nullable: bool = r.try_get::<bool, _>(2).ok().flatten().unwrap_or(true);
            let default_value: Option<String> =
                r.try_get::<&str, _>(3).ok().flatten().map(String::from);
            let is_fk = fk_map.contains_key(name);
            let (foreign_table, foreign_column) = fk_map
                .get(name)
                .map(|(t, c)| (Some(t.clone()), Some(c.clone())))
                .unwrap_or((None, None));
            SchemaColumnInfo {
                name: name.to_string(),
                data_type: map_mssql_type(data_type),
                nullable,
                default_value,
                is_primary_key: pk_columns.contains(name),
                is_foreign_key: is_fk,
                foreign_table,
                foreign_column,
                ordinal_position: (ordinal + 1) as i32,
            }
        })
        .collect())
}

fn map_mssql_type(type_name: &str) -> String {
    match type_name.to_lowercase().as_str() {
        "int" => "integer",
        "bigint" => "bigint",
        "smallint" => "smallint",
        "tinyint" => "tinyint",
        "bit" => "boolean",
        "float" | "real" => "float",
        "decimal" | "numeric" | "money" | "smallmoney" => "decimal",
        "char" | "nchar" => "char",
        "varchar" | "nvarchar" | "text" | "ntext" => "text",
        "binary" | "varbinary" | "image" => "binary",
        "date" => "date",
        "time" => "time",
        "datetime" | "datetime2" | "smalldatetime" | "datetimeoffset" => "datetime",
        "uniqueidentifier" => "uuid",
        "xml" => "xml",
        "json" => "json",
        other => other,
    }
    .to_string()
}

fn mssql_value_to_json(row: &tiberius::Row, index: usize) -> serde_json::Value {
    // Each try_get returns Ok(Some(v)) for a match, Ok(None) for NULL, Err for type mismatch.
    // The first Ok(None) we see means the value is NULL.
    macro_rules! try_get {
        ($ty:ty) => {
            match row.try_get::<$ty, _>(index) {
                Ok(Some(v)) => return serde_json::json!(v),
                Ok(None) => return serde_json::Value::Null,
                Err(_) => {}
            }
        };
        ($ty:ty, $conv:expr) => {
            match row.try_get::<$ty, _>(index) {
                Ok(Some(v)) => return $conv(v),
                Ok(None) => return serde_json::Value::Null,
                Err(_) => {}
            }
        };
    }

    try_get!(bool);
    try_get!(i64);
    try_get!(i32);
    try_get!(i16);
    try_get!(u8);
    try_get!(f64);
    try_get!(f32);
    try_get!(chrono::NaiveDateTime, |v: chrono::NaiveDateTime| serde_json::json!(v.to_string()));
    try_get!(chrono::NaiveDate, |v: chrono::NaiveDate| serde_json::json!(v.to_string()));
    try_get!(chrono::NaiveTime, |v: chrono::NaiveTime| serde_json::json!(v.to_string()));
    try_get!(&str, |v: &str| serde_json::Value::String(v.to_string()));
    try_get!(&[u8], |v: &[u8]| serde_json::json!(format!("<binary: {} bytes>", v.len())));

    let type_name = format!("{:?}", row.columns()[index].column_type());
    serde_json::Value::String(format!("<{}>", type_name))
}
