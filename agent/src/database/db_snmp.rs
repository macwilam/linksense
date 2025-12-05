//! SNMP query task database operations

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{
        AggregatedMetricData, AggregatedMetrics, AggregatedSnmpMetric, MetricData, RawSnmpMetric,
    },
};
use tracing::debug;

/// Create SNMP-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_snmp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            response_time_ms REAL,
            success BOOLEAN NOT NULL,
            value TEXT,
            value_type TEXT,
            oid_queried TEXT NOT NULL,
            error TEXT,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_snmp table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_snmp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
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
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_snmp table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_snmp_timestamp ON raw_metric_snmp(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_snmp_task ON raw_metric_snmp(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_snmp_period ON agg_metric_snmp(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw SNMP metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    snmp_data: &RawSnmpMetric,
) -> Result<i64> {
    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_snmp (task_name, timestamp, response_time_ms, success, value, value_type, oid_queried, error, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            snmp_data.response_time_ms,
            snmp_data.success,
            snmp_data.value,
            snmp_data.value_type,
            snmp_data.oid_queried,
            snmp_data.error,
            snmp_data.target_id
        ],
    )?;
    debug!("Stored SNMP metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated SNMP metrics for a time period
/// Since SNMP tasks run at minimum 60s intervals, aggregation typically contains 1 sample
/// We use first-value strategy for value aggregation
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
            AVG(CASE WHEN success = 1 AND response_time_ms IS NOT NULL THEN response_time_ms END) as avg_response_time,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_queries,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_queries,
            MAX(oid_queried) as oid_queried,
            (SELECT value FROM raw_metric_snmp
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3 AND success = 1
             ORDER BY timestamp ASC
             LIMIT 1) as first_value,
            (SELECT value_type FROM raw_metric_snmp
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3 AND success = 1
             ORDER BY timestamp ASC
             LIMIT 1) as first_value_type,
            (SELECT target_id FROM raw_metric_snmp
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_snmp
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

            let successful_queries: i64 = row.get("successful_queries")?;
            let failed_queries: i64 = row.get("failed_queries")?;
            let success_rate_percent = if total_count > 0 {
                (successful_queries as f64 / total_count as f64) * 100.0
            } else {
                0.0
            };

            let oid_queried: String = row
                .get("oid_queried")
                .unwrap_or_else(|_| "unknown".to_string());

            let first_value: Option<String> = row.get("first_value").ok();
            let first_value_type: Option<String> = row.get("first_value_type").ok();
            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some(AggregatedSnmpMetric {
                success_rate_percent,
                avg_response_time_ms: row.get("avg_response_time").unwrap_or(0.0),
                successful_queries: successful_queries as u32,
                failed_queries: failed_queries as u32,
                first_value,
                first_value_type,
                oid_queried,
                target_id,
            }))
        },
    )?;

    if let Some(snmp_metric) = row {
        let total_samples = snmp_metric.successful_queries + snmp_metric.failed_queries;
        return Ok(Some(AggregatedMetrics::new(
            task_name.to_string(),
            TaskType::Snmp,
            period_start,
            period_end,
            total_samples,
            AggregatedMetricData::Snmp(snmp_metric),
        )));
    }

    Ok(None)
}

/// Store aggregated SNMP metric
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    snmp_data: &AggregatedSnmpMetric,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_snmp
        (task_name, period_start, period_end, sample_count, success_rate_percent, avg_response_time_ms, successful_queries, failed_queries, first_value, first_value_type, oid_queried, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            snmp_data.success_rate_percent,
            snmp_data.avg_response_time_ms,
            snmp_data.successful_queries,
            snmp_data.failed_queries,
            snmp_data.first_value,
            snmp_data.first_value_type,
            snmp_data.oid_queried,
            snmp_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated SNMP metric by row ID
#[allow(dead_code)]
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                success_rate_percent, avg_response_time_ms,
                successful_queries, failed_queries, first_value, first_value_type,
                oid_queried, target_id
         FROM agg_metric_snmp WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::Snmp,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::Snmp(AggregatedSnmpMetric {
                success_rate_percent: row.get(4)?,
                avg_response_time_ms: row.get(5)?,
                successful_queries: row.get(6)?,
                failed_queries: row.get(7)?,
                first_value: row.get(8).ok(),
                first_value_type: row.get(9).ok(),
                oid_queried: row.get(10)?,
                target_id: row.get(11).ok(),
            }),
        })
    });

    match result {
        Ok(metric) => Ok(Some(metric)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Clean up old SNMP metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_snmp WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_snmp
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'snmp' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
