//! TCP task database operations for server
//!
//! This module handles all database operations specific to TCP connection monitoring
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedMetrics, AggregatedTcpMetric};

/// Create TCP aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_tcp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
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
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_tcp table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tcp_agent_id ON agg_metric_tcp(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tcp_period ON agg_metric_tcp(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_tcp_task ON agg_metric_tcp(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated TCP metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    tcp_data: &AggregatedTcpMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_tcp (agent_id, task_name, period_start, period_end, sample_count, avg_connect_time_ms, max_connect_time_ms, min_connect_time_ms, failure_percent, successful_connections, failed_connections, host, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            tcp_data.avg_connect_time_ms,
            tcp_data.max_connect_time_ms,
            tcp_data.min_connect_time_ms,
            tcp_data.failure_percent,
            tcp_data.successful_connections,
            tcp_data.failed_connections,
            tcp_data.host,
            tcp_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old TCP metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_tcp WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
