//! SQL query task database operations for server
//!
//! This module handles all database operations specific to SQL query monitoring
//! on the server side, including table creation, metric storage, and cleanup.
//! This module is feature-gated and only compiled when sql-tasks feature is enabled.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use shared::metrics::{AggregatedMetrics, AggregatedSqlQueryMetric};

/// Create SQL query aggregated metrics table
#[cfg(feature = "sql-tasks")]
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_sql_query (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            success_rate_percent REAL NOT NULL,
            avg_total_time_ms REAL NOT NULL,
            max_total_time_ms REAL NOT NULL,
            avg_row_count REAL NOT NULL,
            max_row_count INTEGER NOT NULL,
            successful_queries INTEGER NOT NULL,
            failed_queries INTEGER NOT NULL,
            target_id TEXT,
            avg_value REAL,
            min_value REAL,
            max_value REAL,
            json_truncated_count INTEGER NOT NULL DEFAULT 0,
            received_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id),
            UNIQUE(agent_id, task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_sql_query table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_sql_query_agent_id ON agg_metric_sql_query(agent_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_sql_query_period ON agg_metric_sql_query(period_start, period_end)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_sql_query_task ON agg_metric_sql_query(task_name, period_start)",
        [],
    )?;

    // Migration: add new columns to existing tables
    let _ = conn.execute(
        "ALTER TABLE agg_metric_sql_query ADD COLUMN avg_value REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agg_metric_sql_query ADD COLUMN min_value REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agg_metric_sql_query ADD COLUMN max_value REAL",
        [],
    );
    let _ = conn.execute("ALTER TABLE agg_metric_sql_query ADD COLUMN json_truncated_count INTEGER NOT NULL DEFAULT 0", []);

    Ok(())
}

/// Store aggregated SQL query metric within a transaction
#[cfg(feature = "sql-tasks")]
pub(super) fn store_metric(
    tx: &Transaction,
    agent_id: &str,
    metric: &AggregatedMetrics,
    sql_data: &AggregatedSqlQueryMetric,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agg_metric_sql_query
        (agent_id, task_name, period_start, period_end, sample_count, success_rate_percent,
         avg_total_time_ms, max_total_time_ms, avg_row_count, max_row_count,
         successful_queries, failed_queries, target_id, avg_value, min_value, max_value, json_truncated_count)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
        params![
            agent_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            metric.sample_count,
            sql_data.success_rate_percent,
            sql_data.avg_total_time_ms,
            sql_data.max_total_time_ms,
            sql_data.avg_row_count,
            sql_data.max_row_count,
            sql_data.successful_queries,
            sql_data.failed_queries,
            sql_data.target_id,
            sql_data.avg_value,
            sql_data.min_value,
            sql_data.max_value,
            sql_data.json_truncated_count,
        ],
    )?;
    Ok(())
}

/// Delete old SQL query metrics
#[cfg(feature = "sql-tasks")]
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agg_metric_sql_query WHERE period_end < ?1",
        params![cutoff_time],
    )?;
    Ok(deleted)
}
