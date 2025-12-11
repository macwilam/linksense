//! DNS query task database operations

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{
        AggregatedDnsMetric, AggregatedMetricData, AggregatedMetrics, MetricData, RawDnsMetric,
    },
};
use std::collections::HashSet;
use tracing::debug;

/// Create DNS-specific tables and indexes
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS raw_metric_dns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            query_time_ms REAL,
            success BOOLEAN NOT NULL,
            record_count INTEGER,
            resolved_addresses TEXT,
            domain_queried TEXT NOT NULL,
            error TEXT,
            expected_ip TEXT,
            resolved_ip TEXT,
            correct_resolution BOOLEAN NOT NULL DEFAULT 1,
            target_id TEXT
        )
        "#,
        [],
    )
    .context("Failed to create raw_metric_dns table")?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agg_metric_dns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
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
            UNIQUE(task_name, period_start, period_end)
        )
        "#,
        [],
    )
    .context("Failed to create agg_metric_dns table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_dns_timestamp ON raw_metric_dns(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_raw_dns_task ON raw_metric_dns(task_name, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agg_dns_period ON agg_metric_dns(period_start, period_end)",
        [],
    )?;

    Ok(())
}

/// Store a raw DNS metric
pub(super) fn store_raw_metric(
    conn: &Connection,
    metric: &MetricData,
    dns_data: &RawDnsMetric,
) -> Result<i64> {
    let resolved_addresses_json = dns_data
        .resolved_addresses
        .as_ref()
        .map(|addrs| serde_json::to_string(addrs).unwrap_or_else(|_| "[]".to_string()));

    let row_id = conn.execute(
        r#"
        INSERT INTO raw_metric_dns (task_name, timestamp, query_time_ms, success, record_count, resolved_addresses, domain_queried, error, expected_ip, resolved_ip, correct_resolution, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            metric.task_name,
            metric.timestamp as i64,
            dns_data.query_time_ms,
            dns_data.success,
            dns_data.record_count.map(|c| c as i64),
            resolved_addresses_json,
            dns_data.domain_queried,
            dns_data.error,
            dns_data.expected_ip,
            dns_data.resolved_ip,
            dns_data.correct_resolution,
            dns_data.target_id
        ],
    )?;
    debug!("Stored DNS metric with ID: {}", row_id);
    Ok(row_id as i64)
}

/// Generate aggregated DNS metrics for a time period
pub(super) fn generate_aggregated_metrics(
    conn: &Connection,
    task_name: &str,
    period_start: u64,
    period_end: u64,
) -> Result<Option<AggregatedMetrics>> {
    // First query: get aggregate statistics
    let mut stmt = conn.prepare(
        r#"
        SELECT
            COUNT(*) as total_count,
            AVG(CASE WHEN success = 1 AND query_time_ms IS NOT NULL THEN query_time_ms END) as avg_query_time,
            MAX(CASE WHEN success = 1 AND query_time_ms IS NOT NULL THEN query_time_ms END) as max_query_time,
            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as successful_queries,
            SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failed_queries,
            MAX(domain_queried) as domain_queried,
            AVG(CASE WHEN correct_resolution = 1 THEN 1.0 ELSE 0.0 END) * 100.0 as correct_resolution_percent,
            (SELECT target_id FROM raw_metric_dns
             WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
             AND target_id IS NOT NULL
             ORDER BY timestamp ASC
             LIMIT 1) as first_target_id
        FROM raw_metric_dns
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

            let domain_queried: String = row
                .get("domain_queried")
                .unwrap_or_else(|_| "unknown".to_string());

            let correct_resolution_percent: f64 =
                row.get("correct_resolution_percent").unwrap_or(100.0);

            let target_id: Option<String> = row.get("first_target_id").ok();

            Ok(Some((
                success_rate_percent,
                row.get::<_, f64>("avg_query_time").unwrap_or(0.0),
                row.get::<_, f64>("max_query_time").unwrap_or(0.0),
                successful_queries as u32,
                failed_queries as u32,
                domain_queried,
                correct_resolution_percent,
                target_id,
            )))
        },
    )?;

    let Some((
        success_rate_percent,
        avg_query_time_ms,
        max_query_time_ms,
        successful_queries,
        failed_queries,
        domain_queried,
        correct_resolution_percent,
        target_id,
    )) = row
    else {
        return Ok(None);
    };

    // Second query: collect all resolved addresses separately to avoid GROUP_CONCAT issues
    // GROUP_CONCAT with JSON arrays doesn't work well because commas in JSON conflict with the separator
    let mut all_resolved_addresses = HashSet::new();
    let mut addr_stmt = conn.prepare(
        r#"
        SELECT DISTINCT resolved_addresses
        FROM raw_metric_dns
        WHERE task_name = ?1 AND timestamp >= ?2 AND timestamp < ?3
          AND resolved_addresses IS NOT NULL
        "#,
    )?;

    let rows = addr_stmt.query_map(
        params![task_name, period_start as i64, period_end as i64],
        |row| row.get::<_, String>(0),
    )?;

    for json_result in rows {
        if let Ok(json_str) = json_result {
            if let Ok(addresses) = serde_json::from_str::<Vec<String>>(&json_str) {
                for addr in addresses {
                    all_resolved_addresses.insert(addr);
                }
            }
        }
    }

    let dns_metric = AggregatedDnsMetric {
        success_rate_percent,
        avg_query_time_ms,
        max_query_time_ms,
        successful_queries,
        failed_queries,
        all_resolved_addresses,
        domain_queried,
        correct_resolution_percent,
        target_id,
    };

    let total_samples = dns_metric.successful_queries + dns_metric.failed_queries;
    Ok(Some(AggregatedMetrics::new(
        task_name.to_string(),
        TaskType::DnsQuery,
        period_start,
        period_end,
        total_samples,
        AggregatedMetricData::DnsQuery(dns_metric),
    )))
}

