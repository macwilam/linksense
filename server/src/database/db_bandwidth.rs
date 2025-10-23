//! Bandwidth test task database operations for server
//!
//! This module handles all database operations specific to bandwidth testing
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedBandwidthMetric, AggregatedMetrics};

/// Create bandwidth aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_bandwidth (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
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
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_bandwidth table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_bandwidth_agent_id ON agg_metric_bandwidth(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_bandwidth_period ON agg_metric_bandwidth(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_bandwidth_task ON agg_metric_bandwidth(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated bandwidth metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    bandwidth_data: &AggregatedBandwidthMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_bandwidth (agent_id, task_name, period_start, period_end, sample_count, avg_bandwidth_mbps, max_bandwidth_mbps, min_bandwidth_mbps, successful_tests, failed_tests, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            bandwidth_data.avg_bandwidth_mbps,
            bandwidth_data.max_bandwidth_mbps,
            bandwidth_data.min_bandwidth_mbps,
            bandwidth_data.successful_tests,
            bandwidth_data.failed_tests,
            bandwidth_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old bandwidth metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_bandwidth WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
