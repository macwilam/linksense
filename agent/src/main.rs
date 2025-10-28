//! Network Monitoring Agent
//!
//! The agent is a lightweight service that executes monitoring tasks and reports
//! metrics to a central server. It runs scheduled tasks like ICMP ping, HTTP GET,
//! DNS queries, and bandwidth tests.
// This is the main entry point for the agent application. It is responsible for:
// - Initializing logging and configuration.
// - Setting up the main `Agent` struct.
// - Handling command-line arguments.
// - Managing the application's lifecycle, including graceful shutdown.

// Use jemalloc as the global allocator for better performance
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use anyhow::{Context, Result};
use base64::Engine;
use clap::Parser;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

// The agent is organized into several modules, each with a distinct responsibility.
mod config;
mod database;
mod scheduler;
mod task_bandwidth;
mod task_dns;
mod task_http;
mod task_http_content;
mod task_ping;
#[cfg(feature = "sql-tasks")]
mod task_sql;
mod task_tcp;
mod task_tls;
mod tasks;
use std::sync::Arc;
use tokio::sync::RwLock;

use config::ConfigManager;
use database::AgentDatabase;
use scheduler::TaskScheduler;
use shared::api::{
    endpoints, headers, ConfigUploadRequest, ConfigUploadResponse, MetricsRequest, MetricsResponse,
};
use shared::metrics::AggregatedMetrics;

/// Command-line arguments for the agent
#[derive(Parser, Debug)]
#[command(name = "agent")]
#[command(about = "Network monitoring agent that executes tasks and reports metrics", long_about = None)]
struct CliArgs {
    /// Path to the configuration directory containing agent.toml and tasks.toml
    #[arg(value_name = "CONFIG_DIR")]
    config_dir: PathBuf,

    /// Override the agent ID from config file
    #[arg(long = "agent-id", value_name = "ID")]
    agent_id: Option<String>,

    /// Override the central server URL from config file
    #[arg(long = "server-url", value_name = "URL")]
    server_url: Option<String>,

    /// Override the API key from config file
    #[arg(long = "api-key", value_name = "KEY")]
    api_key: Option<String>,

    /// Override the local data retention days from config file
    #[arg(long = "retention-days", value_name = "DAYS")]
    retention_days: Option<u32>,

    /// Override the auto update tasks flag from config file
    #[arg(long = "auto-update-tasks", value_name = "BOOL")]
    auto_update_tasks: Option<bool>,

    /// Override the local-only mode flag from config file
    #[arg(long = "local-only", value_name = "BOOL")]
    local_only: Option<bool>,
}

/// The main application structure for the agent.
/// It holds the core components of the agent, such as the configuration manager.
/// In later phases, it will also hold the database connection, task scheduler,
/// and the client for communicating with the central server.
pub struct Agent {
    pub config_manager: ConfigManager,
    database: Arc<RwLock<AgentDatabase>>,
    task_scheduler: Option<TaskScheduler>,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// The last time metrics were sent to server (as Unix timestamp)
    last_metrics_send: u64,
    /// The last time data cleanup was performed (as Unix timestamp)
    last_data_cleanup: u64,
    /// The last time HTTP clients were refreshed (as Unix timestamp)
    last_client_refresh: u64,
}

impl Agent {
    /// Creates and fully initializes a new agent instance.
    /// This performs all setup including loading config, initializing database,
    /// creating the scheduler, and registering with the server (if not in local-only mode).
    pub async fn new(config_dir: PathBuf) -> Result<Self> {
        info!("Starting Network Monitoring Agent");

        // The data directory is expected to be a sibling of the config directory.
        let data_dir = config_dir
            .parent()
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("./data"));
        info!("Data directory: {}", data_dir.display());

        let mut config_manager = ConfigManager::new(config_dir)?;
        // Load the initial configuration first to get database timeout
        config_manager.load_config().await?;

        let agent_config = config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded. Call load_config() first.");

        let database = Arc::new(RwLock::new(AgentDatabase::new(
            data_dir,
            agent_config.database_busy_timeout_seconds,
        )?));

        // If not in local_only mode, register with server by uploading config
        if !agent_config.local_only {
            info!("Registering with server by uploading configuration");
            if let Err(e) = Self::upload_config_to_server_static(&config_manager).await {
                warn!("Failed to upload configuration to server: {}", e);
                warn!("Continuing with local configuration");
            }
        }

