//! Queue management for metric sending
//!
//! This module handles the send queue for aggregated metrics, including
//! retry logic with exponential backoff and queue statistics.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{AggregatedMetricData, AggregatedMetrics},
};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Status of a metric in the send queue
#[derive(Debug, Clone, PartialEq)]
pub enum QueueStatus {
    Pending, // Ready to send
    Sending, // Currently being sent (prevents duplicate sends)
    Sent,    // Successfully sent
    Failed,  // Permanently failed after max retries
}

impl QueueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueueStatus::Pending => "pending",
            QueueStatus::Sending => "sending",
            QueueStatus::Sent => "sent",
            QueueStatus::Failed => "failed",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(QueueStatus::Pending),
            "sending" => Ok(QueueStatus::Sending),
            "sent" => Ok(QueueStatus::Sent),
            "failed" => Ok(QueueStatus::Failed),
            _ => Err(anyhow::anyhow!("Invalid queue status: {}", s)),
        }
    }
}

/// Queue entry for tracking metric sends
pub struct QueueEntry {
    pub id: i64,
    pub metric_type: String,
    pub metric_row_id: i64,
    pub task_name: String,
    pub period_start: u64,
    pub period_end: u64,
    pub retry_count: i32,
}

/// Metric with queue metadata
#[derive(Debug, Clone)]
pub struct QueuedMetric {
    pub queue_id: i64,
    pub metric: AggregatedMetrics,
    pub retry_count: i32,
}

/// Queue statistics for monitoring
#[derive(Debug, Default, Clone)]
pub struct QueueStats {
    pub pending: i64,
    pub sending: i64,
    pub sent: i64,
    pub failed: i64,
}

/// Create metric send queue table
pub fn create_queue_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS metric_send_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,

            -- Metric identification
            metric_type TEXT NOT NULL,
            metric_row_id INTEGER NOT NULL,
            task_name TEXT NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,

            -- Send tracking
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            sent_at INTEGER,

            -- Retry management
            retry_count INTEGER NOT NULL DEFAULT 0,
            last_retry_at INTEGER,
            last_error TEXT,
            next_retry_at INTEGER,

            -- Prevent duplicates
            UNIQUE(metric_type, metric_row_id)
        )
        "#,
        [],
    )
    .context("Failed to create metric_send_queue table")?;

    // Indexes for efficient queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_queue_status
         ON metric_send_queue(status, next_retry_at)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_queue_created
         ON metric_send_queue(created_at)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_queue_task
         ON metric_send_queue(task_name, period_start, period_end)",
        [],
    )?;

    info!("Metric send queue table created");
    Ok(())
}

/// Queue an aggregated metric for sending to the server
pub fn enqueue_metric_for_send(
    conn: &Connection,
    metric: &AggregatedMetrics,
    metric_row_id: i64,
) -> Result<()> {
    let now = current_timestamp();

    let metric_type = match &metric.data {
        AggregatedMetricData::Ping(_) => "ping",
        AggregatedMetricData::Tcp(_) => "tcp",
        AggregatedMetricData::HttpGet(_) => "http",
        AggregatedMetricData::TlsHandshake(_) => "tls",
        AggregatedMetricData::HttpContent(_) => "http_content",
        AggregatedMetricData::DnsQuery(_) => "dns",
        AggregatedMetricData::Bandwidth(_) => "bandwidth",
        #[cfg(feature = "sql-tasks")]
        AggregatedMetricData::SqlQuery(_) => "sql_query",
    };

    // Use INSERT OR IGNORE to prevent duplicate queue entries
    conn.execute(
        r#"
        INSERT OR IGNORE INTO metric_send_queue (
            metric_type, metric_row_id, task_name, period_start, period_end,
            status, created_at, next_retry_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
        "#,
        params![
            metric_type,
            metric_row_id,
            metric.task_name,
            metric.period_start as i64,
            metric.period_end as i64,
            QueueStatus::Pending.as_str(),
            now as i64,
        ],
    )?;

    debug!(
        "Enqueued {} metric {} (row_id={}) for sending",
        metric_type, metric.task_name, metric_row_id
    );

    Ok(())
}

/// Get next batch of metrics to send, respecting exponential backoff
pub fn get_pending_queue_entries(conn: &Connection, batch_size: usize) -> Result<Vec<QueueEntry>> {
    let now = current_timestamp();

    // Get queue entries that are pending and ready to retry
    let mut stmt = conn.prepare(
        r#"
        SELECT
            id, metric_type, metric_row_id, task_name,
            period_start, period_end, retry_count
        FROM metric_send_queue
        WHERE status = 'pending'
          AND next_retry_at <= ?1
        ORDER BY created_at ASC
        LIMIT ?2
        "#,
    )?;

    let rows = stmt.query_map(params![now as i64, batch_size as i64], |row| {
        Ok(QueueEntry {
            id: row.get(0)?,
            metric_type: row.get(1)?,
            metric_row_id: row.get(2)?,
            task_name: row.get(3)?,
            period_start: row.get::<_, i64>(4)? as u64,
            period_end: row.get::<_, i64>(5)? as u64,
            retry_count: row.get(6)?,
        })
    })?;

    let queue_entries: Vec<QueueEntry> = rows.collect::<Result<Vec<_>, _>>()?;

    Ok(queue_entries)
}

/// Mark metrics as being sent (status = 'sending')
pub fn mark_as_sending(conn: &Connection, queue_ids: &[i64]) -> Result<()> {
    if queue_ids.is_empty() {
        return Ok(());
    }

    let now = current_timestamp();

    let placeholders = queue_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE metric_send_queue
         SET status = 'sending', last_retry_at = ?
         WHERE id IN ({})",
        placeholders
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now as i64)];
    for id in queue_ids {
        params_vec.push(Box::new(*id));
    }

    conn.execute(&sql, rusqlite::params_from_iter(params_vec))?;

    Ok(())
}

