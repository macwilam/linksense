//! Agent health monitoring system
//!
//! This module periodically checks the health of all registered agents by comparing
//! expected vs received metrics, storing results in the database, and exporting
//! problematic agents to a text file.

use crate::config::ConfigManager;
use crate::database::db_agent_health::{store_health_check, AgentHealthCheck};
use crate::database::{AgentInfo, ServerDatabase};
use anyhow::{Context, Result};
use shared::config::ServerConfig;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Health monitor manages periodic agent health checks
pub struct HealthMonitor {
    database: Arc<Mutex<ServerDatabase>>,
    config_manager: Arc<Mutex<ConfigManager>>,
    output_dir: PathBuf,
    server_version: String,
}

/// Health metrics for a single agent
#[derive(Debug, Clone)]
pub struct AgentHealthMetrics {
    pub agent_id: String,
    pub seconds_since_last_push: i64,
    pub expected_entries: i64,
    pub received_entries: i64,
    pub success_ratio: f64,
    pub is_problematic: bool,
    pub agent_version: Option<String>,
    pub version_outdated: bool,
}

impl HealthMonitor {
    /// Creates a new health monitor instance
    ///
    /// # Arguments
    /// * `database` - Shared database handle
    /// * `config_manager` - Shared configuration manager
    /// * `output_dir` - Directory where problematic_agents.txt will be written
    /// * `server_version` - Current server version for comparison with agent versions
    pub fn new(
        database: Arc<Mutex<ServerDatabase>>,
        config_manager: Arc<Mutex<ConfigManager>>,
        output_dir: PathBuf,
        server_version: String,
    ) -> Result<Self> {
        // Ensure output directory exists
        if !output_dir.exists() {
            std::fs::create_dir_all(&output_dir).with_context(|| {
                format!(
                    "Failed to create health monitor output directory: {}",
                    output_dir.display()
                )
            })?;
        }

        Ok(Self {
            database,
            config_manager,
            output_dir,
            server_version,
        })
    }

    /// Performs a health check on all registered agents
    ///
    /// Returns the number of problematic agents found
    pub async fn check_all_agents(&self) -> Result<usize> {
        info!("Starting agent health check");

        let config = self.get_server_config().await?;
        let check_period_seconds = config.health_check_interval_seconds;
        let threshold = config.health_check_success_ratio_threshold;

        // Exclude the current incomplete minute (agents aggregate every 60 seconds)
        // The most recent minute hasn't been aggregated and sent yet
        const AGGREGATION_WINDOW_SECONDS: u64 = 60;
        let current_time = current_timestamp() - AGGREGATION_WINDOW_SECONDS;
        let period_start = current_time - check_period_seconds;

        // Get all registered agents
        let agents = {
            let mut db = self.database.lock().await;
            db.get_all_agents()
                .await
                .context("Failed to retrieve agents list")?
        };

        if agents.is_empty() {
            info!("No agents registered, skipping health check");
            return Ok(0);
        }

        info!("Checking health for {} agents", agents.len());

        let mut problematic_count = 0;
        let mut health_checks = Vec::new();

        for agent in agents {
            match self
                .calculate_health_metrics(&agent, period_start, current_time, threshold)
                .await
            {
                Ok(metrics) => {
                    if metrics.is_problematic {
                        problematic_count += 1;
                        warn!(
                            "Agent {} is problematic: ratio={:.2}, expected={}, received={}",
                            metrics.agent_id,
                            metrics.success_ratio,
                            metrics.expected_entries,
                            metrics.received_entries
                        );
                    } else {
                        debug!(
                            "Agent {} is healthy: ratio={:.2}",
                            metrics.agent_id, metrics.success_ratio
                        );
                    }

                    health_checks.push(AgentHealthCheck {
                        agent_id: metrics.agent_id,
                        check_timestamp: current_time as i64,
                        period_start: period_start as i64,
                        period_end: current_time as i64,
                        seconds_since_last_push: metrics.seconds_since_last_push,
                        expected_entries: metrics.expected_entries,
                        received_entries: metrics.received_entries,
                        success_ratio: metrics.success_ratio,
                        is_problematic: metrics.is_problematic,
                    });
                }
                Err(e) => {
                    error!(
                        "Failed to calculate health metrics for agent {}: {}",
                        agent.agent_id, e
                    );
                }
            }
        }

        // Store health check results in database
        if !health_checks.is_empty() {
            let mut db = self.database.lock().await;
            let conn = db.get_connection()?;
            let tx = conn.transaction()?;

            for check in &health_checks {
                store_health_check(&tx, check)?;
            }

            tx.commit()?;
            info!("Stored {} health check results", health_checks.len());
        }

        // Export problematic agents to file
        if problematic_count > 0 {
            self.export_problematic_agents().await?;
            info!("Exported {} problematic agents to file", problematic_count);
        } else {
            // Clear the problematic agents file if no issues
            self.clear_problematic_agents_file().await?;
            info!("No problematic agents found, cleared report file");
        }

        info!(
            "Health check complete: {} agents checked, {} problematic",
            health_checks.len(),
            problematic_count
        );

        Ok(problematic_count)
    }

