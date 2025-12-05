//! SNMP query task database operations for server
//!
//! This module handles all database operations specific to SNMP query monitoring
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedMetrics, AggregatedSnmpMetric};

/// Create SNMP aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_snmp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_response_time_ms REAL NOT NULL,
            successful_queries INTEGER NOT NULL,
            failed_queries INTEGER NOT NULL,
            first_value TEXT,
            first_value_type TEXT,
            oid_queried TEXT NOT NULL,
            target_id TEXT,
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_snmp table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_snmp_agent_id ON agg_metric_snmp(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_snmp_period ON agg_metric_snmp(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_snmp_task ON agg_metric_snmp(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated SNMP metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    snmp_data: &AggregatedSnmpMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_snmp (agent_id, task_name, period_start, period_end, sample_count, success_rate_percent, avg_response_time_ms, successful_queries, failed_queries, first_value, first_value_type, oid_queried, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            snmp_data.success_rate_percent,
            snmp_data.avg_response_time_ms,
            snmp_data.successful_queries,
            snmp_data.failed_queries,
            snmp_data.first_value,
            snmp_data.first_value_type,
            snmp_data.oid_queried,
            snmp_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old SNMP metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_snmp WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