        // Get configuration references
        let agent_config = config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded. Call load_config() first.");
        let tasks_config = config_manager
            .tasks_config
            .as_ref()
            .expect("Tasks configuration not loaded. Call load_config() first.");

        // Log key information
        if agent_config.local_only {
            info!(
                agent_id = %agent_config.agent_id,
                task_count = tasks_config.tasks.len(),
                local_only = true,
                "Agent configuration loaded (LOCAL-ONLY MODE)"
            );
        } else {
            info!(
                agent_id = %agent_config.agent_id,
                server_url = %agent_config.central_server_url,
                task_count = tasks_config.tasks.len(),
                "Agent configuration loaded"
            );
        }

        // Initialize the database
        {
            let mut db = database.write().await;
            db.initialize().await?;
        }
        info!("Database initialized successfully");

        // Create shutdown channel
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

        // Extract server config for bandwidth tests (None if local-only mode)
        let (server_url, api_key, agent_id) = if !agent_config.local_only {
            (
                Some(agent_config.central_server_url.clone()),
                Some(agent_config.api_key.clone()),
                Some(agent_config.agent_id.clone()),
            )
        } else {
            (None, None, None)
        };

        // Create and start the task scheduler
        let mut task_scheduler = TaskScheduler::new(
            tasks_config.clone(),
            database.clone(),
            agent_config.metrics_flush_interval_seconds,
            agent_config.graceful_shutdown_timeout_seconds,
            agent_config.channel_buffer_size,
            agent_config.queue_cleanup_interval_seconds,
            server_url,
            api_key,
            agent_id,
        )?;
        task_scheduler.start().await?;

