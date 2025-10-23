//! TCP task database operations
//!
//! This module handles all database operations specific to TCP connection monitoring:
//! - Table creation and indexing
//! - Raw metric storage
//! - Aggregated metric generation and storage
//! - Loading aggregated metrics

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::config::TaskType;
use shared::metrics::{
    AggregatedMetricData, AggregatedMetrics, AggregatedTcpMetric, MetricData, RawTcpMetric,
};
use tracing::debug;

/// Create TCP-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_tcp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            connect_time_ms REAL,
            success BOOLEAN NOT NULL,
            error TEXT,
            host TEXT NOT NULL,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_tcp table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_tcp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            avg_connect_time_ms REAL NOT NULL,
            max_connect_time_ms REAL NOT NULL,
            min_connect_time_ms REAL NOT NULL,
            failure_percent REAL NOT NULL,
            successful_connections INTEGER NOT NULL,
            failed_connections INTEGER NOT NULL,
            host TEXT,
            target_id TEXT,
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_tcp table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_tcp_timestamp ON raw_metric_tcp(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_tcp_task ON raw_metric_tcp(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tcp_period ON agg_metric_tcp(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw TCP metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    tcp_data: &RawTcpMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_tcp (task_name, timestamp, connect_time_ms, success, error, host, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            tcp_data.connect_time_ms,
            tcp_data.success,
            tcp_data.error,
            tcp_data.host,
            tcp_data.target_id
        ],
    )?;
    debug!("Stored TCP metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated TCP metrics for a period
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
            AVG(CASE WHEN success = 1 AND connect_time_ms IS NOT NULL THEN connect_time_ms END) as avg_connect_time,
            MAX(CASE WHEN success = 1 AND connect_time_ms IS NOT NULL THEN connect_time_ms END) as max_connect_time,
            MIN(CASE WHEN success = 1 AND connect_time_ms IS NOT NULL THEN connect_time_ms END) as min_connect_time,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_connections,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_connections,
            (SELECT host FROM raw_metric_tcp
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             ORDER BY timestamp ASC
             LIMIT 1) as first_host,
            (SELECT target_id FROM raw_metric_tcp
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_tcp
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

            let successful_connections: i64 = row.get("successful_connections")?;
            let failed_connections: i64 = row.get("failed_connections")?;
            let failure_percent = if total_count > 0 {
                (failed_connections as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let host: String = row.get("first_host").unwrap_or_default();
            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedTcpMetric {
                avg_connect_time_ms: row.get("avg_connect_time").unwrap_or(0.0),
                max_connect_time_ms: row.get("max_connect_time").unwrap_or(0.0),
                min_connect_time_ms: row.get("min_connect_time").unwrap_or(0.0),
                failure_percent,
                successful_connections: successful_connections as u32,
                failed_connections: failed_connections as u32,
                host,
                target_id,
            }))
        },
    )?;

    if let Some(tcp_metric) = row {
        let total_samples = tcp_metric.successful_connections + tcp_metric.failed_connections;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::Tcp,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::Tcp(tcp_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated TCP metrics
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    tcp_data: &AggregatedTcpMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_tcp
        (task_name, period_start, period_end, sample_count, avg_connect_time_ms, max_connect_time_ms, min_connect_time_ms, failure_percent, successful_connections, failed_connections, host, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            tcp_data.avg_connect_time_ms,
            tcp_data.max_connect_time_ms,
            tcp_data.min_connect_time_ms,
            tcp_data.failure_percent,
            tcp_data.successful_connections,
            tcp_data.failed_connections,
            tcp_data.host,
            tcp_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated TCP metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                avg_connect_time_ms, min_connect_time_ms, max_connect_time_ms,
                failure_percent, successful_connections, failed_connections,
                host, target_id
         FROM agg_metric_tcp WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::Tcp,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::Tcp(AggregatedTcpMetric {
                avg_connect_time_ms: row.get(4)?,
                min_connect_time_ms: row.get(5)?,
                max_connect_time_ms: row.get(6)?,
                failure_percent: row.get(7)?,
                successful_connections: row.get(8)?,
                failed_connections: row.get(9)?,
                host: row.get(10).unwrap_or_default(),
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

/// Clean up old TCP metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_tcp WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_tcp
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'tcp' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
