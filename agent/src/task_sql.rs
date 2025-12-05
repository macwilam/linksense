//! SQL query task execution with Value and JSON modes

use anyhow::{Context, Result};
use shared::config::SqlQueryParams;
use shared::metrics::RawSqlQueryMetric;
use std::time::{Duration, Instant};

#[cfg(feature = "sql-tasks")]
use rsql_drivers::{DriverManager, Value};

#[cfg(feature = "sql-tasks")]
use shared::config::SqlQueryMode;

/// Result from Value mode execution
#[cfg(feature = "sql-tasks")]
struct ValueModeResult {
    row_count: u64,
    column_count: u32,
    value: Option<f64>,
}

/// Result from JSON mode execution
#[cfg(feature = "sql-tasks")]
struct JsonModeResult {
    row_count: u64,
    column_count: u32,
    json_result: String,
    truncated: bool,
}

/// Execute a SQL query task and return raw metrics
#[cfg(feature = "sql-tasks")]
pub async fn execute_sql_query(params: &SqlQueryParams) -> Result<RawSqlQueryMetric> {
    let start = Instant::now();
    let timeout = Duration::from_secs(params.timeout_seconds as u64);

    // Build connection URL based on database type and credentials
    let connection_url = build_connection_url(params);

    // Initialize driver manager and connect
    DriverManager::initialize().context("Failed to initialize driver manager")?;

    let mut connection = DriverManager::connect(&connection_url)
        .await
        .context("Failed to connect to database")?;

    // Execute based on mode with timeout
    let result = match params.mode {
        SqlQueryMode::Value => {
            let value_result =
                tokio::time::timeout(timeout, execute_value_mode(&mut *connection, &params.query))
                    .await;

            match value_result {
                Ok(Ok(r)) => Ok(ModeResult::Value(r)),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(anyhow::anyhow!(
                    "SQL query timeout after {}s",
                    params.timeout_seconds
                )),
            }
        }
        SqlQueryMode::Json => {
            let json_result = tokio::time::timeout(
                timeout,
                execute_json_mode(
                    &mut *connection,
                    &params.query,
                    params.max_rows,
                    params.max_json_size_bytes,
                ),
            )
            .await;

            match json_result {
                Ok(Ok(r)) => Ok(ModeResult::Json(r)),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(anyhow::anyhow!(
                    "SQL query timeout after {}s",
                    params.timeout_seconds
                )),
            }
        }
    };

    let elapsed = start.elapsed();
    let total_time_ms = elapsed.as_secs_f64() * 1000.0;

    // Build metric from result
    build_metric(result, total_time_ms, params)
}

#[cfg(feature = "sql-tasks")]
enum ModeResult {
    Value(ValueModeResult),
    Json(JsonModeResult),
}

#[cfg(feature = "sql-tasks")]
fn build_connection_url(params: &SqlQueryParams) -> String {
    if let (Some(username), Some(password)) = (&params.username, &params.password) {
        format!(
            "{}://{}:{}@{}",
            params.database_type, username, password, params.database_url
        )
    } else if let Some(username) = &params.username {
        format!(
            "{}://{}@{}",
            params.database_type, username, params.database_url
        )
    } else {
        format!("{}://{}", params.database_type, params.database_url)
    }
}

#[cfg(feature = "sql-tasks")]
fn build_metric(
    result: Result<ModeResult>,
    total_time_ms: f64,
    params: &SqlQueryParams,
) -> Result<RawSqlQueryMetric> {
    match result {
        Ok(ModeResult::Value(r)) => Ok(RawSqlQueryMetric {
            total_time_ms: Some(total_time_ms),
            row_count: Some(r.row_count),
            success: true,
            error: None,
            target_id: params.target_id.clone(),
            mode: params.mode.clone(),
            value: r.value,
            json_result: None,
            json_truncated: false,
            column_count: Some(r.column_count),
        }),
        Ok(ModeResult::Json(r)) => Ok(RawSqlQueryMetric {
            total_time_ms: Some(total_time_ms),
            row_count: Some(r.row_count),
            success: true,
            error: None,
            target_id: params.target_id.clone(),
            mode: params.mode.clone(),
            value: None,
            json_result: Some(r.json_result),
            json_truncated: r.truncated,
            column_count: Some(r.column_count),
        }),
        Err(e) => Ok(RawSqlQueryMetric {
            total_time_ms: Some(total_time_ms),
            row_count: None,
            success: false,
            error: Some(e.to_string()),
            target_id: params.target_id.clone(),
            mode: params.mode.clone(),
            value: None,
            json_result: None,
            json_truncated: false,
            column_count: None,
        }),
    }
}

/// Execute query in Value mode - extract first row, first column as numeric
#[cfg(feature = "sql-tasks")]
async fn execute_value_mode(
    connection: &mut dyn rsql_drivers::Connection,
    query: &str,
) -> Result<ValueModeResult> {
    let mut query_result = connection
        .query(query)
        .await
        .context("Failed to execute query")?;

    let columns = query_result.columns().await;
    let column_count = columns.len() as u32;

    // Get first row
    let first_row = query_result.next().await;

    // Count all rows
    let mut row_count: u64 = if first_row.is_some() { 1 } else { 0 };
    while query_result.next().await.is_some() {
        row_count += 1;
    }

    // Extract numeric value from first cell
    let value = first_row
        .as_ref()
        .and_then(|row| row.first())
        .and_then(|v| value_to_f64(v));

    Ok(ValueModeResult {
        row_count,
        column_count,
        value,
    })
}