    /// Calculates health metrics for a single agent
    async fn calculate_health_metrics(
        &self,
        agent: &AgentInfo,
        period_start: u64,
        period_end: u64,
        threshold: f64,
    ) -> Result<AgentHealthMetrics> {
        let current_time = current_timestamp();
        let seconds_since_last_push = (current_time - agent.last_seen) as i64;

        // Calculate expected entries based on agent's task configuration
        let expected_entries = self
            .calculate_expected_entries(&agent.agent_id, period_start, period_end)
            .await?;

        // Calculate received entries from database
        let received_entries = self
            .calculate_received_entries(&agent.agent_id, period_start, period_end)
            .await?;

        // Calculate success ratio
        let success_ratio = if expected_entries > 0 {
            received_entries as f64 / expected_entries as f64
        } else {
            1.0 // No tasks expected = healthy by definition
        };

        // Check if agent version is outdated
        let version_outdated = self.is_agent_version_outdated(agent);

        // Agent is problematic if success ratio is below threshold OR version is outdated
        let is_problematic = success_ratio < threshold || version_outdated;

        Ok(AgentHealthMetrics {
            agent_id: agent.agent_id.clone(),
            seconds_since_last_push,
            expected_entries,
            received_entries,
            success_ratio,
            is_problematic,
            agent_version: agent.agent_version.clone(),
            version_outdated,
        })
    }

    /// Checks if an agent's version is outdated compared to server version
    ///
    /// Returns true if agent version is lower than server version, or if agent version is unknown
    fn is_agent_version_outdated(&self, agent: &AgentInfo) -> bool {
        // If agent has no version info, consider it outdated
        let agent_version_str = match &agent.agent_version {
            Some(v) => v,
            None => return true,
        };

        // Parse versions using semver-like comparison (simplified)
        match self.compare_versions(&self.server_version, agent_version_str) {
            Ok(ordering) => ordering == std::cmp::Ordering::Greater, // Server > Agent = outdated
            Err(_) => true, // If we can't parse, consider it outdated for safety
        }
    }

    /// Compare two version strings (semver format: major.minor.patch)
    ///
    /// Returns Ok(Ordering) where Greater means v1 > v2, Less means v1 < v2, Equal means v1 == v2
    fn compare_versions(&self, v1: &str, v2: &str) -> Result<std::cmp::Ordering> {
        let parse_version = |v: &str| -> Result<(u32, u32, u32)> {
            let parts: Vec<&str> = v.split('.').collect();
            if parts.len() != 3 {
                anyhow::bail!("Invalid version format: {}", v);
            }
            let major = parts[0].parse::<u32>()?;
            let minor = parts[1].parse::<u32>()?;
            let patch = parts[2].parse::<u32>()?;
            Ok((major, minor, patch))
        };

        let (major1, minor1, patch1) = parse_version(v1)?;
        let (major2, minor2, patch2) = parse_version(v2)?;

        Ok(
            match (
                major1.cmp(&major2),
                minor1.cmp(&minor2),
                patch1.cmp(&patch2),
            ) {
                (std::cmp::Ordering::Equal, std::cmp::Ordering::Equal, patch_cmp) => patch_cmp,
                (std::cmp::Ordering::Equal, minor_cmp, _) => minor_cmp,
                (major_cmp, _, _) => major_cmp,
            },
        )
    }

