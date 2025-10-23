//! TLS handshake task database operations

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{
        AggregatedMetricData, AggregatedMetrics, AggregatedTlsMetric, MetricData, RawTlsMetric,
    },
};
use tracing::debug;

/// Create TLS handshake-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_tls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            tcp_timing_ms REAL,
            tls_timing_ms REAL,
            ssl_valid BOOLEAN,
            ssl_cert_days_until_expiry INTEGER,
            success BOOLEAN NOT NULL,
            error TEXT,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_tls table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_tls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_tcp_timing_ms REAL NOT NULL,
            avg_tls_timing_ms REAL NOT NULL,
            successful_checks INTEGER NOT NULL,
            failed_checks INTEGER NOT NULL,
            ssl_valid_percent REAL,
            avg_ssl_cert_days_until_expiry REAL,
            target_id TEXT,
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_tls table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_tls_timestamp ON raw_metric_tls(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_tls_task ON raw_metric_tls(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tls_period ON agg_metric_tls(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw TLS metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    tls_data: &RawTlsMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_tls (task_name, timestamp, tcp_timing_ms, tls_timing_ms, ssl_valid, ssl_cert_days_until_expiry, success, error, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            tls_data.tcp_timing_ms,
            tls_data.tls_timing_ms,
            tls_data.ssl_valid,
            tls_data.ssl_cert_days_until_expiry,
            tls_data.success,
            tls_data.error,
            tls_data.target_id
        ],
    )?;
    debug!("Stored TLS metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated TLS metrics for a time period
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
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_checks,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_checks,
            AVG(CASE WHEN ssl_valid = 1 THEN 1.0 WHEN ssl_valid = 0 THEN 0.0 END) * 100.0 as ssl_valid_percent,
            AVG(CASE WHEN ssl_cert_days_until_expiry IS NOT NULL THEN ssl_cert_days_until_expiry END) as avg_ssl_cert_days_until_expiry,
            (SELECT target_id FROM raw_metric_tls
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_tls
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

            let successful_checks: i64 = row.get("successful_checks")?;
            let failed_checks: i64 = row.get("failed_checks")?;
            let success_rate_percent = if total_count > 0 {
                (successful_checks as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let ssl_valid_percent: Option<f64> = row.get("ssl_valid_percent").ok();
            let avg_ssl_cert_days_until_expiry: Option<f64> =
                row.get("avg_ssl_cert_days_until_expiry").ok();
            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedTlsMetric {
                success_rate_percent,
                avg_tcp_timing_ms: row.get("avg_tcp").unwrap_or(0.0),
                avg_tls_timing_ms: row.get("avg_tls").unwrap_or(0.0),
                successful_checks: successful_checks as u32,
                failed_checks: failed_checks as u32,
                ssl_valid_percent: ssl_valid_percent.unwrap_or(0.0),
                avg_ssl_cert_days_until_expiry: avg_ssl_cert_days_until_expiry.unwrap_or(0.0),
                target_id,
            }))
        },
    )?;

    if let Some(tls_metric) = row {
        let total_samples = tls_metric.successful_checks + tls_metric.failed_checks;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::TlsHandshake,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::TlsHandshake(tls_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated TLS metric
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    tls_data: &AggregatedTlsMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_tls
        (task_name, period_start, period_end, sample_count, success_rate_percent, avg_tcp_timing_ms, avg_tls_timing_ms, successful_checks, failed_checks, ssl_valid_percent, avg_ssl_cert_days_until_expiry, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            tls_data.success_rate_percent,
            tls_data.avg_tcp_timing_ms,
            tls_data.avg_tls_timing_ms,
            tls_data.successful_checks,
            tls_data.failed_checks,
            tls_data.ssl_valid_percent,
            tls_data.avg_ssl_cert_days_until_expiry,
            tls_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated TLS metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                success_rate_percent, avg_tcp_timing_ms, avg_tls_timing_ms,
                successful_checks, failed_checks, ssl_valid_percent,
                avg_ssl_cert_days_until_expiry, target_id
         FROM agg_metric_tls WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::TlsHandshake,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::TlsHandshake(AggregatedTlsMetric {
                success_rate_percent: row.get(4)?,
                avg_tcp_timing_ms: row.get(5)?,
                avg_tls_timing_ms: row.get(6)?,
                successful_checks: row.get(7)?,
                failed_checks: row.get(8)?,
                ssl_valid_percent: row.get(9)?,
                avg_ssl_cert_days_until_expiry: row.get(10)?,
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

/// Clean up old TLS metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_tls WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_tls
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'tls' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
