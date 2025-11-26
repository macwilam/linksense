//! Network Monitoring Central Server
//!
//! The central server aggregates metrics from multiple agents, manages agent
//! configurations, and provides REST API endpoints for data ingestion and
//! configuration distribution.
// This is the main entry point for the server application. It's responsible for:
// - Initializing logging and configuration.
// - Setting up the main `Server` struct.
// - Starting the web server and API endpoints.
// - Handling graceful shutdown.

// Use jemalloc as the global allocator for better performance
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Server version from Cargo.toml
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

use anyhow::{Context, Result};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

// The server is organized into modules for API, configuration, and database management.
mod api;
mod bandwidth_state;
mod config;
mod database;
mod health_monitor;
mod reconfigure;

use config::ConfigManager;
use reconfigure::ReconfigureManager;

/// Command-line arguments for the server
#[derive(Parser, Debug)]
#[command(name = "server")]
#[command(about = "Central server for network monitoring that aggregates metrics", long_about = None)]
struct CliArgs {
    /// Path to the server configuration file (server.toml)
    #[arg(value_name = "CONFIG_FILE")]
    config_file: PathBuf,

    /// Override the listen address from config file
    #[arg(long = "listen-address", value_name = "ADDRESS")]
    listen_address: Option<String>,

    /// Override the API key from config file
    #[arg(long = "api-key", value_name = "KEY")]
    api_key: Option<String>,

    /// Override the data retention days from config file
    #[arg(long = "retention-days", value_name = "DAYS")]
    retention_days: Option<u32>,

    /// Override the agent configs directory from config file
    #[arg(long = "agent-configs-dir", value_name = "DIR")]
    agent_configs_dir: Option<String>,

    /// Override the bandwidth test size from config file
    #[arg(long = "bandwidth-size-mb", value_name = "MB")]
    bandwidth_size_mb: Option<u32>,

    /// Override the reconfigure check interval from config file
    #[arg(long = "reconfigure-interval", value_name = "SECONDS")]
    reconfigure_interval: Option<u32>,
}

