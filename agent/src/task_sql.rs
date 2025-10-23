use anyhow::{Context, Result};
use shared::config::SqlQueryParams;
use shared::metrics::RawSqlQueryMetric;
use std::time::{Duration, Instant};

#[cfg(feature = "sql-tasks")]
use rsql_drivers::DriverManager;

/// Execute a SQL query task and return raw metrics
#[cfg(feature = "sql-tasks")]
pub async fn execute_sql_query(params: &SqlQueryParams) -> Result<RawSqlQueryMetric> {
    let start = Instant::now();
    let timeout = Duration::from_secs(params.timeout_seconds as u64);

    // Build connection URL based on database type and credentials
    let connection_url =
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
        };

    // Initialize driver manager and connect
    DriverManager::initialize().context("Failed to initialize driver manager")?;

    let mut connection = DriverManager::connect(&connection_url)
        .await
        .context("Failed to connect to database")?;

    // Apply timeout to query execution
    let query_result = tokio::time::timeout(timeout, connection.execute(&params.query)).await;

    let elapsed = start.elapsed();
    let total_time_ms = elapsed.as_secs_f64() * 1000.0;

    match query_result {
        Ok(Ok(row_count)) => Ok(RawSqlQueryMetric {
            total_time_ms: Some(total_time_ms),
            row_count: Some(row_count),
            success: true,
            error: None,
            target_id: params.target_id.clone(),
        }),
        Ok(Err(e)) => Ok(RawSqlQueryMetric {
            total_time_ms: Some(total_time_ms),
            row_count: None,
            success: false,
            error: Some(e.to_string()),
            target_id: params.target_id.clone(),
        }),
        Err(_) => Ok(RawSqlQueryMetric {
            total_time_ms: Some(total_time_ms),
            row_count: None,
            success: false,
            error: Some(format!(
                "SQL query timeout after {}s",
                params.timeout_seconds
            )),
            target_id: params.target_id.clone(),
        }),
    }
}

#[cfg(not(feature = "sql-tasks"))]
pub async fn execute_sql_query(_params: &SqlQueryParams) -> Result<RawSqlQueryMetric> {
    anyhow::bail!("SQL tasks feature is not enabled. Compile with --features sql-tasks")
}
