//! Database management for the network monitoring central server
//!
//! This module handles SQLite database operations for storing aggregated metrics
//! from agents, managing agent registration, and logging configuration errors.
// The server's database is the central repository for all data collected by the
// monitoring system. It's designed to be the single source of truth. SQLite is
// chosen for its simplicity and ease of deployment, making the server self-contained.
// For larger-scale deployments, this module could be adapted to use a more
// powerful database like PostgreSQL.

// Task-specific database modules
pub mod db_agent_health;
mod db_bandwidth;
mod db_dns;
mod db_http;
mod db_http_content;
mod db_ping;
#[cfg(feature = "sql-tasks")]
mod db_sql;
mod db_tcp;
mod db_tls;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use shared::metrics::{AggregatedMetricData, AggregatedMetrics};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// The default name for the server's database file.
const DATABASE_FILE: &str = "server_metrics.db";

/// Manages the SQLite database for the server.
/// This struct encapsulates the database connection and all related operations,
/// providing a clean, high-level API to the rest of the server application.
pub struct ServerDatabase {
    /// The path to the SQLite database file.
    db_path: PathBuf,
    /// The active database connection. It's an `Option` to allow for lazy
    /// initialization and handling of connection state.
    pub(crate) connection: Option<Connection>,
}

impl ServerDatabase {
    /// Creates a new `ServerDatabase` manager.
    /// It ensures that the directory for the database file exists.
    pub fn new<P: AsRef<Path>>(data_dir: P) -> Result<Self> {
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
        })
    }

    /// Initializes the database by creating tables and indexes if they don't exist.
    /// This method is idempotent and safe to call on every server startup.
    pub async fn initialize(&mut self) -> Result<()> {
        info!("Initializing server database at {}", self.db_path.display());

        let conn = self.get_connection()?;

        // The `agents` table keeps a record of every agent that has ever contacted the server.
        // It's used for tracking agent status, last seen time, and configuration version.
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS agents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT UNIQUE NOT NULL,
                first_seen INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                last_config_checksum TEXT,
                total_metrics_received INTEGER DEFAULT 0,
                agent_version TEXT,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            )
            "#,
            [],
        )
        .context("Failed to create agents table")?;

        // Create task-specific aggregated metrics tables using submodules
        db_ping::create_table(conn)?;
        db_tcp::create_table(conn)?;
        db_http::create_table(conn)?;
        db_tls::create_table(conn)?;
        db_http_content::create_table(conn)?;
        db_dns::create_table(conn)?;
        db_bandwidth::create_table(conn)?;
        #[cfg(feature = "sql-tasks")]
        db_sql::create_table(conn)?;

        // Create agent health checks table
        db_agent_health::create_table(conn)?;

        // The `config_errors` table is used to log any time an agent reports
        // a problem with its configuration. This is useful for debugging.
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS config_errors (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                timestamp_utc TEXT NOT NULL,
                error_message TEXT NOT NULL,
                received_at INTEGER DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (agent_id) REFERENCES agents (agent_id)
            )
            "#,
            [],
        )
        .context("Failed to create config_errors table")?;

        // Indexes are critical for query performance on aggregated metrics tables.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agents_agent_id ON agents(agent_id)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agents_last_seen ON agents(last_seen)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_config_errors_agent_id ON config_errors(agent_id)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_config_errors_received_at ON config_errors(received_at)",
            [],
        )?;

        info!("Server database initialization complete");
        Ok(())
    }

    /// Lazily gets a mutable reference to the database connection, creating it if needed.
    pub fn get_connection(&mut self) -> Result<&mut Connection> {
        if self.connection.is_none() {
            let conn = Connection::open(&self.db_path)
                .with_context(|| format!("Failed to open database: {}", self.db_path.display()))?;

            // WAL mode is good for concurrency.
            conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
                .context("Failed to enable WAL mode")?;

            // Configure WAL auto-checkpoint to prevent unbounded WAL file growth.
            // Checkpoint automatically when WAL reaches 1000 pages (~4MB).
            conn.query_row("PRAGMA wal_autocheckpoint=1000", [], |_| Ok(()))
                .context("Failed to set WAL auto-checkpoint")?;

            // It's important to enforce foreign key constraints at the database level
            // to maintain data integrity.
            conn.execute("PRAGMA foreign_keys=ON", [])
                .context("Failed to enable foreign key constraints")?;

            // Set a busy timeout to reduce errors in a concurrent environment.
            conn.busy_timeout(std::time::Duration::from_secs(30))
                .context("Failed to set busy timeout")?;

            self.connection = Some(conn);
        }
        Ok(self
            .connection
            .as_mut()
            .expect("Database connection should exist after initialization in get_connection()"))
    }

    /// Registers a new agent or updates the status of an existing one.
    /// This is an "upsert" operation: it updates if the agent exists, or inserts if not.
    pub async fn upsert_agent(
        &mut self,
        agent_id: &str,
        config_checksum: &str,
        agent_version: Option<&str>,
    ) -> Result<()> {
        debug!("Upserting agent: {}", agent_id);

        let conn = self.get_connection()?;
        let current_time = current_timestamp() as i64;

        // First, try to update an existing record. This is often faster than
        // checking for existence first.
        let updated_rows = conn.execute(
            r#"
            UPDATE agents
            SET last_seen = ?1, last_config_checksum = ?2, agent_version = ?3, total_metrics_received = total_metrics_received + 1
            WHERE agent_id = ?4
            "#,
            params![current_time, config_checksum, agent_version, agent_id],
        )?;

        // If `execute` returns 0, no rows were updated, which means the agent is new.
        if updated_rows == 0 {
            conn.execute(
                r#"
                INSERT INTO agents (agent_id, first_seen, last_seen, last_config_checksum, agent_version, total_metrics_received)
                VALUES (?1, ?2, ?3, ?4, ?5, 1)
                "#,
                params![agent_id, current_time, current_time, config_checksum, agent_version],
            ).with_context(|| format!("Failed to insert new agent: {}", agent_id))?;

            info!("Registered new agent: {}", agent_id);
        } else {
            debug!("Updated existing agent: {}", agent_id);
        }

        Ok(())
    }

    /// Stores a batch of aggregated metrics from an agent into task-specific tables.
    /// This operation is performed within a database transaction to ensure that
    /// all metrics in the batch are inserted atomically.
    pub async fn store_metrics(
        &mut self,
        agent_id: &str,
        metrics: &[AggregatedMetrics],
    ) -> Result<()> {
        debug!("Storing {} metrics from agent: {}", metrics.len(), agent_id);

        let conn = self.get_connection()?;
        let tx = conn.transaction()?;

        for metric in metrics {
            match &metric.data {
                AggregatedMetricData::Ping(ping_data) => {
                    db_ping::store_metric(&tx, agent_id, metric, ping_data)?;
                }
                AggregatedMetricData::Tcp(tcp_data) => {
                    db_tcp::store_metric(&tx, agent_id, metric, tcp_data)?;
                }
                AggregatedMetricData::HttpGet(http_data) => {
                    db_http::store_metric(&tx, agent_id, metric, http_data)?;
                }
                AggregatedMetricData::TlsHandshake(tls_data) => {
                    db_tls::store_metric(&tx, agent_id, metric, tls_data)?;
                }
                AggregatedMetricData::HttpContent(http_content_data) => {
                    db_http_content::store_metric(&tx, agent_id, metric, http_content_data)?;
                }
                AggregatedMetricData::DnsQuery(dns_data) => {
                    db_dns::store_metric(&tx, agent_id, metric, dns_data)?;
                }
                AggregatedMetricData::Bandwidth(bandwidth_data) => {
                    db_bandwidth::store_metric(&tx, agent_id, metric, bandwidth_data)?;
                }
                #[cfg(feature = "sql-tasks")]
                AggregatedMetricData::SqlQuery(sql_data) => {
                    db_sql::store_metric(&tx, agent_id, metric, sql_data)?;
                }
            }
        }

        tx.commit()
            .context("Failed to commit metrics transaction")?;

        debug!("Stored {} metrics for agent: {}", metrics.len(), agent_id);
        Ok(())
    }

    /// Logs a configuration error reported by an agent.
    pub async fn log_config_error(
        &mut self,
        agent_id: &str,
        timestamp_utc: &str,
        error_message: &str,
    ) -> Result<()> {
        debug!("Logging config error from agent: {}", agent_id);

        let conn = self.get_connection()?;

        conn.execute(
            r#"
            INSERT INTO config_errors (agent_id, timestamp_utc, error_message)
            VALUES (?1, ?2, ?3)
            "#,
            params![agent_id, timestamp_utc, error_message],
        )
        .with_context(|| format!("Failed to insert config error for agent: {}", agent_id))?;

        warn!(
            agent_id = %agent_id,
            timestamp = %timestamp_utc,
            error = %error_message,
            "Logged configuration error from agent"
        );

        Ok(())
    }

    /// Retrieves information about a specific agent.
    #[allow(dead_code)]
    pub async fn get_agent_info(&mut self, agent_id: &str) -> Result<Option<AgentInfo>> {
        debug!("Querying agent info for: {}", agent_id);

        let conn = self.get_connection()?;

        let mut stmt = conn.prepare(
            r#"
            SELECT agent_id, first_seen, last_seen, last_config_checksum, total_metrics_received, agent_version
            FROM agents
            WHERE agent_id = ?1
            "#,
        )?;

        // `query_row` is used to fetch a single row.
        let result = stmt.query_row(params![agent_id], |row| {
            Ok(AgentInfo {
                agent_id: row.get(0)?,
                first_seen: row.get::<_, i64>(1)? as u64,
                last_seen: row.get::<_, i64>(2)? as u64,
                last_config_checksum: row.get(3)?,
                total_metrics_received: row.get::<_, i64>(4)? as u64,
                agent_version: row.get(5)?,
            })
        });

        // Handle the case where the agent is not found.
        match result {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Retrieves a list of all registered agents.
    #[allow(dead_code)]
    pub async fn get_all_agents(&mut self) -> Result<Vec<AgentInfo>> {
        debug!("Querying all agents");

        let conn = self.get_connection()?;

        let mut stmt = conn.prepare(
            r#"
            SELECT agent_id, first_seen, last_seen, last_config_checksum, total_metrics_received, agent_version
            FROM agents
            ORDER BY last_seen DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(AgentInfo {
                agent_id: row.get(0)?,
                first_seen: row.get::<_, i64>(1)? as u64,
                last_seen: row.get::<_, i64>(2)? as u64,
                last_config_checksum: row.get(3)?,
                total_metrics_received: row.get::<_, i64>(4)? as u64,
                agent_version: row.get(5)?,
            })
        })?;

        let mut agents = Vec::new();
        for row in rows {
            agents.push(row?);
        }

        debug!("Retrieved {} agents", agents.len());
        Ok(agents)
    }

    /// Cleans up old data from the database based on the configured retention period.
    /// This is a critical maintenance task to prevent the database from growing indefinitely.
    pub async fn cleanup_old_data(&mut self, retention_days: u32) -> Result<()> {
        // Use saturating arithmetic to prevent overflow with large retention values
        let retention_seconds = (retention_days as u64)
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60);
        let cutoff_time = current_timestamp().saturating_sub(retention_seconds);
        info!(
            "Cleaning up data older than {} days (before timestamp: {})",
            retention_days, cutoff_time
        );

        let conn = self.get_connection()?;

        // Delete old metrics from task-specific tables using submodules
        let agg_ping_deleted = db_ping::cleanup_old_data(conn, cutoff_time as i64)?;
        let agg_tcp_deleted = db_tcp::cleanup_old_data(conn, cutoff_time as i64)?;
        let agg_http_deleted = db_http::cleanup_old_data(conn, cutoff_time as i64)?;
        let agg_tls_deleted = db_tls::cleanup_old_data(conn, cutoff_time as i64)?;
        let agg_http_content_deleted = db_http_content::cleanup_old_data(conn, cutoff_time as i64)?;
        let agg_dns_deleted = db_dns::cleanup_old_data(conn, cutoff_time as i64)?;
        let agg_bandwidth_deleted = db_bandwidth::cleanup_old_data(conn, cutoff_time as i64)?;

        #[cfg(feature = "sql-tasks")]
        let agg_sql_query_deleted = db_sql::cleanup_old_data(conn, cutoff_time as i64)?;
        #[cfg(not(feature = "sql-tasks"))]
        let agg_sql_query_deleted = 0;

        let total_metrics_deleted = agg_ping_deleted
            + agg_tcp_deleted
            + agg_http_deleted
            + agg_tls_deleted
            + agg_http_content_deleted
            + agg_dns_deleted
            + agg_bandwidth_deleted
            + agg_sql_query_deleted;

        // Delete old config errors.
        let errors_deleted = conn.execute(
            "DELETE FROM config_errors WHERE received_at < ?1",
            params![cutoff_time as i64],
        )?;

        // Optionally, delete records of agents that have been inactive for a long time.
        let agents_deleted = conn.execute(
            "DELETE FROM agents WHERE last_seen < ?1",
            params![cutoff_time as i64],
        )?;

        info!(
            "Cleanup complete: {} metrics, {} config errors, {} agents deleted",
            total_metrics_deleted, errors_deleted, agents_deleted
        );

        // Reclaim disk space after deletion.
        conn.execute("VACUUM", [])?;
        debug!("Database vacuum complete");

        // Checkpoint WAL after VACUUM to merge changes and reset WAL file.
        // TRUNCATE mode ensures WAL file is reset to minimal size.
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        debug!("WAL checkpoint after cleanup complete");

        Ok(())
    }

    /// Performs a WAL checkpoint to merge WAL file changes back into the main database.
    /// This method should be called periodically to prevent unbounded WAL file growth.
    ///
    /// Returns the number of frames checkpointed.
    ///
    /// # WAL Checkpoint Modes
    /// - TRUNCATE: After checkpointing, resets the WAL file to minimal size
    /// - This is the most aggressive mode and prevents WAL accumulation
    ///
    /// # Errors
    /// Returns error if checkpoint operation fails (e.g., database locked)
    ///
    /// # Note
    /// This operation is non-blocking. If the database is busy, it will return
    /// immediately without waiting, and the checkpoint may be incomplete.
    pub async fn checkpoint_wal(&mut self) -> Result<i64> {
        debug!("Performing WAL checkpoint on server database");

        let conn = self.get_connection()?;

        // PRAGMA wal_checkpoint(TRUNCATE) returns (busy, log, checkpointed)
        // - busy: 0 if checkpoint completed, 1 if blocked
        // - log: Number of frames in WAL after checkpoint
        // - checkpointed: Number of frames checkpointed
        let (busy, log_frames, checkpointed): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

        if busy != 0 {
            warn!(
                "WAL checkpoint was blocked (busy). Frames remaining in log: {}",
                log_frames
            );
        } else {
            debug!(
                "WAL checkpoint complete: {} frames checkpointed, {} frames remaining",
                checkpointed, log_frames
            );
        }

        Ok(checkpointed)
    }

    /// Gathers statistics about the database.
    #[allow(dead_code)]
    pub async fn get_stats(&mut self) -> Result<ServerDatabaseStats> {
        let conn = self.get_connection()?;

        // Use a read transaction to ensure consistent snapshot across all queries
        // This is critical in WAL mode to prevent inconsistent counts
        let tx = conn.transaction()?;

        let agent_count: i64 = tx.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;

        // Count task-specific metrics
        let agg_ping_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_ping", [], |row| row.get(0))?;
        let agg_tcp_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_tcp", [], |row| row.get(0))?;
        let agg_http_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_http", [], |row| row.get(0))?;
        let agg_tls_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_tls", [], |row| row.get(0))?;
        let agg_http_content_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_http_content", [], |row| {
                row.get(0)
            })?;
        let agg_dns_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_dns", [], |row| row.get(0))?;
        let agg_bandwidth_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_bandwidth", [], |row| {
                row.get(0)
            })?;

        #[cfg(feature = "sql-tasks")]
        let agg_sql_query_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM agg_metric_sql_query", [], |row| {
                row.get(0)
            })?;
        #[cfg(not(feature = "sql-tasks"))]
        let agg_sql_query_count: i64 = 0;

        let total_metrics = agg_ping_count
            + agg_tcp_count
            + agg_http_count
            + agg_tls_count
            + agg_http_content_count
            + agg_dns_count
            + agg_bandwidth_count
            + agg_sql_query_count;

        let config_errors_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM config_errors", [], |row| row.get(0))?;

        // Commit the read transaction (releases locks)
        tx.commit()?;

        let db_size = std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(ServerDatabaseStats {
            agent_count: agent_count as u64,
            metrics_count: total_metrics as u64,
            config_errors_count: config_errors_count as u64,
            database_size_bytes: db_size,
        })
    }

    /// Closes the database connection.
    pub async fn close(&mut self) {
        if let Some(conn) = self.connection.take() {
            if let Err(e) = conn.close() {
                warn!("Error closing database connection: {:?}", e);
            } else {
                debug!("Database connection closed");
            }
        }
    }
}

/// A struct to hold information about a registered agent.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AgentInfo {
    pub agent_id: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub last_config_checksum: Option<String>,
    pub total_metrics_received: u64,
    pub agent_version: Option<String>,
}

/// A struct to hold statistics about the server's database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ServerDatabaseStats {
    pub agent_count: u64,
    pub metrics_count: u64,
    pub config_errors_count: u64,
    pub database_size_bytes: u64,
}

/// Helper function to get the current Unix timestamp.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