/// The main application structure for the server.
/// It holds the core components, such as the configuration manager and the
/// network address to listen on.
pub struct Server {
    /// The configuration manager, responsible for loading and accessing server settings.
    /// Wrapped in Arc<Mutex<>> to allow sharing between server and API handlers.
    pub config_manager: Arc<Mutex<ConfigManager>>,
    /// The `SocketAddr` (IP address and port) on which the web server will listen.
    listen_address: SocketAddr,
    /// The reconfigure manager for handling multi-agent configuration updates.
    /// Wrapped in Arc<Mutex<>> to allow sharing between server and background task.
    reconfigure_manager: Arc<Mutex<ReconfigureManager>>,
    /// Database handle for metrics storage. Wrapped in Arc<Mutex<>> for sharing.
    database: Option<Arc<tokio::sync::Mutex<crate::database::ServerDatabase>>>,
    /// Handle to the reconfigure background task for graceful shutdown.
    reconfigure_task_handle: Option<JoinHandle<()>>,
    /// Handle to the database cleanup task for graceful shutdown.
    cleanup_task_handle: Option<JoinHandle<()>>,
    /// Handle to the WAL checkpoint task for graceful shutdown.
    wal_checkpoint_task_handle: Option<JoinHandle<()>>,
    /// Handle to the health monitoring task for graceful shutdown.
    health_monitor_task_handle: Option<JoinHandle<()>>,
    /// Shutdown signal sender for notifying background tasks.
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl Server {
    /// Creates a new server instance.
    /// This function initializes the configuration manager and parses the listen
    /// address from the configuration. It returns a `Result` because these
    /// initial steps can fail (e.g., invalid configuration file, invalid address format).
    pub fn new(config_path: PathBuf) -> Result<Self> {
        // The `ConfigManager` is created first, as it's needed to get other settings.
        let config_manager = ConfigManager::new(config_path)?;
        let server_config = config_manager.server_config.as_ref().expect(
            "Server configuration not loaded. This should not happen as config is loaded in new().",
        );

        // The listen address is parsed from a string in the config into a `SocketAddr`.
        // This can fail if the string is not a valid IP:port combination.
        let listen_address: SocketAddr = server_config.listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Invalid listen address '{}': {}",
                server_config.listen_address,
                e
            )
        })?;

        // Initialize the reconfigure manager
        let agent_configs_dir = std::path::PathBuf::from(&server_config.agent_configs_dir);
        let reconfigure_dir = agent_configs_dir
            .parent()
            .unwrap_or(&agent_configs_dir)
            .join("reconfigure");

        let reconfigure_manager = Arc::new(Mutex::new(ReconfigureManager::new(
            reconfigure_dir,
            agent_configs_dir,
        )?));

        Ok(Self {
            config_manager: Arc::new(Mutex::new(config_manager)),
            listen_address,
            reconfigure_manager,
            database: None,
            reconfigure_task_handle: None,
            cleanup_task_handle: None,
            wal_checkpoint_task_handle: None,
            health_monitor_task_handle: None,
            shutdown_tx: None,
        })
    }

    /// Starts the server and runs indefinitely.
    /// This is the main operational function of the server.
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting Network Monitoring Central Server");

        let server_config = {
            let config_manager = self.config_manager.lock().await;
            config_manager.server_config.as_ref()
                .expect("Server configuration not loaded. This should not happen as config is loaded in new().")
                .clone()
        };
        // Log key configuration details at startup for verification.
        info!(
            listen_address = %self.listen_address,
            retention_days = server_config.data_retention_days,
            config_dir = %server_config.agent_configs_dir,
            "Server configuration loaded"
        );

        // Initialize the database
        info!("Initializing database");
        let data_dir = std::path::PathBuf::from("./data");
        let mut database = crate::database::ServerDatabase::new(&data_dir)
            .context("Failed to create database manager")?;

        database
            .initialize()
            .await
            .context("Failed to initialize database")?;

        info!("Database initialized successfully");

        // Initialize bandwidth test manager with config values
        let bandwidth_manager = crate::bandwidth_state::BandwidthTestManager::new(
            server_config.bandwidth_test_timeout_seconds,
            server_config.bandwidth_max_delay_seconds,
            server_config.bandwidth_queue_base_delay_seconds,
            server_config.bandwidth_queue_current_test_delay_seconds,
            server_config.bandwidth_queue_position_multiplier_seconds,
        );
        info!("Bandwidth test manager initialized");

        // Create shutdown broadcast channel
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx.clone());

        // Start periodic cleanup task for old data
        let cleanup_interval_hours = server_config.cleanup_interval_hours;
        let retention_days = server_config.data_retention_days;
        let db_for_cleanup = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::database::ServerDatabase::new(&data_dir)
                .context("Failed to create database manager for cleanup task")?,
        ));

        let mut cleanup_shutdown_rx = shutdown_tx.subscribe();
        let cleanup_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                (cleanup_interval_hours as u64) * 3600,
            ));

            // Run first cleanup after configured initial delay
            tokio::time::sleep(std::time::Duration::from_secs(
                server_config.initial_cleanup_delay_seconds,
            ))
            .await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        info!("Running periodic database cleanup");
                        let mut db = db_for_cleanup.lock().await;
                        if let Err(e) = db.cleanup_old_data(retention_days).await {
                            error!("Database cleanup failed: {}", e);
                        } else {
                            info!("Database cleanup completed successfully");
                        }
                    }
                    _ = cleanup_shutdown_rx.recv() => {
                        info!("Cleanup task received shutdown signal");
                        break;
                    }
                }
            }
        });

        // Store database handle for graceful shutdown
        let database_arc = Arc::new(tokio::sync::Mutex::new(database));
        self.database = Some(Arc::clone(&database_arc));

        // Start periodic WAL checkpoint task
        let wal_checkpoint_interval_secs = server_config.wal_checkpoint_interval_seconds;
        let db_for_wal = Arc::clone(&database_arc);
        let mut wal_shutdown_rx = shutdown_tx.subscribe();
        let wal_checkpoint_task = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(wal_checkpoint_interval_secs));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        info!("Running periodic WAL checkpoint");
                        let mut db = db_for_wal.lock().await;
                        match db.checkpoint_wal().await {
                            Ok(frames) => {
                                info!("WAL checkpoint completed: {} frames checkpointed", frames);
                            }
                            Err(e) => {
                                warn!("WAL checkpoint failed: {}", e);
                            }
                        }
                    }
                    _ = wal_shutdown_rx.recv() => {
                        info!("WAL checkpoint task received shutdown signal");
                        break;
                    }
                }
            }
        });

        // Create application state with all dependencies
        let app_state = crate::api::AppState::new(
            server_config.clone(),
            Arc::clone(&database_arc),
            Arc::clone(&self.config_manager),
            bandwidth_manager,
        );

        // Set up the full REST API using the `api` module
        let app = crate::api::create_router(app_state);

        info!("Starting HTTP server on {}", self.listen_address);

        // Bind a TCP listener to the configured address.
        let listener = tokio::net::TcpListener::bind(self.listen_address)
            .await
            .with_context(|| {
                format!(
                    "Failed to bind TCP listener to {}. \
                     Check if port is already in use (EADDRINUSE) or requires elevated permissions (EACCES).",
                    self.listen_address
                )
            })?;

        // Start the reconfigure monitoring task
        let reconfigure_dir = {
            let manager = self.reconfigure_manager.lock().await;
            manager.reconfigure_dir().to_path_buf()
        };

        info!(
            "Starting reconfigure monitoring on: {}",
            reconfigure_dir.display()
        );

        // Clone the Arc to share the reconfigure manager with the background task
        let reconfigure_manager_clone = Arc::clone(&self.reconfigure_manager);

        let mut shutdown_rx = shutdown_tx.subscribe();

        // Start reconfigure monitoring task with shutdown support
        let interval_secs = server_config.reconfigure_check_interval_seconds;
        let reconfigure_task = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(interval_secs as u64));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let mut manager = reconfigure_manager_clone.lock().await;
                        if let Err(e) = manager.check_and_process().await {
                            error!("Reconfigure check failed: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Reconfigure task received shutdown signal");
                        break;
                    }
                }
            }
        });

        self.reconfigure_task_handle = Some(reconfigure_task);
        self.cleanup_task_handle = Some(cleanup_task);
        self.wal_checkpoint_task_handle = Some(wal_checkpoint_task);

        // Start health monitoring task if enabled
        if server_config.monitor_agents_health {
            info!("Agent health monitoring is enabled");
            let health_monitor = crate::health_monitor::HealthMonitor::new(
                Arc::clone(&database_arc),
                Arc::clone(&self.config_manager),
                data_dir.clone(),
                SERVER_VERSION.to_string(),
            )?;

            let health_check_interval_secs = server_config.health_check_interval_seconds;
            let health_retention_days = server_config.health_check_retention_days;
            let mut health_shutdown_rx = shutdown_tx.subscribe();

            let health_monitor_task = tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                    health_check_interval_secs,
                ));

                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            info!("Running scheduled agent health check");
                            if let Err(e) = health_monitor.check_all_agents().await {
                                error!("Agent health check failed: {}", e);
                            }

                            // Clean up old health check data
                            if let Err(e) = health_monitor.cleanup_old_health_data(health_retention_days).await {
                                error!("Health check data cleanup failed: {}", e);
                            }
                        }
                        _ = health_shutdown_rx.recv() => {
                            info!("Health monitor task received shutdown signal");
                            break;
                        }
                    }
                }
            });

            self.health_monitor_task_handle = Some(health_monitor_task);
            info!(
                "Health monitoring task started (interval: {}s, threshold: {:.0}%)",
                health_check_interval_secs,
                server_config.health_check_success_ratio_threshold * 100.0
            );
        } else {
            info!("Agent health monitoring is disabled");
        }

        // Create a shutdown signal receiver for axum
        let shutdown_signal = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                info!("HTTP server received shutdown signal");
            }
        };

        // Start the axum server with graceful shutdown support
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await
            .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

        Ok(())
    }

    /// Performs a graceful shutdown of the server.
    ///
    /// Shutdown sequence:
    /// 1. Broadcast shutdown signal to all background tasks
    /// 2. Wait for reconfigure task to complete (configurable timeout)
    /// 3. Wait for HTTP server to finish in-flight requests
    /// 4. Close database connections (when implemented)
    /// 5. Flush any pending data (when implemented)
    pub async fn shutdown(&mut self) {
        info!("Shutting down Network Monitoring Central Server gracefully");

        let shutdown_timeout_secs = {
            let config_manager = self.config_manager.lock().await;
            config_manager
                .server_config
                .as_ref()
                .map(|c| c.graceful_shutdown_timeout_seconds)
                .unwrap_or(30)
        };

        // Send shutdown signal to all background tasks
        if let Some(shutdown_tx) = &self.shutdown_tx {
            if let Err(e) = shutdown_tx.send(()) {
                warn!("Failed to send shutdown signal: {}", e);
            }
        }

        // Wait for reconfigure task to complete
        if let Some(handle) = self.reconfigure_task_handle.take() {
            info!(
                "Waiting for reconfigure task to complete (timeout: {}s)",
                shutdown_timeout_secs
            );

            match tokio::time::timeout(
                std::time::Duration::from_secs(shutdown_timeout_secs),
                handle,
            )
            .await
            {
                Ok(Ok(())) => {
                    info!("Reconfigure task completed successfully");
                }
                Ok(Err(e)) => {
                    warn!("Reconfigure task panicked: {}", e);
                }
                Err(_) => {
                    warn!("Reconfigure task shutdown timeout reached, aborting");
                }
            }
        }

        // Wait for cleanup task to complete
        if let Some(handle) = self.cleanup_task_handle.take() {
            info!(
                "Waiting for cleanup task to complete (timeout: {}s)",
                shutdown_timeout_secs
            );

            match tokio::time::timeout(
                std::time::Duration::from_secs(shutdown_timeout_secs),
                handle,
            )
            .await
            {
                Ok(Ok(())) => {
                    info!("Cleanup task completed successfully");
                }
                Ok(Err(e)) => {
                    warn!("Cleanup task panicked: {}", e);
                }
                Err(_) => {
                    warn!("Cleanup task shutdown timeout reached, aborting");
                }
            }
        }

        // Wait for WAL checkpoint task to complete
        if let Some(handle) = self.wal_checkpoint_task_handle.take() {
            info!(
                "Waiting for WAL checkpoint task to complete (timeout: {}s)",
                shutdown_timeout_secs
            );

            match tokio::time::timeout(
                std::time::Duration::from_secs(shutdown_timeout_secs),
                handle,
            )
            .await
            {
                Ok(Ok(())) => {
                    info!("WAL checkpoint task completed successfully");
                }
                Ok(Err(e)) => {
                    warn!("WAL checkpoint task panicked: {}", e);
                }
                Err(_) => {
                    warn!("WAL checkpoint task shutdown timeout reached, aborting");
                }
            }
        }

        // Wait for health monitor task to complete
        if let Some(handle) = self.health_monitor_task_handle.take() {
            info!(
                "Waiting for health monitor task to complete (timeout: {}s)",
                shutdown_timeout_secs
            );

            match tokio::time::timeout(
                std::time::Duration::from_secs(shutdown_timeout_secs),
                handle,
            )
            .await
            {
                Ok(Ok(())) => {
                    info!("Health monitor task completed successfully");
                }
                Ok(Err(e)) => {
                    warn!("Health monitor task panicked: {}", e);
                }
                Err(_) => {
                    warn!("Health monitor task shutdown timeout reached, aborting");
                }
            }
        }

        // Close database connection
        if let Some(database_arc) = &self.database {
            info!("Closing database connection");
            let mut db = database_arc.lock().await;
            db.close().await;
            info!("Database connection closed");
        }

        info!("Server shutdown complete");
    }
}

