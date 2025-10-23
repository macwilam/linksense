//! Bandwidth test task database operations

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{
        AggregatedBandwidthMetric, AggregatedMetricData, AggregatedMetrics, MetricData,
        RawBandwidthMetric,
    },
};
use tracing::debug;

/// Create bandwidth-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_bandwidth (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            bandwidth_mbps REAL,
            duration_ms REAL,
            bytes_downloaded INTEGER,
            success BOOLEAN NOT NULL,
            error TEXT,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_bandwidth table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_bandwidth (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            avg_bandwidth_mbps REAL NOT NULL,
            max_bandwidth_mbps REAL NOT NULL,
            min_bandwidth_mbps REAL NOT NULL,
            successful_tests INTEGER NOT NULL,
            failed_tests INTEGER NOT NULL,
            target_id TEXT,
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_bandwidth table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_bandwidth_timestamp ON raw_metric_bandwidth(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_bandwidth_task ON raw_metric_bandwidth(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_bandwidth_period ON agg_metric_bandwidth(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw bandwidth metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    bandwidth_data: &RawBandwidthMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_bandwidth (task_name, timestamp, bandwidth_mbps, duration_ms, bytes_downloaded, success, error, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            bandwidth_data.bandwidth_mbps,
            bandwidth_data.duration_ms,
            bandwidth_data.bytes_downloaded.map(|b| b as i64),
            bandwidth_data.success,
            bandwidth_data.error,
            bandwidth_data.target_id
        ],
    )?;
    debug!("Stored bandwidth metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated bandwidth metrics for a time period
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
            AVG(CASE WHEN success = 1 AND bandwidth_mbps IS NOT NULL THEN bandwidth_mbps END) as avg_bandwidth,
            MAX(CASE WHEN success = 1 AND bandwidth_mbps IS NOT NULL THEN bandwidth_mbps END) as max_bandwidth,
            MIN(CASE WHEN success = 1 AND bandwidth_mbps IS NOT NULL THEN bandwidth_mbps END) as min_bandwidth,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_tests,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_tests,
            (SELECT target_id FROM raw_metric_bandwidth
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_bandwidth
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

            let successful_tests: i64 = row.get("successful_tests")?;
            let failed_tests: i64 = row.get("failed_tests")?;
            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedBandwidthMetric {
                avg_bandwidth_mbps: row.get("avg_bandwidth").unwrap_or(0.0),
                max_bandwidth_mbps: row.get("max_bandwidth").unwrap_or(0.0),
                min_bandwidth_mbps: row.get("min_bandwidth").unwrap_or(0.0),
                successful_tests: successful_tests as u32,
                failed_tests: failed_tests as u32,
                target_id,
            }))
        },
    )?;

    if let Some(bandwidth_metric) = row {
        let total_samples = bandwidth_metric.successful_tests + bandwidth_metric.failed_tests;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::Bandwidth,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::Bandwidth(bandwidth_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated bandwidth metric
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    bandwidth_data: &AggregatedBandwidthMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_bandwidth
        (task_name, period_start, period_end, sample_count, avg_bandwidth_mbps, max_bandwidth_mbps, min_bandwidth_mbps, successful_tests, failed_tests, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            bandwidth_data.avg_bandwidth_mbps,
            bandwidth_data.max_bandwidth_mbps,
            bandwidth_data.min_bandwidth_mbps,
            bandwidth_data.successful_tests,
            bandwidth_data.failed_tests,
            bandwidth_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated bandwidth metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                avg_bandwidth_mbps, max_bandwidth_mbps, min_bandwidth_mbps,
                successful_tests, failed_tests, target_id
         FROM agg_metric_bandwidth WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::Bandwidth,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::Bandwidth(AggregatedBandwidthMetric {
                avg_bandwidth_mbps: row.get(4)?,
                max_bandwidth_mbps: row.get(5)?,
                min_bandwidth_mbps: row.get(6)?,
                successful_tests: row.get(7)?,
                failed_tests: row.get(8)?,
                target_id: row.get(9).ok(),
            }),
        })
    });

    match result {
        Ok(metric) => Ok(Some(metric)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Clean up old bandwidth metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_bandwidth WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_bandwidth
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'bandwidth' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
