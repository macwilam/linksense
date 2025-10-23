//! TLS handshake task database operations for server
//!
//! This module handles all database operations specific to TLS handshake monitoring
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedMetrics, AggregatedTlsMetric};

/// Create TLS handshake aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_tls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
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
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_tls table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tls_agent_id ON agg_metric_tls(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tls_period ON agg_metric_tls(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tls_task ON agg_metric_tls(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated TLS metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    tls_data: &AggregatedTlsMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_tls (agent_id, task_name, period_start, period_end, sample_count, success_rate_percent, avg_tcp_timing_ms, avg_tls_timing_ms, successful_checks, failed_checks, ssl_valid_percent, avg_ssl_cert_days_until_expiry, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            tls_data.success_rate_percent,
            tls_data.avg_tcp_timing_ms,
            tls_data.avg_tls_timing_ms,
            tls_data.successful_checks,
            tls_data.failed_checks,
            tls_data.ssl_valid_percent,
            tls_data.avg_ssl_cert_days_until_expiry,
            tls_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old TLS metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_tls WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
