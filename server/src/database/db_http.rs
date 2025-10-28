//! HTTP GET task database operations for server
//!
//! This module handles all database operations specific to HTTP GET monitoring
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedHttpMetric, AggregatedMetrics};

/// Create HTTP aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_http (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_tcp_timing_ms REAL NOT NULL,
            avg_tls_timing_ms REAL NOT NULL,
            avg_ttfb_timing_ms REAL NOT NULL,
            avg_content_download_timing_ms REAL NOT NULL,
            avg_total_time_ms REAL NOT NULL,
            max_total_time_ms REAL NOT NULL,
            successful_requests INTEGER NOT NULL,
            failed_requests INTEGER NOT NULL,
            status_code_distribution TEXT,
            ssl_valid_percent REAL,
            avg_ssl_cert_days_until_expiry REAL,
            target_id TEXT,
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_http table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_agent_id ON agg_metric_http(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_period ON agg_metric_http(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_task ON agg_metric_http(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated HTTP metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    http_data: &AggregatedHttpMetric,
) -> Result<()> {
    // Convert HashMap to Vec of tuples for proper JSON serialization
    // JSON only supports string keys, so we serialize as array of [code, count] pairs
    let status_code_vec: Vec<(u16, u32)> = http_data
        .status_code_distribution
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    let status_code_json = serde_json::to_string(&status_code_vec)?;
    tx.execute(
        r#"
        INSERT INTO agg_metric_http (agent_id, task_name, period_start, period_end, sample_count, success_rate_percent, avg_tcp_timing_ms, avg_tls_timing_ms, avg_ttfb_timing_ms, avg_content_download_timing_ms, avg_total_time_ms, max_total_time_ms, successful_requests, failed_requests, status_code_distribution, ssl_valid_percent, avg_ssl_cert_days_until_expiry, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
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
            http_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old HTTP metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_http WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
