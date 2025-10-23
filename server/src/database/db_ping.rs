//! Ping task database operations for server
//!
//! This module handles all database operations specific to ICMP ping monitoring
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedMetrics, AggregatedPingMetric};

/// Create ping aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_ping (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
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
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_ping table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_ping_agent_id ON agg_metric_ping(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_ping_period ON agg_metric_ping(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_ping_task ON agg_metric_ping(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated ping metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    ping_data: &AggregatedPingMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_ping (agent_id, task_name, period_start, period_end, sample_count, avg_latency_ms, max_latency_ms, min_latency_ms, packet_loss_percent, successful_pings, failed_pings, domain, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            ping_data.avg_latency_ms,
            ping_data.max_latency_ms,
            ping_data.min_latency_ms,
            ping_data.packet_loss_percent,
            ping_data.successful_pings,
            ping_data.failed_pings,
            ping_data.domain,
            ping_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old ping metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_ping WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
