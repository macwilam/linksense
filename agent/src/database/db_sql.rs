//! SQL query task database operations

#[cfg(feature = "sql-tasks")]
use anyhow::{Context, Result};
#[cfg(feature = "sql-tasks")]
use rusqlite::{params, Connection};
#[cfg(feature = "sql-tasks")]
use shared::{
    config::TaskType,
    metrics::{
        AggregatedMetricData, AggregatedMetrics, AggregatedSqlQueryMetric, MetricData,
        RawSqlQueryMetric,
    },
};
#[cfg(feature = "sql-tasks")]
use tracing::debug;

/// Create SQL query-specific tables and indexes
#[cfg(feature = "sql-tasks")]
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_sql_query (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            total_time_ms REAL,
            row_count INTEGER,
            success BOOLEAN NOT NULL,
            error TEXT,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_sql_query table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_sql_query (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_total_time_ms REAL NOT NULL,
            max_total_time_ms REAL NOT NULL,
            avg_row_count REAL NOT NULL,
            max_row_count INTEGER NOT NULL,
            successful_queries INTEGER NOT NULL,
            failed_queries INTEGER NOT NULL,
            target_id TEXT,
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_sql_query table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_sql_query_timestamp ON raw_metric_sql_query(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_sql_query_task ON raw_metric_sql_query(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_sql_query_period ON agg_metric_sql_query(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw SQL query metric
#[cfg(feature = "sql-tasks")]
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    sql_data: &RawSqlQueryMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_sql_query (task_name, timestamp, total_time_ms, row_count, success, error, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            sql_data.total_time_ms,
            sql_data.row_count.map(|c| c as i64),
            sql_data.success,
            sql_data.error,
            sql_data.target_id
        ],
    )?;
    debug!("Stored SQL query metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated SQL query metrics for a time period
#[cfg(feature = "sql-tasks")]
pub(super) fn generate_aggregated_metrics(
    conn: &Connection,
    task_name: &str,
    period_start: u64,
    period_end: u64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            COUNT(*) as total_count,
            AVG(CASE WHEN success = 1 AND total_time_ms IS NOT NULL THEN total_time_ms END) as avg_total_time,
            MAX(CASE WHEN success = 1 AND total_time_ms IS NOT NULL THEN total_time_ms END) as max_total_time,
            AVG(CASE WHEN success = 1 AND row_count IS NOT NULL THEN row_count END) as avg_row_count,
            MAX(CASE WHEN success = 1 AND row_count IS NOT NULL THEN row_count END) as max_row_count,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_queries,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_queries,
            (SELECT target_id FROM raw_metric_sql_query
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_sql_query
        WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
        "#,
    )?;

    let row = stmt.query_row(
        params![task_name, period_start as i64, period_end as i64],
        |row| {
            let total_count: i64 = row.get("total_count")?;
            if total_count == 0 {
                return Ok(None);
            }

            let successful_queries: i64 = row.get("successful_queries")?;
            let failed_queries: i64 = row.get("failed_queries")?;
            let success_rate_percent = if total_count > 0 {
                (successful_queries as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedSqlQueryMetric {
                success_rate_percent,
                avg_total_time_ms: row.get("avg_total_time").unwrap_or(0.0),
                max_total_time_ms: row.get("max_total_time").unwrap_or(0.0),
                avg_row_count: row.get("avg_row_count").unwrap_or(0.0),
                max_row_count: row.get("max_row_count").unwrap_or(0),
                successful_queries: successful_queries as u32,
                failed_queries: failed_queries as u32,
                target_id,
            }))
        },
    )?;

    if let Some(sql_metric) = row {
        let total_samples = sql_metric.successful_queries + sql_metric.failed_queries;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::SqlQuery,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::SqlQuery(sql_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated SQL query metric
#[cfg(feature = "sql-tasks")]
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    sql_data: &AggregatedSqlQueryMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_sql_query
        (task_name, period_start, period_end, sample_count, success_rate_percent, avg_total_time_ms, max_total_time_ms, avg_row_count, max_row_count, successful_queries, failed_queries, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            sql_data.success_rate_percent,
            sql_data.avg_total_time_ms,
            sql_data.max_total_time_ms,
            sql_data.avg_row_count,
            sql_data.max_row_count as i64,
            sql_data.successful_queries,
            sql_data.failed_queries,
            sql_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated SQL query metric by row ID
#[cfg(feature = "sql-tasks")]
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                success_rate_percent, avg_total_time_ms, max_total_time_ms,
                avg_row_count, max_row_count, successful_queries,
                failed_queries, target_id
         FROM agg_metric_sql_query WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::SqlQuery,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::SqlQuery(AggregatedSqlQueryMetric {
                success_rate_percent: row.get(4)?,
                avg_total_time_ms: row.get(5)?,
                max_total_time_ms: row.get(6)?,
                avg_row_count: row.get(7)?,
                max_row_count: row.get(8)?,
                successful_queries: row.get(9)?,
                failed_queries: row.get(10)?,
                target_id: row.get(11).ok(),
            }),
        })
    });

    match result {
        Ok(metric) => Ok(Some(metric)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Clean up old SQL query metrics
#[cfg(feature = "sql-tasks")]
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_sql_query WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_sql_query
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'sql_query' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}

/// Stub implementation when sql-tasks feature is disabled
#[cfg(not(feature = "sql-tasks"))]
pub(super) fn create_tables(_conn: &rusqlite::Connection) -> anyhow::Result<()> {
    Ok(())
}