    /// Calculates the expected number of metric entries based on agent's task configuration
    ///
    /// Note: Agents aggregate metrics every 60 seconds (1 minute) and send those aggregated
    /// entries to the server. Each task produces one aggregated entry per minute if it runs
    /// at least once during that minute.
    async fn calculate_expected_entries(
        &self,
        agent_id: &str,
        period_start: u64,
        period_end: u64,
    ) -> Result<i64> {
        // Agents aggregate metrics every 60 seconds
        const AGGREGATION_WINDOW_SECONDS: u64 = 60;

        let config_manager = self.config_manager.lock().await;

        // Get agent's tasks configuration
        let tasks_toml = config_manager
            .get_agent_tasks_config(agent_id)
            .context("Failed to get agent tasks configuration")?;

        // Parse tasks configuration
        let tasks_config: shared::config::TasksConfig =
            toml::from_str(&tasks_toml).context("Failed to parse agent tasks configuration")?;

        let period_duration = period_end - period_start;

        // Calculate how many aggregation windows fit in the health check period
        let num_aggregation_windows = period_duration / AGGREGATION_WINDOW_SECONDS;

        // Each task produces one aggregated entry per minute (if it runs at least once in that minute)
        // Tasks that run faster than once per minute still only produce one aggregated entry per minute
        // Tasks that run slower than once per minute may not produce an entry in every minute
        let mut total_expected = 0i64;

        for task in &tasks_config.tasks {
            let schedule_seconds = task.schedule_seconds as u64;

            if schedule_seconds == 0 {
                continue; // Skip invalid tasks
            }

            // For tasks that run more frequently than the aggregation window (< 60s):
            // They produce 1 aggregated entry per minute
            // For tasks that run less frequently (>= 60s):
            // They produce fewer entries based on their schedule
            let expected_entries_for_task = if schedule_seconds < AGGREGATION_WINDOW_SECONDS {
                // Fast tasks: one aggregated entry per minute
                num_aggregation_windows
            } else {
                // Slow tasks: one entry per execution
                period_duration / schedule_seconds
            };

            total_expected += expected_entries_for_task as i64;

            debug!(
                "Agent {}, task '{}': schedule={}s, expected_entries={} in {}s period ({} aggregation windows)",
                agent_id, task.name, schedule_seconds, expected_entries_for_task, period_duration, num_aggregation_windows
            );
        }

        debug!(
            "Agent {}: total expected entries = {} for period {}s",
            agent_id, total_expected, period_duration
        );

        Ok(total_expected)
    }

    /// Calculates the actual number of metric entries received from an agent
    async fn calculate_received_entries(
        &self,
        agent_id: &str,
        period_start: u64,
        period_end: u64,
    ) -> Result<i64> {
        let mut db = self.database.lock().await;
        let conn = db.get_connection()?;

        let period_start_i64 = period_start as i64;
        let period_end_i64 = period_end as i64;

        // Use a read transaction to ensure consistent snapshot across all queries
        // This is critical in WAL mode to see all committed data
        let tx = conn.transaction()?;

        // Count entries from all aggregated metrics tables
        let count_ping: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_ping WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        let count_tcp: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_tcp WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        let count_http: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_http WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        let count_tls: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_tls WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        let count_http_content: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_http_content WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        let count_dns: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_dns WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        let count_bandwidth: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_bandwidth WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;

        #[cfg(feature = "sql-tasks")]
        let count_sql: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agg_metric_sql_query WHERE agent_id = ?1 AND period_start >= ?2 AND period_start < ?3",
            rusqlite::params![agent_id, period_start_i64, period_end_i64],
            |row| row.get(0),
        )?;
        #[cfg(not(feature = "sql-tasks"))]
        let count_sql: i64 = 0;

        // Commit the read transaction (this is a no-op for reads but releases locks)
        tx.commit()?;

        let total = count_ping
            + count_tcp
            + count_http
            + count_tls
            + count_http_content
            + count_dns
            + count_bandwidth
            + count_sql;

        debug!(
            "Agent {}: received entries = {} (ping={}, tcp={}, http={}, tls={}, http_content={}, dns={}, bandwidth={}, sql={})",
            agent_id, total, count_ping, count_tcp, count_http, count_tls, count_http_content, count_dns, count_bandwidth, count_sql
        );