/// Store aggregated DNS metric
pub(super) fn store_aggregated_metric(
    conn: &Connection,
    metrics: &AggregatedMetrics,
    dns_data: &AggregatedDnsMetric,
) -> Result<i64> {
    let all_addresses_json = serde_json::to_string(&dns_data.all_resolved_addresses)
        .unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        r#"
        INSERT OR REPLACE INTO agg_metric_dns
        (task_name, period_start, period_end, sample_count, success_rate_percent, avg_query_time_ms, max_query_time_ms, successful_queries, failed_queries, all_resolved_addresses, domain_queried, correct_resolution_percent, target_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            metrics.task_name,
            metrics.period_start as i64,
            metrics.period_end as i64,
            metrics.sample_count,
            dns_data.success_rate_percent,
            dns_data.avg_query_time_ms,
            dns_data.max_query_time_ms,
            dns_data.successful_queries,
            dns_data.failed_queries,
            all_addresses_json,
            dns_data.domain_queried,
            dns_data.correct_resolution_percent,
            dns_data.target_id
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load aggregated DNS metric by row ID
pub(super) fn load_aggregated_metric(
    conn: &Connection,
    row_id: i64,
) -> Result<Option<AggregatedMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT task_name, period_start, period_end, sample_count,
                success_rate_percent, avg_query_time_ms, max_query_time_ms,
                successful_queries, failed_queries, all_resolved_addresses,
                domain_queried, correct_resolution_percent, target_id
         FROM agg_metric_dns WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![row_id], |row| {
        let all_addresses_json: String = row.get(9).unwrap_or_else(|_| "[]".to_string());
        let all_resolved_addresses: HashSet<String> =
            serde_json::from_str(&all_addresses_json).unwrap_or_else(|_| HashSet::new());

        Ok(AggregatedMetrics {
            task_name: row.get(0)?,
            task_type: TaskType::DnsQuery,
            period_start: row.get::<_, i64>(1)? as u64,
            period_end: row.get::<_, i64>(2)? as u64,
            sample_count: row.get(3)?,
            data: AggregatedMetricData::DnsQuery(AggregatedDnsMetric {
                success_rate_percent: row.get(4)?,
                avg_query_time_ms: row.get(5)?,
                max_query_time_ms: row.get(6)?,
                successful_queries: row.get(7)?,
                failed_queries: row.get(8)?,
                all_resolved_addresses,
                domain_queried: row.get(10)?,
                correct_resolution_percent: row.get(11)?,
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

/// Clean up old DNS metrics
pub(super) fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<(usize, usize)> {
    let raw_deleted = conn.execute(
        "DELETE FROM raw_metric_dns WHERE timestamp < ?1",
        params![cutoff_time],
    )?;

    let agg_deleted = conn.execute(
        r#"
        DELETE FROM agg_metric_dns
        WHERE period_end < ?1
          AND id NOT IN (
              SELECT metric_row_id FROM metric_send_queue
              WHERE metric_type = 'dns' AND status != 'sent'
          )
        "#,
        params![cutoff_time],
    )?;

    Ok((raw_deleted, agg_deleted))
}
