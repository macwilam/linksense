//! HTTP content check task database operations

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{
        AggregatedHttpContentMetric, AggregatedMetricData, AggregatedMetrics, MetricData,
        RawHttpContentMetric,
    },
};
use tracing::debug;

/// Create HTTP content check-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_http_content (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            status_code INTEGER,
            total_time_ms REAL,
            total_size INTEGER,
            regexp_match BOOLEAN,
            success BOOLEAN NOT NULL,
            error TEXT,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_http_content table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_http_content (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
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
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_http_content table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_http_content_timestamp ON raw_metric_http_content(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_http_content_task ON raw_metric_http_content(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_http_content_period ON agg_metric_http_content(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw HTTP content metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    http_content_data: &RawHttpContentMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_http_content (task_name, timestamp, status_code, total_time_ms, total_size, regexp_match, success, error, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            http_content_data.status_code.map(|s| s as i64),
            http_content_data.total_time_ms,
            http_content_data.total_size.map(|s| s as i64),
            http_content_data.regexp_match,
            http_content_data.success,
            http_content_data.error,
            http_content_data.target_id
        ],
    )?;
    debug!("Stored HTTP content metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated HTTP content metrics for a time period
pub(super) fn generate_aggregated_metrics(
    conn: &Connection,
    task_name: &str,
    period_start: u64,
    period_end: u64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            COUNT(*) as total_count,
            AVG(CASE WHEN success = 1 AND total_time_ms IS NOT NULL THEN total_time_ms END) as avg_total_time,
            MAX(CASE WHEN success = 1 AND total_time_ms IS NOT NULL THEN total_time_ms END) as max_total_time,
            AVG(CASE WHEN success = 1 AND total_size IS NOT NULL THEN total_size END) as avg_total_size,
            SUM(CASE WHEN success = 1 AND regexp_match = 1 THEN 1 ELSE 0 END) as regexp_matched_count,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_requests,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_requests,
            (SELECT target_id FROM raw_metric_http_content
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_http_content
        WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
        "#,
    )?;

    let row = stmt.query_row(
        params![task_name, period_start as i64, period_end as i64],
        |row| {
            let total_count: i64 = row.get("total_count")?;
            if total_count == 0 {
                return Ok(None);
            }

            let successful_requests: i64 = row.get("successful_requests")?;
            let failed_requests: i64 = row.get("failed_requests")?;
            let regexp_matched_count: i64 = row.get("regexp_matched_count")?;

            let success_rate_percent = if total_count > 0 {
                (successful_requests as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let regexp_match_rate_percent = if successful_requests > 0 {
                (regexp_matched_count as f64 / successful_requests as f64) * 100.0
            } else {
                0.0
            };

            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedHttpContentMetric {
                success_rate_percent,
                avg_total_time_ms: row.get("avg_total_time").unwrap_or(0.0),
                max_total_time_ms: row.get("max_total_time").unwrap_or(0.0),
                avg_total_size: row.get("avg_total_size").unwrap_or(0.0),
                regexp_match_rate_percent,
                successful_requests: successful_requests as u32,
                failed_requests: failed_requests as u32,
                regexp_matched_count: regexp_matched_count as u32,
                target_id,
            }))
        },
    )?;

    if let Some(http_content_metric) = row {
        let total_samples =
            http_content_metric.successful_requests + http_content_metric.failed_requests;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::HttpContent,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::HttpContent(http_content_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated HTTP content metric
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    http_content_data: &AggregatedHttpContentMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_http_content
        (task_name, period_start, period_end, sample_count, success_rate_percent, avg_total_time_ms, max_total_time_ms, avg_total_size, regexp_match_rate_percent, successful_requests, failed_requests, regexp_matched_count, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            http_content_data.success_rate_percent,
            http_content_data.avg_total_time_ms,
            http_content_data.max_total_time_ms,
            http_content_data.avg_total_size,
            http_content_data.regexp_match_rate_percent,
            http_content_data.successful_requests,
            http_content_data.failed_requests,
            http_content_data.regexp_matched_count,
            http_content_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated HTTP content metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                success_rate_percent, avg_total_time_ms, max_total_time_ms,
                avg_total_size, regexp_match_rate_percent, successful_requests,
                failed_requests, regexp_matched_count, target_id
         FROM agg_metric_http_content WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::HttpContent,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::HttpContent(AggregatedHttpContentMetric {
                success_rate_percent: row.get(4)?,
                avg_total_time_ms: row.get(5)?,
                max_total_time_ms: row.get(6)?,
                avg_total_size: row.get(7)?,
                regexp_match_rate_percent: row.get(8)?,
                successful_requests: row.get(9)?,
                failed_requests: row.get(10)?,
                regexp_matched_count: row.get(11)?,
                target_id: row.get(12).ok(),
            }),
        })
    });

    match result {
        Ok(metric) => Ok(Some(metric)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Clean up old HTTP content metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_http_content WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_http_content
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'http_content' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
