//! Ping task database operations
//!
//! This module handles all database operations specific to ICMP ping monitoring:
//! - Table creation and indexing
//! - Raw metric storage
//! - Aggregated metric generation and storage
//! - Loading aggregated metrics

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::config::TaskType;
use shared::metrics::{
    AggregatedMetricData, AggregatedMetrics, AggregatedPingMetric, MetricData, RawPingMetric,
};
use tracing::debug;

/// Create ping-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_ping (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            rtt_ms REAL,
            success BOOLEAN NOT NULL,
            error TEXT,
            ip_address TEXT NOT NULL,
            domain TEXT,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_ping table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_ping (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            avg_latency_ms REAL NOT NULL,
            max_latency_ms REAL NOT NULL,
            min_latency_ms REAL NOT NULL,
            packet_loss_percent REAL NOT NULL,
            successful_pings INTEGER NOT NULL,
            failed_pings INTEGER NOT NULL,
            domain TEXT,
            target_id TEXT,
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_ping table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_ping_timestamp ON raw_metric_ping(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_ping_task ON raw_metric_ping(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_ping_period ON agg_metric_ping(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw ping metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    ping_data: &RawPingMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_ping (task_name, timestamp, rtt_ms, success, error, ip_address, domain, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            ping_data.rtt_ms,
            ping_data.success,
            ping_data.error,
            ping_data.ip_address,
            ping_data.domain,
            ping_data.target_id
        ],
    )?;
    debug!("Stored ping metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated ping metrics for a period
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
            AVG(CASE WHEN success = 1 AND rtt_ms IS NOT NULL THEN rtt_ms END) as avg_rtt,
            MAX(CASE WHEN success = 1 AND rtt_ms IS NOT NULL THEN rtt_ms END) as max_rtt,
            MIN(CASE WHEN success = 1 AND rtt_ms IS NOT NULL THEN rtt_ms END) as min_rtt,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_pings,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_pings,
            (SELECT domain FROM raw_metric_ping
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND domain IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_domain,
            (SELECT target_id FROM raw_metric_ping
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_ping
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

            let successful_pings: i64 = row.get("successful_pings")?;
            let failed_pings: i64 = row.get("failed_pings")?;
            let packet_loss_percent = if total_count > 0 {
                (failed_pings as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let domain: Option<String> = row.get("first_domain").ok();
            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedPingMetric {
                avg_latency_ms: row.get("avg_rtt").unwrap_or(0.0),
                max_latency_ms: row.get("max_rtt").unwrap_or(0.0),
                min_latency_ms: row.get("min_rtt").unwrap_or(0.0),
                packet_loss_percent,
                successful_pings: successful_pings as u32,
                failed_pings: failed_pings as u32,
                domain,
                target_id,
            }))
        },
    )?;

    if let Some(ping_metric) = row {
        let total_samples = ping_metric.successful_pings + ping_metric.failed_pings;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::Ping,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::Ping(ping_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated ping metrics
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    ping_data: &AggregatedPingMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_ping
        (task_name, period_start, period_end, sample_count, avg_latency_ms, max_latency_ms, min_latency_ms, packet_loss_percent, successful_pings, failed_pings, domain, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            ping_data.avg_latency_ms,
            ping_data.max_latency_ms,
            ping_data.min_latency_ms,
            ping_data.packet_loss_percent,
            ping_data.successful_pings,
            ping_data.failed_pings,
            ping_data.domain,
            ping_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated ping metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                avg_latency_ms, min_latency_ms, max_latency_ms,
                packet_loss_percent, successful_pings, failed_pings,
                domain, target_id
         FROM agg_metric_ping WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::Ping,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::Ping(AggregatedPingMetric {
                avg_latency_ms: row.get(4)?,
                min_latency_ms: row.get(5)?,
                max_latency_ms: row.get(6)?,
                packet_loss_percent: row.get(7)?,
                successful_pings: row.get(8)?,
                failed_pings: row.get(9)?,
                domain: row.get(10).ok(),
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

/// Clean up old ping metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_ping WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_ping
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'ping' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