        Ok(Self {
            config_manager,
            database,
            task_scheduler: Some(task_scheduler),
            shutdown_tx: Some(shutdown_tx),
            last_metrics_send: 0,
            last_data_cleanup: 0,
            last_client_refresh: 0,
        })
    }

    /// Starts the agent's main loop and runs indefinitely.
    /// The agent must be fully initialized via new() before calling this.
    pub async fn run(&mut self) -> Result<()> {
        let agent_config = self
            .config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded. Call load_config() first.")
            .clone();
        let _shutdown_tx = self.shutdown_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Agent not properly initialized - shutdown channel missing")
        })?;

        // Set up HTTP client if not in local-only mode
        let http_client = if !agent_config.local_only {
            Some(
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(
                        agent_config.http_client_timeout_seconds,
                    ))
                    .build()
                    .expect("Failed to create HTTP client"),
            )
        } else {
            None
        };

        // Main event loop - runs until stopped
        info!("Starting agent main loop");

        loop {
            // Check if scheduler is stopped
            let is_stopped = self
                .task_scheduler
                .as_ref()
                .map(|s| s.state == scheduler::SchedulerState::Stopped)
                .unwrap_or(true);

            if is_stopped {
                debug!("Scheduler stopped, exiting main loop");
                break;
            }

            // Perform periodic checks before handling events
            if let Some(scheduler) = self.task_scheduler.as_mut() {
                scheduler.check_and_perform_aggregation().await?;
                scheduler.flush_metrics_if_needed().await?;
                scheduler.cleanup_sent_queue_if_needed().await?;
            }

            self.send_metrics_if_needed(&http_client).await?;
            self.cleanup_data_if_needed().await?;
            self.refresh_clients_if_needed().await?;

            // Main event selection - only task execution events
            if let Some(scheduler) = self.task_scheduler.as_mut() {
                tokio::select! {
                    Some(result) = scheduler.result_receiver.recv() => {
                        scheduler.handle_task_result(result).await?;
                    },
                    Some(task_name) = scheduler.ready_receiver.recv() => {
                        scheduler.execute_single_task(&task_name).await?;
                    },
                }
            }
        }

        Ok(())
    }

    /// Sends queued metrics from database to the central server (static version)
    /// Returns true if config needs to be updated
    async fn send_queued_metrics_to_server_static(
        scheduler: &mut TaskScheduler,
        client: &reqwest::Client,
        agent_config: &shared::config::AgentConfig,
        config_checksum: &str,
    ) -> bool {
        let batch_size = agent_config.metrics_batch_size;
        let max_retries = agent_config.metrics_max_retries as i32;

        // Get next batch of metrics to send
        let mut db = scheduler.database.write().await;
        let queued_metrics = match db.get_metrics_to_send(batch_size).await {
            Ok(metrics) => metrics,
            Err(e) => {
                error!("Failed to fetch metrics from queue: {}", e);
                return false;
            }
        };

        if queued_metrics.is_empty() {
            return false;
        }

        let queue_ids: Vec<i64> = queued_metrics.iter().map(|m| m.queue_id).collect();
        let metrics: Vec<AggregatedMetrics> =
            queued_metrics.iter().map(|m| m.metric.clone()).collect();

        debug!("Attempting to send {} metrics to server", metrics.len());

        // Mark as 'sending' to prevent duplicate sends
        if let Err(e) = db.mark_as_sending(&queue_ids).await {
            warn!("Failed to mark metrics as sending: {}", e);
        }

        // Drop the write lock before making HTTP request
        drop(db);

        // Attempt to send
        match Self::send_metrics_batch(client, agent_config, config_checksum, &metrics).await {
            Ok(config_status) => {
                // Success! Mark as sent
                let mut db = scheduler.database.write().await;
                if let Err(e) = db.mark_as_sent(&queue_ids).await {
                    error!("Failed to mark metrics as sent: {}", e);
                } else {
                    info!("Successfully sent {} metrics to server", metrics.len());
                }

                // Check if config needs update
                if config_status == shared::api::ConfigStatus::Stale {
                    info!("Server indicates config is stale");
                    return true;
                }

                false
            }
            Err(e) => {
                // Failed - mark for retry with exponential backoff
                warn!("Failed to send metrics: {}", e);

                let mut db = scheduler.database.write().await;
                for queue_id in queue_ids {
                    if let Err(e) = db
                        .mark_as_failed(queue_id, &e.to_string(), max_retries)
                        .await
                    {
                        error!("Failed to update queue entry {}: {}", queue_id, e);
                    }
                }

                false
            }
        }
    }

    /// Sends aggregated metrics to the central server
    pub async fn send_metrics_to_server(&self, metrics: Vec<AggregatedMetrics>) -> Result<()> {
        let agent_config = self
            .config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded. Call load_config() first.");
        let client = reqwest::Client::new();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let request = MetricsRequest {
            agent_id: agent_config.agent_id.clone(),
            timestamp_utc: timestamp.to_string(),
            config_checksum: self
                .config_manager
                .get_tasks_config_hash()
                .expect("Failed to calculate tasks config hash"),
            metrics,
        };

        let url = format!("{}{}", agent_config.central_server_url, endpoints::METRICS);

        let response = client
            .post(&url)
            .header(headers::API_KEY, &agent_config.api_key)
            .header(headers::AGENT_ID, &agent_config.agent_id)
            .header(headers::CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send metrics to server: {}", url))?;

        if response.status().is_success() {
            let metrics_response: MetricsResponse = response
                .json()
                .await
                .context("Failed to parse server response")?;
            info!(
                "Successfully sent {} metrics to server",
                request.metrics.len()
            );
            info!("Server config status: {:?}", metrics_response.config_status);
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow::anyhow!(
                "Server returned error {}: {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Upload local configuration to server (static version for use in new())
    async fn upload_config_to_server_static(config_manager: &ConfigManager) -> Result<()> {
        let agent_config = config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded. Call load_config() first.");

        // Get the tasks.toml content
        let tasks_content = config_manager.get_tasks_config_content()?;

        // Compress with gzip
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(tasks_content.as_bytes())
            .context("Failed to compress tasks config")?;
        let compressed_data = encoder
            .finish()
            .context("Failed to finish gzip compression")?;

        // Encode as base64
        let tasks_toml_encoded = base64::engine::general_purpose::STANDARD.encode(&compressed_data);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let request = ConfigUploadRequest {
            agent_id: agent_config.agent_id.clone(),
            timestamp_utc: timestamp.to_string(),
            tasks_toml: tasks_toml_encoded,
        };

        let url = format!(
            "{}{}",
            agent_config.central_server_url,
            endpoints::CONFIG_UPLOAD
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header(headers::API_KEY, &agent_config.api_key)
            .header(headers::AGENT_ID, &agent_config.agent_id)
            .header(headers::CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send config upload request to {}", url))?;

        if response.status().is_success() {
            let upload_response: ConfigUploadResponse = response
                .json()
                .await
                .context("Failed to parse config upload response")?;

            if upload_response.accepted {
                info!("Server accepted and saved our configuration");
            } else {
                info!("Server response: {}", upload_response.message);
            }
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow::anyhow!(
                "Config upload failed {}: {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Upload local configuration to server (instance method)
    async fn upload_config_to_server(&self) -> Result<()> {
        Self::upload_config_to_server_static(&self.config_manager).await
    }

    /// Sends queued metrics to server if enough time has passed.
    ///
    /// Checks if the configured send interval has elapsed since the last send.
    /// If so, and if not in local-only mode, fetches a batch of metrics from
    /// the queue and sends them to the central server.
    ///
    /// # Parameters
    /// * `http_client` - The HTTP client to use for sending metrics
    /// * `config_checksum` - The current configuration checksum
    ///
    /// # Returns
    /// `Ok(())` on success or if no send needed
    async fn send_metrics_if_needed(
        &mut self,
        http_client: &Option<reqwest::Client>,
    ) -> Result<()> {
        let agent_config = self
            .config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded");

        // Skip if in local-only mode or no HTTP client
        if agent_config.local_only || http_client.is_none() {
            return Ok(());
        }

        let auto_update_enabled = agent_config.auto_update_tasks;
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Only send if the configured interval has passed
        if current_time
            >= self.last_metrics_send + agent_config.metrics_send_interval_seconds as u64
        {
            // Get fresh tasks config hash (server only compares tasks.toml, not agent.toml)
            let config_checksum = self
                .config_manager
                .get_tasks_config_hash()
                .expect("Failed to calculate tasks config hash");

            let config_needs_update = if let Some(scheduler) = self.task_scheduler.as_mut() {
                if let Some(client) = http_client {
                    Self::send_queued_metrics_to_server_static(
                        scheduler,
                        client,
                        agent_config,
                        &config_checksum,
                    )
                    .await
                } else {
                    false
                }
            } else {
                false
            };

            // If config is stale and auto-update is enabled, download and apply new config
            if config_needs_update && auto_update_enabled {
                info!("Auto-update enabled, downloading new configuration from server");
                if let Err(e) = self.download_and_apply_config().await {
                    error!("Failed to download and apply new configuration: {}", e);
                }
            } else if config_needs_update && !auto_update_enabled {
                warn!("Config update available from server but auto_update_tasks is disabled");
            }

            self.last_metrics_send = current_time;
        }

        Ok(())
    }

    /// Downloads new configuration from server and applies it
    ///
    /// Calls the /api/v1/config/verify endpoint to get the new configuration,
    /// then updates the tasks.toml file and reloads the scheduler.
    ///
    /// # Returns
    /// `Ok(())` on success, error if download or application fails
    async fn download_and_apply_config(&mut self) -> Result<()> {
        let agent_config = self
            .config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded")
            .clone();

        let current_checksum = self
            .config_manager
            .get_tasks_config_hash()
            .context("Failed to calculate tasks config hash")?;

        info!("Downloading new configuration from server");

        // Prepare request
        let request = shared::api::ConfigVerifyRequest {
            agent_id: agent_config.agent_id.clone(),
            tasks_config_hash: current_checksum.clone(),
        };

        let url = format!(
            "{}{}",
            agent_config.central_server_url,
            shared::api::endpoints::CONFIG_VERIFY
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                agent_config.http_client_timeout_seconds,
            ))
            .build()
            .context("Failed to create HTTP client")?;

        let response = client
            .post(&url)
            .header(shared::api::headers::API_KEY, &agent_config.api_key)
            .header(shared::api::headers::AGENT_ID, &agent_config.agent_id)
            .header(shared::api::headers::CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send config verify request to {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow::anyhow!(
                "Config verify failed {}: {}",
                status,
                error_text
            ));
        }

        let verify_response: shared::api::ConfigVerifyResponse = response
            .json()
            .await
            .context("Failed to parse config verify response")?;

        // Check if we received new config
        if verify_response.config_status == shared::api::ConfigStatus::Stale {
            if let Some(tasks_toml_compressed) = verify_response.tasks_toml {
                info!("Received new configuration from server, applying update");

                // Decode base64
                let compressed_data = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &tasks_toml_compressed,
                )
                .context("Failed to decode base64 config")?;

                // Decompress gzip
                use std::io::Read;
                let mut decoder = flate2::read::GzDecoder::new(&compressed_data[..]);
                let mut new_tasks_toml = String::new();
                decoder
                    .read_to_string(&mut new_tasks_toml)
                    .context("Failed to decompress config")?;

                // Update config using ConfigManager
                self.config_manager
                    .update_tasks_config(&new_tasks_toml)
                    .await
                    .context("Failed to update tasks configuration")?;

                // Reload scheduler with new tasks
                if let Some(scheduler) = self.task_scheduler.as_mut() {
                    let new_tasks_config = self
                        .config_manager
                        .tasks_config
                        .as_ref()
                        .expect("Tasks config should be loaded after update")
                        .clone();

                    // Stop current scheduler
                    scheduler.stop().await?;

                    // Extract server config for bandwidth tests (None if local-only mode)
                    let (server_url, api_key, agent_id) = if !agent_config.local_only {
                        (
                            Some(agent_config.central_server_url.clone()),
                            Some(agent_config.api_key.clone()),
                            Some(agent_config.agent_id.clone()),
                        )
                    } else {
                        (None, None, None)
                    };

                    // Create new scheduler with new config
                    let new_scheduler = TaskScheduler::new(
                        new_tasks_config,
                        scheduler.database.clone(),
                        agent_config.metrics_flush_interval_seconds,
                        agent_config.graceful_shutdown_timeout_seconds,
                        agent_config.channel_buffer_size,
                        agent_config.queue_cleanup_interval_seconds,
                        server_url,
                        api_key,
                        agent_id,
                    )?;

                    *scheduler = new_scheduler;
                    scheduler.start().await?;

                    info!("Successfully applied new configuration and restarted scheduler");
                } else {
                    warn!("No scheduler to reload");
                }
            } else {
                // Server doesn't have a config for us - upload our config
                warn!("Server indicated config is stale but did not provide new config");
                info!("Uploading local configuration to server");

                if let Err(e) = self.upload_config_to_server().await {
                    error!("Failed to upload configuration to server: {}", e);
                    return Err(e);
                }

                info!("Successfully uploaded configuration to server");
            }
        } else {
            debug!("Config is up to date according to server");
        }

        Ok(())
    }

    /// Cleans up old data from database if enough time has passed.
    ///
    /// Checks if the configured cleanup interval has elapsed since the last cleanup.
    /// If so, removes raw metrics and aggregated data older than the configured
    /// retention period.
    ///
    /// # Returns
    /// `Ok(())` on success or if no cleanup needed, database error otherwise
    async fn cleanup_data_if_needed(&mut self) -> Result<()> {
        let agent_config = self
            .config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded");

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Only cleanup if the configured interval has passed
        if current_time >= self.last_data_cleanup + agent_config.data_cleanup_interval_seconds {
            let retention_days = agent_config.local_data_retention_days;
            info!(
                "Running database cleanup (retention: {} days)",
                retention_days
            );

            if let Some(scheduler) = self.task_scheduler.as_mut() {
                let mut db = scheduler.database.write().await;
                if let Err(e) = db.cleanup_old_data(retention_days).await {
                    error!("Failed to cleanup old database data: {}", e);
                } else {
                    info!("Database cleanup completed successfully");
                }
            }

            self.last_data_cleanup = current_time;
        }

        Ok(())
    }

    /// Refreshes HTTP clients and TLS connectors if enough time has passed.
    ///
    /// Checks if the configured refresh interval has elapsed since the last refresh.
    /// If so, calls the scheduler to refresh the task executor's HTTP clients and
    /// TLS connectors. This helps prevent memory leaks and ensures clean state.
    ///
    /// # Returns
    /// `Ok(())` on success or if no refresh needed, error if refresh fails
    async fn refresh_clients_if_needed(&mut self) -> Result<()> {
        let agent_config = self
            .config_manager
            .agent_config
            .as_ref()
            .expect("Agent configuration not loaded");

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Only refresh if the configured interval has passed
        if current_time
            >= self.last_client_refresh + agent_config.http_client_refresh_interval_seconds
        {
            info!("Refreshing HTTP clients and TLS connectors");

            if let Some(scheduler) = self.task_scheduler.as_mut() {
                if let Err(e) = scheduler.refresh_http_clients() {
                    error!("Failed to refresh HTTP clients: {}", e);
                } else {
                    info!("HTTP clients and TLS connectors refreshed successfully");
                }
            }

            self.last_client_refresh = current_time;
        }

        Ok(())
    }

    /// Performs a graceful shutdown of the agent.
    /// This method is called when a shutdown signal is received. It should ensure
    /// that any pending data is saved and resources are released cleanly.
    ///
    pub async fn shutdown(&mut self) {
        info!("Shutting down Network Monitoring Agent");

        // Step 1: Signal all background tasks to stop
        if let Some(shutdown_tx) = &self.shutdown_tx {
            info!("Sending shutdown signal to background tasks");
            let _ = shutdown_tx.send(());
        }

        // Step 2: Stop the task scheduler (waits for in-flight tasks, flushes metrics)
        if let Some(scheduler) = self.task_scheduler.as_mut() {
            info!("Stopping task scheduler");
            if let Err(e) = scheduler.stop().await {
                error!("Error stopping task scheduler: {}", e);
            } else {
                info!("Task scheduler stopped successfully");
            }
        }

        // Step 3: Give background tasks a moment to process shutdown signal
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Step 4: Close the database connection
        {
            let mut database = self.database.write().await;
            database.close().await;
        }

        info!("Network Monitoring Agent shutdown complete");
    }

    /// Send a batch of metrics to the server
    /// Returns the config status from the server response
    async fn send_metrics_batch(
        client: &reqwest::Client,
        agent_config: &shared::config::AgentConfig,
        config_checksum: &str,
        metrics: &[AggregatedMetrics],
    ) -> Result<shared::api::ConfigStatus> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let request = MetricsRequest {
            agent_id: agent_config.agent_id.clone(),
            timestamp_utc: timestamp.to_string(),
            config_checksum: config_checksum.to_string(),
            metrics: metrics.to_vec(),
        };

        let url = format!("{}{}", agent_config.central_server_url, endpoints::METRICS);

        let response = client
            .post(&url)
            .header(headers::API_KEY, &agent_config.api_key)
            .header(headers::AGENT_ID, &agent_config.agent_id)
            .header(headers::CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send metrics to {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(anyhow::anyhow!(
                "Server returned {}: {}",
                status,
                error_text
            ));
        }

        // Parse the response to get config status
        let metrics_response: MetricsResponse = response
            .json()
            .await
            .context("Failed to parse metrics response")?;

        Ok(metrics_response.config_status)
    }
}

// The `#[tokio::main]` attribute transforms the `async fn main` into a synchronous
// `fn main` that initializes a tokio runtime and runs the async code.
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize the logging framework (`tracing`).
    // `tracing_subscriber` is used to configure how logs are processed and displayed.
    let file_appender = tracing_appender::rolling::daily("./logs", "agent.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Configure logging with proper RUST_LOG environment variable handling
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // Default directives are only used if RUST_LOG is not set
        tracing_subscriber::EnvFilter::new("agent=info,shared=info")
    });

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(non_blocking)
        .init();

    // Parse command-line arguments
    let cli_args = CliArgs::parse();

    info!("Network Monitoring Agent starting up");
    info!("Configuration directory: {}", cli_args.config_dir.display());

    // Log which overrides are provided
    if cli_args.agent_id.is_some() {
        info!("Agent ID override provided via command line");
    }
    if cli_args.server_url.is_some() {
        info!("Server URL override provided via command line");
    }
    if cli_args.api_key.is_some() {
        info!("API key override provided via command line");
    }
    if cli_args.retention_days.is_some() {
        info!("Retention days override provided via command line");
    }
    if cli_args.auto_update_tasks.is_some() {
        info!("Auto update tasks override provided via command line");
    }
    if cli_args.local_only.is_some() {
        info!("Local-only mode override provided via command line");
    }

    // Create and initialize a new `Agent` instance. If this fails, log the error and exit,
    // as the agent cannot run without successful initialization.
    let mut agent = match Agent::new(cli_args.config_dir).await {
        Ok(agent) => agent,
        Err(e) => {
            error!(
                "================================================================================"
            );
            error!("FATAL ERROR: Failed to initialize agent");
            error!(
                "================================================================================"
            );
            error!("");
            error!("Error: {}", e);

            // Print the full error chain to show all context
            let mut current_error = e.source();
            while let Some(err) = current_error {
                error!("  Caused by: {}", err);
                current_error = err.source();
            }
            error!("");

            // Check if this is a validation error and provide additional context
            let error_msg = format!("{:?}", e);
            if error_msg.contains("Validation") || error_msg.contains("validation") {
                error!("This appears to be a CONFIGURATION VALIDATION ERROR.");
                error!("");
                error!("Please review your configuration files:");
                error!(
                    "  - agent.toml: Check agent settings (agent_id, server_url, api_key, etc.)"
                );
                error!("  - tasks.toml: Check all task definitions");
                error!("");
                error!("Common validation issues:");
                error!("  * Empty required parameters (host, url, domain, etc.)");
                error!("  * Invalid URL formats (must start with http:// or https://)");
                error!("  * Invalid regular expressions in http_content tasks");
                error!("  * Duplicate task names (each task must have a unique name)");
                error!("  * Invalid schedule_seconds (must be > 0, bandwidth tasks >= 60)");
                error!("  * Task type mismatch (type field doesn't match parameters)");
            } else if error_msg.contains("Failed to read") {
                error!("This appears to be a CONFIGURATION FILE ERROR.");
                error!(
                    "Please ensure both agent.toml and tasks.toml exist in the config directory."
                );
            } else if error_msg.contains("Failed to parse") {
                error!("This appears to be a TOML SYNTAX ERROR.");
                error!("Please check your TOML files for syntax errors:");
                error!("  * Missing quotes around strings");
                error!("  * Incorrect array syntax [[tasks]] for task definitions");
                error!("  * Invalid key-value pairs");
                error!("  * Unclosed brackets or quotes");
            }

            error!("");
            error!(
                "================================================================================"
            );
            error!("Agent startup ABORTED. Please fix the errors above and try again.");
            error!(
                "================================================================================"
            );
            std::process::exit(1);
        }
    };

    // Apply command-line overrides if provided
    if cli_args.agent_id.is_some()
        || cli_args.server_url.is_some()
        || cli_args.api_key.is_some()
        || cli_args.retention_days.is_some()
        || cli_args.auto_update_tasks.is_some()
        || cli_args.local_only.is_some()
    {
        match agent
            .config_manager
            .override_and_persist_agent_config(
                cli_args.agent_id,
                cli_args.server_url,
                cli_args.api_key,
                cli_args.retention_days,
                cli_args.auto_update_tasks,
                cli_args.local_only,
            )
            .await
        {
            Ok(changed) => {
                if changed {
                    info!("Configuration overrides applied and persisted to disk");
                } else {
                    info!("Command-line values match existing config, no changes needed");
                }
            }
            Err(e) => {
                error!("Failed to apply configuration overrides: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Set up signal handling for graceful shutdown. This is crucial for a
    // long-running service to be able to shut down cleanly.
    let shutdown_signal = async {
        // On Unix-like systems (Linux, macOS), we listen for SIGTERM and SIGINT.
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).unwrap();
            let mut sigint = signal(SignalKind::interrupt()).unwrap();

            // `tokio::select!` waits for the first branch to complete.
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("Received SIGTERM");
                },
                _ = sigint.recv() => {
                    info!("Received SIGINT");
                },
            }
        }

        // On other systems (like Windows), we listen for Ctrl+C.
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.unwrap();
            info!("Received Ctrl+C");
        }
    };

    // The main execution block. `tokio::select!` is used to run the agent's
    // main loop and the shutdown signal handler concurrently. The first one
    // to complete will cause the `select!` block to exit.
    tokio::select! {
        // Run the agent's main logic.
        result = agent.run() => {
            // If `agent.run()` returns an error, log it and exit.
            if let Err(e) = result {
                error!("Agent error: {}", e);
                std::process::exit(1);
            }
        }
        // Wait for the shutdown signal.
        _ = shutdown_signal => {
            info!("Shutdown signal received");
        }
    }

    // Once the `select!` block exits (due to an error or a shutdown signal),
    // call the `shutdown` method to clean up.
    agent.shutdown().await;
    info!("Agent shutdown complete");
    Ok(())
}

// Unit tests for the main module.
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_agent_creation() {
        // This test is disabled because Agent::new now requires valid config files
        // and performs full initialization including database setup.
        // TODO: Create proper test fixtures for integration testing
    }
}