        Ok(total)
    }

    /// Exports problematic agents to a text file
    async fn export_problematic_agents(&self) -> Result<()> {
        use std::io::Write;

        let output_path = self.output_dir.join("problematic_agents.txt");

        let problematic = {
            let mut db = self.database.lock().await;
            let conn = db.get_connection()?;
            crate::database::db_agent_health::get_problematic_agents(conn)?
        };

        if problematic.is_empty() {
            // No problematic agents, clear the file
            return self.clear_problematic_agents_file().await;
        }

        // Get agent info including versions
        let agents_info = {
            let mut db = self.database.lock().await;
            db.get_all_agents().await?
        };

        let mut file = std::fs::File::create(&output_path)
            .with_context(|| format!("Failed to create report file: {}", output_path.display()))?;

        // Write header
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        writeln!(file, "Agent Health Report - {}", timestamp)?;
        writeln!(file, "============================================")?;
        writeln!(file, "Server Version: {}", self.server_version)?;
        writeln!(file)?;

        // Write each problematic agent
        for agent in &problematic {
            // Find matching agent info for version
            let agent_info = agents_info.iter().find(|a| a.agent_id == agent.agent_id);
            let agent_version = agent_info
                .and_then(|a| a.agent_version.as_deref())
                .unwrap_or("unknown");

            // Check if version is outdated
            let version_outdated = agent_info
                .map(|a| self.is_agent_version_outdated(a))
                .unwrap_or(true);

            writeln!(file, "agent_id: {}", agent.agent_id)?;
            writeln!(file, "  Agent Version: {}", agent_version)?;
            if version_outdated {
                writeln!(
                    file,
                    "  Version Status: OUTDATED (server: {})",
                    self.server_version
                )?;
            }
            writeln!(
                file,
                "  Last Push: {} seconds ago",
                agent.seconds_since_last_push
            )?;
            writeln!(file, "  Expected Entries: {}", agent.expected_entries)?;
            writeln!(file, "  Received Entries: {}", agent.received_entries)?;
            writeln!(file, "  Success Ratio: {:.2}", agent.success_ratio)?;
            writeln!(file, "  Status: PROBLEMATIC")?;
            writeln!(file)?;
        }

        writeln!(file, "============================================")?;
        writeln!(file, "Total problematic agents: {}", problematic.len())?;

        file.flush()?;

        info!(
            "Exported problematic agents report to: {}",
            output_path.display()
        );
        Ok(())
    }

    /// Clears the problematic agents file
    async fn clear_problematic_agents_file(&self) -> Result<()> {
        use std::io::Write;

        let output_path = self.output_dir.join("problematic_agents.txt");

        let mut file = std::fs::File::create(&output_path)
            .with_context(|| format!("Failed to create report file: {}", output_path.display()))?;

        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        writeln!(file, "Agent Health Report - {}", timestamp)?;
        writeln!(file, "============================================")?;
        writeln!(file)?;
        writeln!(file, "All agents are healthy - no issues detected.")?;
        writeln!(file)?;

        file.flush()?;

        debug!("Cleared problematic agents report file");
        Ok(())
    }

    /// Cleans up old health check data based on retention policy
    pub async fn cleanup_old_health_data(&self, retention_days: u32) -> Result<()> {
        // Use saturating arithmetic to prevent overflow with large retention values
        let retention_seconds = (retention_days as u64)
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60);
        let cutoff_time = current_timestamp().saturating_sub(retention_seconds);

        info!(
            "Cleaning up health check data older than {} days (before timestamp: {})",
            retention_days, cutoff_time
        );

        let deleted = {
            let mut db = self.database.lock().await;
            let conn = db.get_connection()?;
            crate::database::db_agent_health::cleanup_old_data(conn, cutoff_time as i64)?
        };

        info!("Deleted {} old health check records", deleted);
        Ok(())
    }

    /// Helper to get server config
    async fn get_server_config(&self) -> Result<ServerConfig> {
        let config_manager = self.config_manager.lock().await;
        config_manager
            .server_config
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Server configuration not loaded"))
    }
}

