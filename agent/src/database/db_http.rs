//! HTTP GET task database operations
//!
//! This module handles all database operations specific to HTTP GET monitoring:
//! - Table creation and indexing
//! - Raw metric storage
//! - Aggregated metric generation and storage
//! - Loading aggregated metrics

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::config::TaskType;
use shared::metrics::{
    AggregatedHttpMetric, AggregatedMetricData, AggregatedMetrics, MetricData, RawHttpMetric,
};
use std::collections::HashMap;
use tracing::debug;

/// Create HTTP-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_http (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            status_code INTEGER,
            tcp_timing_ms REAL,
            tls_timing_ms REAL,
            ttfb_timing_ms REAL,
            content_download_timing_ms REAL,
            total_time_ms REAL,
            success BOOLEAN NOT NULL,
            error TEXT,
            ssl_valid BOOLEAN,
            ssl_cert_days_until_expiry INTEGER,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_http table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_http (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_tcp_timing_ms REAL NOT NULL,
            avg_tls_timing_ms REAL,
            avg_ttfb_timing_ms REAL NOT NULL,
            avg_content_download_timing_ms REAL NOT NULL,
            avg_total_time_ms REAL NOT NULL,
            max_total_time_ms REAL NOT NULL,
            successful_requests INTEGER NOT NULL,
            failed_requests INTEGER NOT NULL,
            status_code_distribution TEXT NOT NULL,
            ssl_valid_percent REAL,
            avg_ssl_cert_days_until_expiry REAL,
            target_id TEXT,
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_http table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_http_timestamp ON raw_metric_http(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_http_task ON raw_metric_http(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_period ON agg_metric_http(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw HTTP metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    http_data: &RawHttpMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_http (task_name, timestamp, status_code, tcp_timing_ms, tls_timing_ms, ttfb_timing_ms, content_download_timing_ms, total_time_ms, success, error, ssl_valid, ssl_cert_days_until_expiry, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            http_data.status_code.map(|s| s as i64),
            http_data.tcp_timing_ms,
            http_data.tls_timing_ms,
            http_data.ttfb_timing_ms,
            http_data.content_download_timing_ms,
            http_data.total_time_ms,
            http_data.success,
            http_data.error,
            http_data.ssl_valid,
            http_data.ssl_cert_days_until_expiry,
            http_data.target_id
        ],
    )?;
    debug!("Stored HTTP metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated HTTP metrics for a period
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
            AVG(CASE WHEN success = 1 AND tcp_timing_ms IS NOT NULL THEN tcp_timing_ms END) as avg_tcp,
            AVG(CASE WHEN success = 1 AND tls_timing_ms IS NOT NULL THEN tls_timing_ms END) as avg_tls,
            AVG(CASE WHEN success = 1 AND ttfb_timing_ms IS NOT NULL THEN ttfb_timing_ms END) as avg_ttfb,
            AVG(CASE WHEN success = 1 AND content_download_timing_ms IS NOT NULL THEN content_download_timing_ms END) as avg_content_download,
            AVG(CASE WHEN success = 1 AND total_time_ms IS NOT NULL THEN total_time_ms END) as avg_total,
            MAX(CASE WHEN success = 1 AND total_time_ms IS NOT NULL THEN total_time_ms END) as max_total,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_requests,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_requests,
            AVG(CASE WHEN ssl_valid = 1 THEN 1.0 WHEN ssl_valid = 0 THEN 0.0 END) * 100.0 as ssl_valid_percent,
            AVG(CASE WHEN ssl_cert_days_until_expiry IS NOT NULL THEN ssl_cert_days_until_expiry END) as avg_ssl_cert_days_until_expiry,
            (SELECT target_id FROM raw_metric_http
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_http
        WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
        "#,
    )?;

    let mut status_code_stmt = conn.prepare(
        r#"
        SELECT status_code, COUNT(*) as count
        FROM raw_metric_http
        WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3 AND status_code IS NOT NULL
        GROUP BY status_code
        "#,
    )?;

    let mut status_code_distribution: HashMap<u16, u32> = HashMap::new();
    let rows = status_code_stmt.query_map(
        params![task_name, period_start as i64, period_end as i64],
        |row| {
            let code: i64 = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((code as u16, count as u32))
        },
    )?;

    for row in rows.flatten() {
        status_code_distribution.insert(row.0, row.1);
    }

    let row = stmt.query_row(
        params![task_name, period_start as i64, period_end as i64],
        |row| {
            let total_count: i64 = row.get("total_count")?;
            if total_count == 0 {
                return Ok(None);
            }

            let successful_requests: i64 = row.get("successful_requests")?;
            let failed_requests: i64 = row.get("failed_requests")?;
            let success_rate_percent = if total_count > 0 {
                (successful_requests as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedHttpMetric {
                success_rate_percent,
                avg_tcp_timing_ms: row.get("avg_tcp").unwrap_or(0.0),
                avg_tls_timing_ms: row.get("avg_tls").unwrap_or(0.0),
                avg_ttfb_timing_ms: row.get("avg_ttfb").unwrap_or(0.0),
                avg_content_download_timing_ms: row.get("avg_content_download").unwrap_or(0.0),
                avg_total_time_ms: row.get("avg_total").unwrap_or(0.0),
                max_total_time_ms: row.get("max_total").unwrap_or(0.0),
                successful_requests: successful_requests as u32,
                failed_requests: failed_requests as u32,
                status_code_distribution: status_code_distribution.clone(),
                ssl_valid_percent: row.get("ssl_valid_percent").ok(),
                avg_ssl_cert_days_until_expiry: row.get("avg_ssl_cert_days_until_expiry").ok(),
                target_id,
            }))
        },
    )?;

    if let Some(http_metric) = row {
        let total_samples = http_metric.successful_requests + http_metric.failed_requests;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::HttpGet,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::HttpGet(http_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated HTTP metrics
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    http_data: &AggregatedHttpMetric,
) -> Result<i64> {
    // Convert HashMap to Vec of tuples for proper JSON serialization
    // JSON only supports string keys, so we serialize as array of [code, count] pairs
    let status_code_vec: Vec<(u16, u32)> = http_data
        .status_code_distribution
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    let status_code_json = serde_json::to_string(&status_code_vec)?;
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_http
        (task_name, period_start, period_end, sample_count, success_rate_percent, avg_tcp_timing_ms, avg_tls_timing_ms, avg_ttfb_timing_ms, avg_content_download_timing_ms, avg_total_time_ms, max_total_time_ms, successful_requests, failed_requests, status_code_distribution, ssl_valid_percent, avg_ssl_cert_days_until_expiry, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            http_data.success_rate_percent,
            http_data.avg_tcp_timing_ms,
            http_data.avg_tls_timing_ms,
            http_data.avg_ttfb_timing_ms,
            http_data.avg_content_download_timing_ms,
            http_data.avg_total_time_ms,
            http_data.max_total_time_ms,
            http_data.successful_requests,
            http_data.failed_requests,
            status_code_json,
            http_data.ssl_valid_percent,
            http_data.avg_ssl_cert_days_until_expiry,
            http_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated HTTP metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                success_rate_percent, avg_tcp_timing_ms,
                avg_tls_timing_ms, avg_ttfb_timing_ms, avg_content_download_timing_ms,
                avg_total_time_ms, max_total_time_ms, successful_requests,
                failed_requests, status_code_distribution, ssl_valid_percent,
                avg_ssl_cert_days_until_expiry, target_id
         FROM agg_metric_http WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        let status_code_json: String = row.get(13)?;
        // Deserialize from Vec of tuples back to HashMap
        let status_code_vec: Vec<(u16, u32)> =
            serde_json::from_str(&status_code_json).unwrap_or_default();
        let status_code_distribution: HashMap<u16, u32> = status_code_vec.into_iter().collect();

        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::HttpGet,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::HttpGet(AggregatedHttpMetric {
                success_rate_percent: row.get(4)?,
                avg_tcp_timing_ms: row.get(5)?,
                avg_tls_timing_ms: row.get(6)?,
                avg_ttfb_timing_ms: row.get(7)?,
                avg_content_download_timing_ms: row.get(8)?,
                avg_total_time_ms: row.get(9)?,
                max_total_time_ms: row.get(10)?,
                successful_requests: row.get(11)?,
                failed_requests: row.get(12)?,
                status_code_distribution,
                ssl_valid_percent: row.get(14).ok(),
                avg_ssl_cert_days_until_expiry: row.get(15).ok(),
                target_id: row.get(16).ok(),
            }),
        })
    });

    match result {
        Ok(metric) => Ok(Some(metric)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Clean up old HTTP metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_http WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_http
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'http' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