/// Sets up signal handlers for graceful shutdown.
/// Returns a future that completes when a shutdown signal is received.
///
/// On Unix systems, handles SIGTERM and SIGINT signals.
/// On non-Unix systems, handles Ctrl+C.
/// If signal registration fails, falls back to Ctrl+C handling.
async fn setup_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let sigterm = signal(SignalKind::terminate());
        let sigint = signal(SignalKind::interrupt());

        match (sigterm, sigint) {
            (Ok(mut sigterm), Ok(mut sigint)) => {
                tokio::select! {
                    _ = sigterm.recv() => info!("Received SIGTERM"),
                    _ = sigint.recv() => info!("Received SIGINT"),
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                error!("Failed to register signal handlers: {}", e);
                error!("Falling back to Ctrl+C only");
                if let Err(e) = tokio::signal::ctrl_c().await {
                    error!("Failed to wait for Ctrl+C: {}", e);
                } else {
                    info!("Received Ctrl+C");
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        if let Err(e) = tokio::signal::ctrl_c().await {
            error!("Failed to receive Ctrl+C signal: {}", e);
        } else {
            info!("Received Ctrl+C");
        }
    }
}

/// Server entry point
///
/// Initializes logging, loads configuration, creates server instance, and runs
/// until shutdown signal is received.
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging.
    let file_appender = tracing_appender::rolling::daily("./logs", "server.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Configure logging with proper RUST_LOG environment variable handling
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // Default directives are only used if RUST_LOG is not set
        tracing_subscriber::EnvFilter::new("server=info,shared=info")
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

    info!("Network Monitoring Central Server starting up");
    info!("Configuration file: {}", cli_args.config_file.display());

    // Log which overrides are provided
    if cli_args.listen_address.is_some() {
        info!("Listen address override provided via command line");
    }
    if cli_args.api_key.is_some() {
        info!("API key override provided via command line");
    }
    if cli_args.retention_days.is_some() {
        info!("Retention days override provided via command line");
    }
    if cli_args.agent_configs_dir.is_some() {
        info!("Agent configs directory override provided via command line");
    }
    if cli_args.bandwidth_size_mb.is_some() {
        info!("Bandwidth size override provided via command line");
    }
    if cli_args.reconfigure_interval.is_some() {
        info!("Reconfigure interval override provided via command line");
    }

    // Create and initialize the server. Exit if initialization fails.
    let mut server = match Server::new(cli_args.config_file) {
        Ok(server) => server,
        Err(e) => {
            error!("Failed to initialize server: {}", e);
            std::process::exit(1);
        }
    };

    // Apply command-line overrides if provided
    if cli_args.listen_address.is_some()
        || cli_args.api_key.is_some()
        || cli_args.retention_days.is_some()
        || cli_args.agent_configs_dir.is_some()
        || cli_args.bandwidth_size_mb.is_some()
        || cli_args.reconfigure_interval.is_some()
    {
        let changed = {
            let mut config_manager = server.config_manager.lock().await;
            match config_manager.override_and_persist_config(
                cli_args.listen_address,
                cli_args.api_key,
                cli_args.retention_days,
                cli_args.agent_configs_dir,
                cli_args.bandwidth_size_mb,
                cli_args.reconfigure_interval,
            ) {
                Ok(changed) => changed,
                Err(e) => {
                    error!("Failed to apply configuration overrides: {}", e);
                    std::process::exit(1);
                }
            }
        };

        if changed {
            info!("Configuration overrides applied and persisted to disk");
            // Reload the server config to update listen_address if it changed
            let server_config = {
                let config_manager = server.config_manager.lock().await;
                config_manager.server_config.as_ref()
                    .expect("Server configuration not loaded. This should not happen as config is loaded in new().")
                    .clone()
            };
            server.listen_address = server_config
                .listen_address
                .parse()
                .map_err(|e| {
                    error!("Invalid listen address after override: {}", e);
                    std::process::exit(1);
                })
                .unwrap();
        } else {
            info!("Command-line values match existing config, no changes needed");
        }
    }

    // Run the server and the shutdown signal handler concurrently.
    tokio::select! {
        result = server.run() => {
            if let Err(e) = result {
                error!("Server error: {}", e);
                std::process::exit(1);
            }
        }
        _ = setup_shutdown_signal() => {
            info!("Shutdown signal received, initiating graceful shutdown");
        }
    }

    // Perform graceful shutdown.
    server.shutdown().await;
    info!("Server shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_server_creation() {
        // Create a temporary configuration file for the test.
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
listen_address = "127.0.0.1:8787"
api_key = "test-api-key"
data_retention_days = 30
agent_configs_dir = "/tmp/configs"
bandwidth_test_size_mb = 10
"#
        )
        .unwrap();

        let config_path = temp_file.path().to_path_buf();

        // In the current development phase, this test might have different
        // expectations. Here, we are just checking if `Server::new` can be
        // called and if it returns a `Result`.
        let result = Server::new(config_path);

        // As of Phase 1.2, with the `config` module implemented, server creation
        // should succeed if a valid config file is provided.
        assert!(result.is_ok());
    }
}
