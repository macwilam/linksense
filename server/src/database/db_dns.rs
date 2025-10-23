//! DNS query task database operations for server
//!
//! This module handles all database operations specific to DNS query monitoring
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedDnsMetric, AggregatedMetrics};

/// Create DNS aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_dns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_query_time_ms REAL NOT NULL,
            max_query_time_ms REAL NOT NULL,
            successful_queries INTEGER NOT NULL,
            failed_queries INTEGER NOT NULL,
            all_resolved_addresses TEXT,
            domain_queried TEXT NOT NULL,
            correct_resolution_percent REAL NOT NULL DEFAULT 100.0,
            target_id TEXT,
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_dns table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_dns_agent_id ON agg_metric_dns(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_dns_period ON agg_metric_dns(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_dns_task ON agg_metric_dns(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated DNS metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    dns_data: &AggregatedDnsMetric,
) -> Result<()> {
    let addresses_json = serde_json::to_string(&dns_data.all_resolved_addresses)?;
    tx.execute(
        r#"
        INSERT INTO agg_metric_dns (agent_id, task_name, period_start, period_end, sample_count, success_rate_percent, avg_query_time_ms, max_query_time_ms, successful_queries, failed_queries, all_resolved_addresses, domain_queried, correct_resolution_percent, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            dns_data.success_rate_percent,
            dns_data.avg_query_time_ms,
            dns_data.max_query_time_ms,
            dns_data.successful_queries,
            dns_data.failed_queries,
            addresses_json,
            dns_data.domain_queried,
            dns_data.correct_resolution_percent,
            dns_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old DNS metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_dns WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
