//! HTTP content check task database operations for server
//!
//! This module handles all database operations specific to HTTP content checking
//! on the server side, including table creation, metric storage, and cleanup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedHttpContentMetric, AggregatedMetrics};

/// Create HTTP content aggregated metrics table
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_http_content (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_total_time_ms REAL NOT NULL,
            max_total_time_ms REAL NOT NULL,
            avg_total_size REAL NOT NULL,
            regexp_match_rate_percent REAL NOT NULL,
            successful_requests INTEGER NOT NULL,
            failed_requests INTEGER NOT NULL,
            regexp_matched_count INTEGER NOT NULL,
            target_id TEXT,
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_http_content table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_content_agent_id ON agg_metric_http_content(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_content_period ON agg_metric_http_content(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_content_task ON agg_metric_http_content(task_name, period_start)",
        [],
    )?;

    Ok(())
}

/// Store aggregated HTTP content metric within a transaction
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    http_content_data: &AggregatedHttpContentMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_http_content (agent_id, task_name, period_start, period_end, sample_count, success_rate_percent, avg_total_time_ms, max_total_time_ms, avg_total_size, regexp_match_rate_percent, successful_requests, failed_requests, regexp_matched_count, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            http_content_data.success_rate_percent,
            http_content_data.avg_total_time_ms,
            http_content_data.max_total_time_ms,
            http_content_data.avg_total_size,
            http_content_data.regexp_match_rate_percent,
            http_content_data.successful_requests,
            http_content_data.failed_requests,
            http_content_data.regexp_matched_count,
            http_content_data.target_id,
        ],
    )?;
    Ok(())
}

/// Delete old HTTP content metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_http_content WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
