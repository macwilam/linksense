//! Database management for the network monitoring agent
//!
//! This module handles SQLite database operations for storing raw metrics
//! and aggregated data locally on the agent.
// The agent uses a local SQLite database for several reasons:
// 1.  **Persistence**: Raw metrics are saved to disk, so they are not lost if the
//     agent restarts.
// 2.  **Buffering**: If the central server is unreachable, the agent can continue
//     to collect and store metrics locally. These can be sent later when the
//     connection is restored.
// 3.  **Data Aggregation**: Raw, high-frequency metrics can be aggregated locally
//     into less frequent, summary data points. This reduces the amount of data
//     that needs to be sent to the central server, saving bandwidth.
//
// This module provides a simple, async-friendly interface for all database
// operations required by the agent.

// Task-specific database modules
mod db_bandwidth;
mod db_dns;
mod db_http;
mod db_http_content;
mod db_ping;
mod db_queue;
#[cfg(feature = "sql-tasks")]
mod db_sql;
mod db_tcp;
mod db_tls;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::{
    config::TaskType,
    metrics::{AggregatedMetricData, AggregatedMetrics, MetricData, RawMetricData},
};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Default database file name. Using a constant avoids magic strings.
const DATABASE_FILE: &str = "agent_metrics.db";

// Re-export queue types for public API
pub use db_queue::{QueueStats, QueuedMetric};

/// SQLite database manager for agent metrics.
/// This struct encapsulates the database connection and related operations.
/// The `connection` field is an `Option<Connection>` to allow for lazy
//  initialization of the connection, which is a good practice.
pub struct AgentDatabase {
    /// Path to the database file.
    db_path: PathBuf,
    /// The active SQLite connection. It's optional because the connection
    /// might be closed or not yet opened.
    connection: Option<Connection>,
    /// Database busy timeout in seconds
    busy_timeout_seconds: u64,
}

impl AgentDatabase {
    /// Create a new database manager for a given data directory.
    /// This constructor sets up the path to the database file and ensures
    /// that the data directory exists, creating it if necessary.
    pub fn new<P: AsRef<Path>>(data_dir: P, busy_timeout_seconds: u64) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        if !data_dir.exists() {
            std::fs::create_dir_all(data_dir).with_context(|| {
                format!("Failed to create data directory: {}", data_dir.display())
            })?;
        }

        let db_path = data_dir.join(DATABASE_FILE);