/// Mark metrics as successfully sent
pub fn mark_as_sent(conn: &Connection, queue_ids: &[i64]) -> Result<()> {
    if queue_ids.is_empty() {
        return Ok(());
    }

    let now = current_timestamp();

    let placeholders = queue_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE metric_send_queue
         SET status = 'sent', sent_at = ?
         WHERE id IN ({})",
        placeholders
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now as i64)];
    for id in queue_ids {
        params_vec.push(Box::new(*id));
    }

    conn.execute(&sql, rusqlite::params_from_iter(params_vec))?;

    debug!("Marked {} metrics as sent", queue_ids.len());
    Ok(())
}

/// Mark a send attempt as failed with exponential backoff
pub fn mark_as_failed(
    conn: &Connection,
    queue_id: i64,
    error_msg: &str,
    max_retries: i32,
) -> Result<()> {
    let now = current_timestamp();

    // Get current retry count
    let retry_count: i32 = conn.query_row(
        "SELECT retry_count FROM metric_send_queue WHERE id = ?1",
        params![queue_id],
        |row| row.get(0),
    )?;

    let new_retry_count = retry_count + 1;

    if new_retry_count >= max_retries {
        // Permanent failure after max retries
        conn.execute(
            "UPDATE metric_send_queue
             SET status = 'failed',
                 retry_count = ?1,
                 last_retry_at = ?2,
                 last_error = ?3
             WHERE id = ?4",
            params![new_retry_count, now as i64, error_msg, queue_id],
        )?;

        warn!(
            "Metric queue_id={} permanently failed after {} retries: {}",
            queue_id, max_retries, error_msg
        );
    } else {
        // Calculate exponential backoff: 2^retry_count minutes (capped at 60 minutes)
        let backoff_minutes = 2_i32.pow(new_retry_count as u32).min(60);
        let next_retry_at = now + (backoff_minutes as u64 * 60);

        conn.execute(
            "UPDATE metric_send_queue
             SET status = 'pending',
                 retry_count = ?1,
                 last_retry_at = ?2,
                 next_retry_at = ?3,
                 last_error = ?4
             WHERE id = ?5",
            params![
                new_retry_count,
                now as i64,
                next_retry_at as i64,
                error_msg,
                queue_id
            ],
        )?;

        debug!(
            "Metric queue_id={} will retry in {} minutes (attempt {}/{})",
            queue_id, backoff_minutes, new_retry_count, max_retries
        );
    }

    Ok(())
}

/// Remove a metric from the send queue
pub fn remove_from_queue(conn: &Connection, queue_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM metric_send_queue WHERE id = ?1",
        params![queue_id],
    )?;
    Ok(())
}

/// Clean up successfully sent queue entries
pub fn cleanup_sent_queue_entries(conn: &Connection, older_than_hours: i64) -> Result<usize> {
    let cutoff = current_timestamp() - (older_than_hours as u64 * 3600);

    let count = conn.execute(
        "DELETE FROM metric_send_queue
         WHERE status = 'sent' AND sent_at < ?1",
        params![cutoff as i64],
    )?;

    if count > 0 {
        debug!("Cleaned up {} sent queue entries", count);
    }

    Ok(count)
}

/// Get queue statistics for monitoring
pub fn get_queue_stats(conn: &Connection) -> Result<QueueStats> {
    let mut stmt =
        conn.prepare("SELECT status, COUNT(*) FROM metric_send_queue GROUP BY status")?;

    let mut stats = QueueStats::default();

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;

    for row in rows {
        let (status, count) = row?;
        match status.as_str() {
            "pending" => stats.pending = count,
            "sending" => stats.sending = count,
            "sent" => stats.sent = count,
            "failed" => stats.failed = count,
            _ => {}
        }
    }

    Ok(stats)
}

/// Fetch a specific metric from the appropriate table by type and row ID
pub fn fetch_metric_by_type_and_id(
    conn: &Connection,
    metric_type: &str,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    match metric_type {
        "ping" => super::db_ping::load_aggregated_metric(conn, row_id),
        "tcp" => super::db_tcp::load_aggregated_metric(conn, row_id),
        "http" => super::db_http::load_aggregated_metric(conn, row_id),
        "tls" => super::db_tls::load_aggregated_metric(conn, row_id),
        "http_content" => super::db_http_content::load_aggregated_metric(conn, row_id),
        "dns" => super::db_dns::load_aggregated_metric(conn, row_id),
        "bandwidth" => super::db_bandwidth::load_aggregated_metric(conn, row_id),
        #[cfg(feature = "sql-tasks")]
        "sql_query" => super::db_sql::load_aggregated_metric(conn, row_id),
        _ => Err(anyhow::anyhow!("Unknown metric type: {}", metric_type)),
    }
}

/// A helper function to get the current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn get_queued_metrics(conn: &Connection, batch_size: usize) -> Result<Vec<QueuedMetric>> {
    let queue_entries = get_pending_queue_entries(conn, batch_size)?;

    // Now fetch the actual metric data for each queue entry
    let mut metrics = Vec::new();

    for entry in queue_entries {
        let metric = fetch_metric_by_type_and_id(conn, &entry.metric_type, entry.metric_row_id)?;

        if let Some(metric) = metric {
            metrics.push(QueuedMetric {
                queue_id: entry.id,
                metric,
                retry_count: entry.retry_count,
            });
        } else {
            // Metric was deleted, remove from queue
            warn!(
                "Metric {} {} not found, removing from queue",
                entry.metric_type, entry.metric_row_id
            );
            remove_from_queue(conn, entry.id)?;
        }
    }

    Ok(metrics)
}