/// Helper function to get current Unix timestamp
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ServerDatabase;
    use shared::config::{PingParams, TaskConfig, TaskParams, TaskType, TasksConfig};
    use tempfile::TempDir;

    async fn setup_test_environment() -> (
        Arc<Mutex<ServerDatabase>>,
        Arc<Mutex<ConfigManager>>,
        TempDir,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join("data");
        let config_dir = temp_dir.path().join("configs");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();

        // Create server config
        let server_config_content = format!(
            r#"
listen_address = "127.0.0.1:8787"
api_key = "test-key"
data_retention_days = 30
agent_configs_dir = "{}"
bandwidth_test_size_mb = 10
            "#,
            config_dir.display()
        );
        let server_config_path = temp_dir.path().join("server.toml");
        std::fs::write(&server_config_path, server_config_content).unwrap();

        // Initialize database
        let mut db = ServerDatabase::new(&data_dir).unwrap();
        db.initialize().await.unwrap();

        // Initialize config manager
        let config_manager = ConfigManager::new(server_config_path).unwrap();

        (
            Arc::new(Mutex::new(db)),
            Arc::new(Mutex::new(config_manager)),
            temp_dir,
        )
    }

    #[tokio::test]
    async fn test_health_monitor_creation() {
        let (db, config_manager, temp_dir) = setup_test_environment().await;
        let output_dir = temp_dir.path().join("output");

        let monitor =
            HealthMonitor::new(db, config_manager, output_dir.clone(), "0.7.6".to_string());
        assert!(monitor.is_ok());
        assert!(output_dir.exists());
    }

    #[tokio::test]
    async fn test_check_all_agents_no_agents() {
        let (db, config_manager, temp_dir) = setup_test_environment().await;
        let output_dir = temp_dir.path().join("output");

        let monitor =
            HealthMonitor::new(db, config_manager, output_dir, "0.7.6".to_string()).unwrap();
        let result = monitor.check_all_agents().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0); // No problematic agents
    }

    #[tokio::test]
    async fn test_calculate_expected_entries() {
        let (db, config_manager, temp_dir) = setup_test_environment().await;
        let output_dir = temp_dir.path().join("output");

        // Create agent config with 3 tasks: fast (<60s), exact (60s), slow (>60s)
        let tasks_config = TasksConfig {
            tasks: vec![
                TaskConfig {
                    task_type: TaskType::Ping,
                    schedule_seconds: 30, // Runs every 30s (faster than aggregation window)
                    name: "ping-fast".to_string(),
                    timeout: None,
                    params: TaskParams::Ping(PingParams {
                        host: "8.8.8.8".to_string(),
                        timeout_seconds: 5,
                        target_id: None,
                    }),
                },
                TaskConfig {
                    task_type: TaskType::Ping,
                    schedule_seconds: 60, // Runs every 60s (equals aggregation window)
                    name: "ping-normal".to_string(),
                    timeout: None,
                    params: TaskParams::Ping(PingParams {
                        host: "1.1.1.1".to_string(),
                        timeout_seconds: 5,
                        target_id: None,
                    }),
                },
                TaskConfig {
                    task_type: TaskType::Ping,
                    schedule_seconds: 120, // Runs every 2 minutes (slower than aggregation window)
                    name: "ping-slow".to_string(),
                    timeout: None,
                    params: TaskParams::Ping(PingParams {
                        host: "9.9.9.9".to_string(),
                        timeout_seconds: 5,
                        target_id: None,
                    }),
                },
            ],
        };

        let tasks_toml = toml::to_string(&tasks_config).unwrap();
        let agent_config_path = {
            let cm = config_manager.lock().await;
            let sc = cm.server_config.as_ref().unwrap();
            PathBuf::from(&sc.agent_configs_dir).join("test-agent.toml")
        };
        std::fs::write(&agent_config_path, tasks_toml).unwrap();

        let monitor =
            HealthMonitor::new(db, config_manager, output_dir, "0.7.6".to_string()).unwrap();

        // For a 5-minute (300s) period:
        // Number of aggregation windows: 300 / 60 = 5
        //
        // Task 1 (30s interval, faster than 60s):
        //   - Produces 1 aggregated entry per minute = 5 entries
        // Task 2 (60s interval, equals 60s):
        //   - Produces 1 aggregated entry per minute = 5 entries
        // Task 3 (120s interval, slower than 60s):
        //   - Runs every 2 minutes, so produces: 300/120 = 2 entries (floor)
        // Total: 5 + 5 + 2 = 12 expected entries
        let current_time = current_timestamp();
        let period_start = current_time - 300;
        let expected = monitor
            .calculate_expected_entries("test-agent", period_start, current_time)
            .await
            .unwrap();

        assert_eq!(expected, 12);
    }
}