/// Execute query in JSON mode - return results as JSON array of objects
#[cfg(feature = "sql-tasks")]
async fn execute_json_mode(
    connection: &mut dyn rsql_drivers::Connection,
    query: &str,
    max_rows: usize,
    max_json_size_bytes: usize,
) -> Result<JsonModeResult> {
    let mut query_result = connection
        .query(query)
        .await
        .context("Failed to execute query")?;

    let columns = query_result.columns().await;
    let column_count = columns.len() as u32;

    // Collect rows up to max_rows
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut total_row_count: u64 = 0;

    while let Some(row) = query_result.next().await {
        total_row_count += 1;
        if rows.len() < max_rows {
            rows.push(row);
        }
    }

    // Convert to JSON array of objects
    let json_rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            let obj: serde_json::Map<String, serde_json::Value> = columns
                .iter()
                .zip(row.into_iter())
                .map(|(col, val)| (col.clone(), value_to_json(val)))
                .collect();
            serde_json::Value::Object(obj)
        })
        .collect();

    let json_string = serde_json::to_string(&json_rows).context("Failed to serialize to JSON")?;

    // Check size limit and truncate if necessary
    let (final_json, truncated) = if json_string.len() > max_json_size_bytes {
        truncate_json_result(&json_rows, max_json_size_bytes)?
    } else {
        (json_string, false)
    };

    Ok(JsonModeResult {
        row_count: total_row_count,
        column_count,
        json_result: final_json,
        truncated,
    })
}

/// Convert rsql_drivers Value to f64 for Value mode
#[cfg(feature = "sql-tasks")]
fn value_to_f64(value: &Value) -> Option<f64> {
    use std::str::FromStr;
    match value {
        Value::I8(v) => Some(*v as f64),
        Value::I16(v) => Some(*v as f64),
        Value::I32(v) => Some(*v as f64),
        Value::I64(v) => Some(*v as f64),
        Value::I128(v) => Some(*v as f64),
        Value::U8(v) => Some(*v as f64),
        Value::U16(v) => Some(*v as f64),
        Value::U32(v) => Some(*v as f64),
        Value::U64(v) => Some(*v as f64),
        Value::U128(v) => Some(*v as f64),
        Value::F32(v) => Some(*v as f64),
        Value::F64(v) => Some(*v),
        Value::String(s) => s.parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Null => None,
        // Decimal - MySQL returns SUM/AVG as Decimal type
        Value::Decimal(d) => f64::from_str(&d.to_string()).ok(),
        // Date, Time, DateTime, Uuid, Bytes, Array, Map - not convertible to simple f64
        _ => None,
    }
}

/// Convert rsql_drivers Value to serde_json::Value for JSON mode
#[cfg(feature = "sql-tasks")]
fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::I8(v) => serde_json::json!(v),
        Value::I16(v) => serde_json::json!(v),
        Value::I32(v) => serde_json::json!(v),
        Value::I64(v) => serde_json::json!(v),
        Value::I128(v) => serde_json::Value::String(v.to_string()),
        Value::U8(v) => serde_json::json!(v),
        Value::U16(v) => serde_json::json!(v),
        Value::U32(v) => serde_json::json!(v),
        Value::U64(v) => serde_json::json!(v),
        Value::U128(v) => serde_json::Value::String(v.to_string()),
        Value::F32(v) => serde_json::json!(v),
        Value::F64(v) => serde_json::json!(v),
        Value::String(s) => serde_json::Value::String(s),
        Value::Bytes(b) => {
            // Encode bytes as base64
            use base64::Engine;
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(&b))
        }
        Value::Date(d) => serde_json::Value::String(d.to_string()),
        Value::Time(t) => serde_json::Value::String(t.to_string()),
        Value::DateTime(dt) => serde_json::Value::String(dt.to_string()),
        Value::Uuid(u) => serde_json::Value::String(u.to_string()),
        Value::Decimal(d) => serde_json::Value::String(d.to_string()),
        Value::Array(arr) => serde_json::Value::Array(arr.into_iter().map(value_to_json).collect()),
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .map(|(k, v)| {
                    // Convert key to string representation
                    let key_str = match k {
                        Value::String(s) => s,
                        other => format!("{:?}", other),
                    };
                    (key_str, value_to_json(v))
                })
                .collect();
            serde_json::Value::Object(obj)
        }
    }
}

/// Truncate JSON result by removing rows until under size limit
#[cfg(feature = "sql-tasks")]
fn truncate_json_result(rows: &[serde_json::Value], max_size: usize) -> Result<(String, bool)> {
    // Binary search for the right number of rows
    let mut low = 0;
    let mut high = rows.len();

    while low < high {
        let mid = (low + high + 1) / 2;
        let subset: Vec<&serde_json::Value> = rows.iter().take(mid).collect();
        let json = serde_json::to_string(&subset)?;
        if json.len() <= max_size {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    // Generate final result
    if low == 0 {
        // Even one row is too big, return empty array
        Ok(("[]".to_string(), true))
    } else {
        let subset: Vec<&serde_json::Value> = rows.iter().take(low).collect();
        let json = serde_json::to_string(&subset)?;
        Ok((json, low < rows.len()))
    }
}

#[cfg(not(feature = "sql-tasks"))]
pub async fn execute_sql_query(_params: &SqlQueryParams) -> Result<RawSqlQueryMetric> {
    anyhow::bail!("SQL tasks feature is not enabled. Compile with --features sql-tasks")
}