        Ok(Self {
            db_path,
            connection: None,
            busy_timeout_seconds,
        })
    }

    /// Initialize the database by creating the necessary tables and indexes.
    /// This method is idempotent; it uses `CREATE TABLE IF NOT EXISTS`, so it's safe
    /// to call on every application startup.
    pub async fn initialize(&mut self) -> Result<()> {
        info!("Initializing agent database at {}", self.db_path.display());

        // Get connection and create tables
        let conn = self.get_connection()?;

        // Create task-specific raw metrics tables
        db_ping::create_tables(conn)?;
        db_tcp::create_tables(conn)?;
        db_http::create_tables(conn)?;
        db_tls::create_tables(conn)?;
        db_http_content::create_tables(conn)?;
        db_dns::create_tables(conn)?;
        db_bandwidth::create_tables(conn)?;
        #[cfg(feature = "sql-tasks")]
        db_sql::create_tables(conn)?;

        // Create queue table
        db_queue::create_queue_table(conn)?;

        info!("Database initialization complete");
        Ok(())
    }

    /// Lazily gets a mutable reference to the database connection.
    /// If the connection doesn't exist, it's created. This method encapsulates
    /// the logic for opening and configuring the connection.
    fn get_connection(&mut self) -> Result<&mut Connection> {
        if self.connection.is_none() {
            let conn = Connection::open(&self.db_path)
                .with_context(|| format!("Failed to open database: {}", self.db_path.display()))?;

            // WAL (Write-Ahead Logging) mode is generally better for concurrency.
            // It allows readers to continue reading while a writer is writing.
            conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
                .context("Failed to enable WAL mode")?;

            // Configure WAL auto-checkpoint at 1000 pages (~4MB typically)
            // This prevents the WAL file from growing unbounded, which would cause
            // memory and disk usage to increase over time. The checkpoint merges
            // WAL changes back into the main database file periodically.
            conn.query_row("PRAGMA wal_autocheckpoint=1000", [], |_| Ok(()))
                .context("Failed to set WAL auto-checkpoint")?;
            debug!("WAL auto-checkpoint configured at 1000 pages");

            // Set a busy timeout to handle cases where multiple parts of the application
            // might try to access the database at the same time. This will make the
            // connection wait for a bit if the database is locked, rather than failing immediately.
            conn.busy_timeout(std::time::Duration::from_secs(self.busy_timeout_seconds))
                .context("Failed to set busy timeout")?;

            self.connection = Some(conn);
        }

        // `unwrap` is safe here because we've just ensured `self.connection` is `Some`.
        Ok(self.connection.as_mut().unwrap())
    }

    /// Store a single raw metric measurement in the database.
    pub async fn store_raw_metric(&mut self, metric: &MetricData) -> Result<i64> {
        debug!("Storing raw metric for task: {}", metric.task_name);

        let conn = self.get_connection()?;

        match &metric.data {
            RawMetricData::Ping(ping_data) => db_ping::store_raw_metric(&conn, metric, ping_data),
            RawMetricData::Tcp(tcp_data) => db_tcp::store_raw_metric(&conn, metric, tcp_data),
            RawMetricData::HttpGet(http_data) => {
                db_http::store_raw_metric(&conn, metric, http_data)
            }
            RawMetricData::TlsHandshake(tls_data) => {
                db_tls::store_raw_metric(&conn, metric, tls_data)
            }
            RawMetricData::HttpContent(http_content_data) => {
                db_http_content::store_raw_metric(&conn, metric, http_content_data)
            }
            RawMetricData::DnsQuery(dns_data) => db_dns::store_raw_metric(&conn, metric, dns_data),
            RawMetricData::Bandwidth(bandwidth_data) => {
                db_bandwidth::store_raw_metric(&conn, metric, bandwidth_data)
            }
            #[cfg(feature = "sql-tasks")]
            RawMetricData::SqlQuery(sql_data) => db_sql::store_raw_metric(&conn, metric, sql_data),
        }
    }

    /// Generate aggregated metrics using SQL GROUP BY for a specific task and time period.
    pub async fn generate_aggregated_metrics(
        &mut self,
        task_name: &str,
        task_type: &TaskType,
        period_start: u64,
        period_end: u64,
    ) -> Result<Option<AggregatedMetrics>> {
        debug!(
            "Generating aggregated metrics for task '{}' from {} to {}",
            task_name, period_start, period_end
        );

        let conn = self.get_connection()?;

        match task_type {
            TaskType::Ping => {
                return db_ping::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            TaskType::Tcp => {
                return db_tcp::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            TaskType::HttpGet => {
                return db_http::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            TaskType::TlsHandshake => {
                return db_tls::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            TaskType::HttpContent => {
                return db_http_content::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            TaskType::DnsQuery | TaskType::DnsQueryDoh => {
                return db_dns::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            TaskType::Bandwidth => {
                return db_bandwidth::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
            #[cfg(feature = "sql-tasks")]
            TaskType::SqlQuery => {
                return db_sql::generate_aggregated_metrics(
                    &conn,
                    task_name,
                    period_start,
                    period_end,
                );
            }
        }

        Ok(None)
    }

    /// Clean up old data from the database based on the retention policy.
    pub async fn cleanup_old_data(&mut self, retention_days: u32) -> Result<()> {
        let cutoff_time = (current_timestamp() - (retention_days as u64 * 24 * 60 * 60)) as i64;
        info!(
            "Cleaning up data older than {} days (before timestamp: {})",
            retention_days, cutoff_time
        );

        let conn = self.get_connection()?;

        let (raw_ping, agg_ping) = db_ping::cleanup_old_data(conn, cutoff_time)?;
        let (raw_tcp, agg_tcp) = db_tcp::cleanup_old_data(conn, cutoff_time)?;
        let (raw_http, agg_http) = db_http::cleanup_old_data(conn, cutoff_time)?;
        let (raw_tls, agg_tls) = db_tls::cleanup_old_data(conn, cutoff_time)?;
        let (raw_dns, agg_dns) = db_dns::cleanup_old_data(conn, cutoff_time)?;
        let (raw_bandwidth, agg_bandwidth) = db_bandwidth::cleanup_old_data(conn, cutoff_time)?;
        let (raw_http_content, agg_http_content) =
            db_http_content::cleanup_old_data(conn, cutoff_time)?;

        #[cfg(feature = "sql-tasks")]
        let (raw_sql, agg_sql) = db_sql::cleanup_old_data(conn, cutoff_time)?;
        #[cfg(not(feature = "sql-tasks"))]
        let (raw_sql, agg_sql) = (0, 0);

        let total_raw_deleted = raw_ping
            + raw_tcp
            + raw_http
            + raw_tls
            + raw_dns
            + raw_bandwidth
            + raw_http_content
            + raw_sql;
        let total_agg_deleted = agg_ping
            + agg_tcp
            + agg_http
            + agg_tls
            + agg_dns
            + agg_bandwidth
            + agg_http_content
            + agg_sql;

        info!(
            "Cleanup complete: {} raw metrics, {} aggregated metrics deleted",
            total_raw_deleted, total_agg_deleted
        );

        // `VACUUM` rebuilds the database file, repacking it into a smaller,
        // more efficient structure. This is useful after deleting a lot of data.
        conn.execute("VACUUM", [])?;
        debug!("Database vacuum complete");

        // Checkpoint WAL to merge it back into main database file
        // TRUNCATE mode resets the WAL file size to zero after successful checkpoint
        // This is critical to prevent unbounded WAL file growth over time
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        debug!("WAL checkpoint (TRUNCATE) complete after cleanup");

        Ok(())
    }

    /// Close the database connection gracefully.
    pub async fn close(&mut self) {
        if let Some(conn) = self.connection.take() {
            // `close` can return an error, which should be logged.
            if let Err(e) = conn.close() {
                warn!("Error closing database connection: {:?}", e);
            } else {
                debug!("Database connection closed");
            }
        }
    }

    /// Checkpoint WAL to prevent unbounded growth
    ///
    /// This method should be called periodically (e.g., hourly) to ensure
    /// the WAL file doesn't grow too large. The TRUNCATE mode ensures the
    /// WAL file is reset to zero size after successful checkpoint.
    ///
    /// # Returns
    /// Number of WAL frames (pages) that were checkpointed
    pub async fn checkpoint_wal(&mut self) -> Result<i64> {
        let conn = self.get_connection()?;

        // Perform checkpoint with TRUNCATE mode
        // Returns: (busy, log, checkpointed)
        // - busy: number of frames not checkpointed due to locks
        // - log: total number of frames in WAL
        // - checkpointed: number of frames checkpointed
        let (busy, log_frames, checkpointed): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

        if busy > 0 {
            debug!(
                "WAL checkpoint: {} frames checkpointed, {} busy, {} total in log",
                checkpointed, busy, log_frames
            );
        } else {
            debug!(
                "WAL checkpoint complete: {} frames checkpointed, WAL truncated",
                checkpointed
            );
        }

        Ok(checkpointed)
    }

    // ========== Queue Management Functions ==========

    /// Queue an aggregated metric for sending to the server
    /// Returns the row ID from the aggregated metrics table
    pub async fn enqueue_metric_for_send(
        &mut self,
        metric: &AggregatedMetrics,
        metric_row_id: i64,
    ) -> Result<()> {
        let conn = self.get_connection()?;
        db_queue::enqueue_metric_for_send(conn, metric, metric_row_id)
    }

    /// Store aggregated metrics and automatically enqueue for sending
    /// This is the new unified method that replaces the old store_aggregated_metrics
    pub async fn store_and_enqueue_aggregated_metrics(
        &mut self,
        metrics: &AggregatedMetrics,
    ) -> Result<i64> {
        // First, store the metric in the appropriate table
        let conn = self.get_connection()?;
        let row_id = match &metrics.data {
            AggregatedMetricData::Ping(ping_data) => {
                db_ping::store_aggregated_metric(&conn, metrics, ping_data)?
            }
            AggregatedMetricData::Tcp(tcp_data) => {
                db_tcp::store_aggregated_metric(&conn, metrics, tcp_data)?
            }
            AggregatedMetricData::HttpGet(http_data) => {
                db_http::store_aggregated_metric(&conn, metrics, http_data)?
            }
            AggregatedMetricData::TlsHandshake(tls_data) => {
                db_tls::store_aggregated_metric(&conn, metrics, tls_data)?
            }
            AggregatedMetricData::HttpContent(http_content_data) => {
                db_http_content::store_aggregated_metric(&conn, metrics, http_content_data)?
            }
            AggregatedMetricData::DnsQuery(dns_data) => {
                db_dns::store_aggregated_metric(&conn, metrics, dns_data)?
            }
            AggregatedMetricData::Bandwidth(bandwidth_data) => {
                db_bandwidth::store_aggregated_metric(&conn, metrics, bandwidth_data)?
            }
            #[cfg(feature = "sql-tasks")]
            AggregatedMetricData::SqlQuery(sql_data) => {
                db_sql::store_aggregated_metric(&conn, metrics, sql_data)?
            }
        };

        // Now enqueue it for sending
        self.enqueue_metric_for_send(metrics, row_id).await?;

        Ok(row_id)
    }

    /// Get next batch of metrics to send, respecting exponential backoff
    pub async fn get_metrics_to_send(&mut self, batch_size: usize) -> Result<Vec<QueuedMetric>> {
        let conn = self.get_connection()?;
        db_queue::get_queued_metrics(conn, batch_size)
    }

    /// Mark metrics as being sent (status = 'sending')
    pub async fn mark_as_sending(&mut self, queue_ids: &[i64]) -> Result<()> {
        let conn = self.get_connection()?;
        db_queue::mark_as_sending(conn, queue_ids)
    }

    /// Mark metrics as successfully sent
    pub async fn mark_as_sent(&mut self, queue_ids: &[i64]) -> Result<()> {
        let conn = self.get_connection()?;
        db_queue::mark_as_sent(conn, queue_ids)
    }

    /// Mark a send attempt as failed with exponential backoff
    pub async fn mark_as_failed(
        &mut self,
        queue_id: i64,
        error_msg: &str,
        max_retries: i32,
    ) -> Result<()> {
        let conn = self.get_connection()?;
        db_queue::mark_as_failed(conn, queue_id, error_msg, max_retries)
    }

    /// Clean up successfully sent queue entries
    pub async fn cleanup_sent_queue_entries(&mut self, older_than_hours: i64) -> Result<usize> {
        let conn = self.get_connection()?;
        db_queue::cleanup_sent_queue_entries(conn, older_than_hours)
    }

    /// Get queue statistics for monitoring
    pub async fn get_queue_stats(&mut self) -> Result<QueueStats> {
        let conn = self.get_connection()?;
        db_queue::get_queue_stats(conn)
    }
}

/// A struct to hold database statistics.
// #[derive(Debug, Clone)]
// #[allow(dead_code)]
// pub struct DatabaseStats {
//     pub raw_metrics_count: u64,
//     pub aggregated_metrics_count: u64,
//     pub database_size_bytes: u64,
// }

/// A helper function to get the current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// Unit tests for the database module.
#[cfg(test)]
mod tests {
    use super::*;
    use shared::config::TaskType;
    use shared::metrics::{MetricData, RawMetricData, RawPingMetric};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_database_creation() {
        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();

        let result = db.initialize().await;
        assert!(result.is_ok());

        // Verify that the database file was actually created on disk.
        assert!(temp_dir.path().join(DATABASE_FILE).exists());
    }

    #[tokio::test]
    async fn test_store_raw_ping_metric() {
        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        let metric = MetricData::new(
            "test_ping".to_string(),
            TaskType::Ping,
            RawMetricData::Ping(RawPingMetric {
                rtt_ms: Some(15.5),
                success: true,
                error: None,
                ip_address: "8.8.8.8".to_string(),
                domain: None,
                target_id: None,
            }),
        );

        let result = db.store_raw_metric(&metric).await;
        assert!(result.is_ok());
        // The returned row ID should be greater than 0.
        assert!(result.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_generate_ping_aggregated_metrics() {
        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        // Store some test ping metrics
        let metrics = vec![
            MetricData::new(
                "test_ping".to_string(),
                TaskType::Ping,
                RawMetricData::Ping(RawPingMetric {
                    rtt_ms: Some(10.0),
                    success: true,
                    error: None,
                    ip_address: "8.8.8.8".to_string(),
                    domain: Some("google.com".to_string()),
                    target_id: None,
                }),
            ),
            MetricData::new(
                "test_ping".to_string(),
                TaskType::Ping,
                RawMetricData::Ping(RawPingMetric {
                    rtt_ms: Some(20.0),
                    success: true,
                    error: None,
                    ip_address: "8.8.8.8".to_string(),
                    domain: Some("google.com".to_string()),
                    target_id: None,
                }),
            ),
            MetricData::new(
                "test_ping".to_string(),
                TaskType::Ping,
                RawMetricData::Ping(RawPingMetric {
                    rtt_ms: None,
                    success: false,
                    error: Some("Timeout".to_string()),
                    ip_address: "8.8.8.8".to_string(),
                    domain: Some("google.com".to_string()),
                    target_id: None,
                }),
            ),
        ];

        for metric in metrics {
            db.store_raw_metric(&metric).await.unwrap();
        }

        // Generate aggregated metrics
        let now = current_timestamp();
        let period_start = now - 60; // 1 minute ago to ensure we include the stored metrics
        let period_end = now + 60; // 1 minute in the future

        let aggregated = db
            .generate_aggregated_metrics("test_ping", &TaskType::Ping, period_start, period_end)
            .await
            .unwrap();

        assert!(aggregated.is_some());
        let agg = aggregated.unwrap();
        assert_eq!(agg.task_name, "test_ping");
        assert_eq!(agg.sample_count, 3);

        if let AggregatedMetricData::Ping(ping_data) = agg.data {
            assert_eq!(ping_data.successful_pings, 2);
            assert_eq!(ping_data.failed_pings, 1);
            assert_eq!(ping_data.avg_latency_ms, 15.0); // (10 + 20) / 2
            assert!((ping_data.packet_loss_percent - 33.333333333333336).abs() < 0.0001);
        // 1 failed out of 3, with floating point tolerance
        } else {
            panic!("Expected ping aggregated data");
        }
    }

    #[tokio::test]
    async fn test_store_raw_http_metric() {
        use shared::metrics::RawHttpMetric;

        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        let metric = MetricData::new(
            "test_http".to_string(),
            TaskType::HttpGet,
            RawMetricData::HttpGet(RawHttpMetric {
                status_code: Some(200),
                tcp_timing_ms: Some(15.2),
                tls_timing_ms: Some(20.3),
                ttfb_timing_ms: Some(50.1),
                content_download_timing_ms: Some(100.5),
                total_time_ms: Some(186.1),
                success: true,
                error: None,
                ssl_valid: Some(true),
                ssl_cert_days_until_expiry: Some(90),
                target_id: None,
            }),
        );

        let result = db.store_raw_metric(&metric).await;
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_store_raw_http_content_metric() {
        use shared::metrics::RawHttpContentMetric;

        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        let metric = MetricData::new(
            "test_http_content".to_string(),
            TaskType::HttpContent,
            RawMetricData::HttpContent(RawHttpContentMetric {
                status_code: Some(200),
                total_time_ms: Some(150.5),
                total_size: Some(1024),
                regexp_match: Some(true),
                success: true,
                error: None,
                target_id: None,
            }),
        );

        let result = db.store_raw_metric(&metric).await;
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_store_raw_dns_metric() {
        use shared::metrics::RawDnsMetric;

        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        let resolved_addresses = vec!["142.250.185.46".to_string()];

        let metric = MetricData::new(
            "test_dns".to_string(),
            TaskType::DnsQuery,
            RawMetricData::DnsQuery(RawDnsMetric {
                query_time_ms: Some(25.3),
                resolved_addresses: Some(resolved_addresses.clone()),
                record_count: Some(1),
                domain_queried: "google.com".to_string(),
                success: true,
                error: None,
                expected_ip: None,
                resolved_ip: Some("142.250.185.46".to_string()),
                correct_resolution: true,
                target_id: None,
            }),
        );

        let result = db.store_raw_metric(&metric).await;
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_store_raw_bandwidth_metric() {
        use shared::metrics::RawBandwidthMetric;

        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        let metric = MetricData::new(
            "test_bandwidth".to_string(),
            TaskType::Bandwidth,
            RawMetricData::Bandwidth(RawBandwidthMetric {
                bandwidth_mbps: Some(100.5),
                duration_ms: Some(5000.0),
                bytes_downloaded: Some(62_500_000),
                success: true,
                error: None,
                target_id: None,
            }),
        );

        let result = db.store_raw_metric(&metric).await;
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[tokio::test]
    #[cfg(feature = "sql-tasks")]
    async fn test_store_raw_sql_query_metric() {
        use shared::metrics::RawSqlQueryMetric;

        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        let metric = MetricData::new(
            "test_sql".to_string(),
            TaskType::SqlQuery,
            RawMetricData::SqlQuery(RawSqlQueryMetric {
                total_time_ms: Some(125.7),
                row_count: Some(42),
                success: true,
                error: None,
                target_id: None,
            }),
        );

        let result = db.store_raw_metric(&metric).await;
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_generate_http_aggregated_metrics() {
        use shared::metrics::RawHttpMetric;

        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        // Store some test HTTP metrics
        for i in 0..3 {
            let metric = MetricData::new(
                "test_http".to_string(),
                TaskType::HttpGet,
                RawMetricData::HttpGet(RawHttpMetric {
                    status_code: Some(200),
                    tcp_timing_ms: Some(15.0 + i as f64),
                    tls_timing_ms: Some(20.0),
                    ttfb_timing_ms: Some(50.0),
                    content_download_timing_ms: Some(100.0),
                    total_time_ms: Some(185.0 + i as f64),
                    success: true,
                    error: None,
                    ssl_valid: Some(true),
                    ssl_cert_days_until_expiry: Some(90),
                    target_id: None,
                }),
            );
            db.store_raw_metric(&metric).await.unwrap();
        }

        let now = current_timestamp();
        let aggregated = db
            .generate_aggregated_metrics("test_http", &TaskType::HttpGet, now - 60, now + 60)
            .await
            .unwrap();

        assert!(aggregated.is_some());
        let agg = aggregated.unwrap();
        assert_eq!(agg.sample_count, 3);

        if let AggregatedMetricData::HttpGet(http_data) = agg.data {
            assert_eq!(http_data.successful_requests, 3);
            assert_eq!(http_data.failed_requests, 0);
            assert_eq!(http_data.success_rate_percent, 100.0);
            // Average of 185.0, 186.0, 187.0 = 186.0
            assert!((http_data.avg_total_time_ms - 186.0).abs() < 0.1);
        } else {
            panic!("Expected HTTP aggregated data");
        }
    }

    #[tokio::test]
    async fn test_cleanup_old_data() {
        let temp_dir = TempDir::new().unwrap();
        let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
        db.initialize().await.unwrap();

        // Calculate timestamps: one old (35 days ago) and one recent (1 day ago)
        let now = current_timestamp();
        let old_timestamp = now - (35 * 24 * 60 * 60); // 35 days ago
        let recent_timestamp = now - (24 * 60 * 60); // 1 day ago

        // Insert test data and verify before cleanup
        {
            let conn = db.get_connection().unwrap();
            conn.execute(
            r#"
            INSERT INTO raw_metric_ping (task_name, timestamp, rtt_ms, success, error, ip_address, domain, target_id)
            VALUES ('old_ping', ?1, 15.5, 1, NULL, '8.8.8.8', NULL, NULL)
            "#,
            params![old_timestamp as i64],
        ).unwrap();

            // Insert recent metric directly via SQL (1 day old)
            conn.execute(
            r#"
            INSERT INTO raw_metric_ping (task_name, timestamp, rtt_ms, success, error, ip_address, domain, target_id)
            VALUES ('recent_ping', ?1, 20.0, 1, NULL, '1.1.1.1', NULL, NULL)
            "#,
            params![recent_timestamp as i64],
        ).unwrap();

            // Insert old aggregated metric directly via SQL
            let period_start = old_timestamp - 60;
            let period_end = old_timestamp;
            conn.execute(
            r#"
            INSERT INTO agg_metric_ping (task_name, period_start, period_end, sample_count, avg_latency_ms, max_latency_ms, min_latency_ms, packet_loss_percent, successful_pings, failed_pings, domain, target_id)
            VALUES ('old_ping', ?1, ?2, 1, 15.5, 15.5, 15.5, 0.0, 1, 0, NULL, NULL)
            "#,
            params![period_start as i64, period_end as i64],
        ).unwrap();

            // Verify old entries exist via SQL
            let old_raw_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM raw_metric_ping WHERE timestamp < ?1",
                    params![(now - 30 * 24 * 60 * 60) as i64],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                old_raw_count, 1,
                "Should have 1 old raw metric before cleanup"
            );

            let old_agg_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agg_metric_ping WHERE period_end < ?1",
                    params![(now - 30 * 24 * 60 * 60) as i64],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                old_agg_count, 1,
                "Should have 1 old aggregated metric before cleanup"
            );

            let total_raw_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM raw_metric_ping", [], |row| row.get(0))
                .unwrap();
            assert_eq!(
                total_raw_count, 2,
                "Should have 2 total raw metrics before cleanup"
            );
        } // Drop connection before cleanup

        // Run cleanup with 30 day retention
        db.cleanup_old_data(30).await.unwrap();

        // Verify old entries were deleted via SQL
        {
            let conn = db.get_connection().unwrap();
            let old_raw_count_after: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM raw_metric_ping WHERE timestamp < ?1",
                    params![(now - 30 * 24 * 60 * 60) as i64],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                old_raw_count_after, 0,
                "Should have 0 old raw metrics after cleanup"
            );

            let old_agg_count_after: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agg_metric_ping WHERE period_end < ?1",
                    params![(now - 30 * 24 * 60 * 60) as i64],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                old_agg_count_after, 0,
                "Should have 0 old aggregated metrics after cleanup"
            );

            // Verify recent metric still exists
            let recent_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM raw_metric_ping WHERE timestamp >= ?1",
                    params![(now - 30 * 24 * 60 * 60) as i64],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                recent_count, 1,
                "Should still have 1 recent metric after cleanup"
            );
        } // Drop connection
    }
}
