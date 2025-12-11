//! Configuration management for the network monitoring central server
//!
//! This module handles loading, validation, and management of server configuration
//! from a `server.toml` file.

use anyhow::{Context, Result};
use shared::config::ServerConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use tracing::{debug, info};

/// The expected name of the configuration file.
const SERVER_CONFIG_FILE: &str = "server.toml";

/// Manages the server's configuration.
/// This struct is responsible for the entire lifecycle of the server's
/// configuration, including initial loading, validation, and reloading.
pub struct ConfigManager {
    /// The full path to the configuration file (e.g., `/etc/networker/server.toml`).
    pub config_path: PathBuf,
    /// The loaded and validated server configuration, wrapped in an `Option`
    /// to represent the unloaded state, although the constructor ensures it's
    /// always `Some` on success.
    pub server_config: Option<ServerConfig>,

    /// Cache: agent_id -> (content, hash, compressed, file mtime, size)
    agent_config_cache: HashMap<String, AgentConfigCache>,
}

/// Cached agent configuration data
struct AgentConfigCache {
    content: String,
    hash: String,
    compressed: String,
    mtime: Duration,
    size: u64,
}

impl ConfigManager {
    /// Creates a new `ConfigManager` and immediately loads the configuration.
    /// This design ensures that a `ConfigManager` instance is always in a valid,
    /// loaded state if successfully created.
    pub fn new(config_path: PathBuf) -> Result<Self> {
        // This logic allows the user to provide either a path to a directory
        // containing `server.toml` or a direct path to the file itself.
        let config_path = if config_path.is_dir() {
            config_path.join(SERVER_CONFIG_FILE)
        } else {
            config_path
        };

        // Fail early if the configuration file doesn't exist.
        if !config_path.exists() {
            return Err(anyhow::anyhow!(
                "Configuration file does not exist: {}",
                config_path.display()
            ));
        }

        let mut manager = Self {
            config_path,
            server_config: None,
            agent_config_cache: HashMap::new(),
        };

        // The configuration is loaded as part of the creation process.
        manager.load_config()?;

        Ok(manager)
    }

    /// Loads the configuration file from disk, parses, and validates it.
    pub fn load_config(&mut self) -> Result<()> {
        info!(
            "Loading server configuration from {}",
            self.config_path.display()
        );

        // Read the file content into a string.
        let config_content = std::fs::read_to_string(&self.config_path)
            .with_context(|| format!("Failed to read {}", self.config_path.display()))?;

        // Deserialize the TOML content into the `ServerConfig` struct.
        let server_config: ServerConfig = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse {}", self.config_path.display()))?;

        // Perform semantic validation on the loaded configuration.
        server_config.validate().with_context(|| {
            format!(
                "Invalid server configuration in {}",
                self.config_path.display()
            )
        })?;

        // Store the valid configuration.
        self.server_config = Some(server_config.clone());

        // Log all server configuration parameters at debug level
        debug!("Server configuration parameters (including defaults):");
        debug!("  listen_address: {}", server_config.listen_address);
        debug!(
            "  api_key: {}",
            if server_config.api_key.is_empty() {
                "<empty>"
            } else {
                "<redacted>"
            }
        );
        debug!(
            "  data_retention_days: {}",
            server_config.data_retention_days
        );
        debug!("  agent_configs_dir: {}", server_config.agent_configs_dir);
        debug!(
            "  bandwidth_test_size_mb: {}",
            server_config.bandwidth_test_size_mb
        );
        debug!(
            "  reconfigure_check_interval_seconds: {}",
            server_config.reconfigure_check_interval_seconds
        );
        debug!(
            "  agent_id_whitelist: {:?}",
            server_config.agent_id_whitelist
        );
        debug!(
            "  cleanup_interval_hours: {}",
            server_config.cleanup_interval_hours
        );
        debug!("  rate_limit_enabled: {}", server_config.rate_limit_enabled);
        debug!(
            "  rate_limit_window_seconds: {}",
            server_config.rate_limit_window_seconds
        );
        debug!(
            "  rate_limit_max_requests: {}",
            server_config.rate_limit_max_requests
        );
        debug!(
            "  bandwidth_test_timeout_seconds: {}",
            server_config.bandwidth_test_timeout_seconds
        );
        debug!(
            "  bandwidth_queue_base_delay_seconds: {}",
            server_config.bandwidth_queue_base_delay_seconds
        );
        debug!(
            "  bandwidth_queue_current_test_delay_seconds: {}",
            server_config.bandwidth_queue_current_test_delay_seconds
        );
        debug!(
            "  bandwidth_queue_position_multiplier_seconds: {}",
            server_config.bandwidth_queue_position_multiplier_seconds
        );
        debug!(
            "  bandwidth_max_delay_seconds: {}",
            server_config.bandwidth_max_delay_seconds
        );
        debug!(
            "  initial_cleanup_delay_seconds: {}",
            server_config.initial_cleanup_delay_seconds
        );
        debug!(
            "  graceful_shutdown_timeout_seconds: {}",
            server_config.graceful_shutdown_timeout_seconds
        );

        let config = self
            .server_config
            .as_ref()
            .expect("Server configuration should be loaded after successful load_config()");

        info!(
            // Log key configuration values for confirmation.
            listen_address = %config.listen_address,
            retention_days = config.data_retention_days,
            config_dir = %config.agent_configs_dir,
            "Server configuration loaded successfully"
        );

        Ok(())
    }

    /// Reloads the configuration from disk and reports if it has changed.
    /// This can be used to apply configuration changes without restarting the server.
    /// Returns `Ok(true)` if the configuration changed, `Ok(false)` otherwise.
    pub fn reload_config(&mut self) -> Result<bool> {
        debug!("Reloading server configuration");

        // Keep a copy of the old configuration to compare against.
        let old_config = self.server_config.clone();

        // Attempt to load the new configuration.
        match self.load_config() {
            Ok(()) => {
                // If loading succeeds, check if the new config is different from the old one.
                if let Some(old) = old_config {
                    let current = self.server_config.as_ref().expect(
                        "Server configuration should be loaded after successful load_config()",
                    );

                    // A simple comparison of key fields. A more robust implementation
                    // might use `PartialEq` on the `ServerConfig` struct.
                    // Clear agent config cache if directory changed
                    if old.agent_configs_dir != current.agent_configs_dir {
                        debug!(
                            "Agent configs directory changed from {} to {}, clearing cache",
                            old.agent_configs_dir, current.agent_configs_dir
                        );
                        self.agent_config_cache.clear();
                    }

                    if old.listen_address != current.listen_address
                        || old.data_retention_days != current.data_retention_days
                        || old.agent_configs_dir != current.agent_configs_dir
                        || old.bandwidth_test_size_mb != current.bandwidth_test_size_mb
                    {
                        info!("Server configuration changed and reloaded");
                        Ok(true)
                    } else {
                        debug!("Server configuration unchanged");
                        Ok(false)
                    }
                } else {
                    // This case handles the first time loading, which is technically a change.
                    info!("Server configuration loaded for first time");
                    Ok(true)
                }
            }
            Err(e) => {
                // If reloading fails, restore the old configuration to ensure the
                // server continues to run with a valid, known state.
                self.server_config = old_config;
                Err(e)
            }
        }
    }

    /// Checks if the configuration is loaded.
    pub fn is_loaded(&self) -> bool {
        self.server_config.is_some()
    }

    /// Validates that the directory for agent configurations exists and is accessible.
    /// This is an important startup check to ensure the server can serve configurations
    /// to agents.
    pub fn validate_agent_configs_dir(&self) -> Result<()> {
        let config = self.server_config.as_ref().expect(
            "Server configuration not loaded. This should not happen as config is loaded in new().",
        );
        let agent_configs_dir = std::path::Path::new(&config.agent_configs_dir);

        // If the directory doesn't exist, try to create it.
        if !agent_configs_dir.exists() {
            info!(
                "Agent configurations directory does not exist, creating: {}",
                agent_configs_dir.display()
            );
            std::fs::create_dir_all(agent_configs_dir).with_context(|| {
                format!(
                    "Failed to create agent configs directory: {}",
                    agent_configs_dir.display()
                )
            })?;
        }

        // Ensure the path points to a directory, not a file.
        if !agent_configs_dir.is_dir() {
            return Err(anyhow::anyhow!(
                "Agent configs path is not a directory: {}",
                agent_configs_dir.display()
            ));
        }

        // Check for read permissions by trying to read the directory's contents.
        match std::fs::read_dir(agent_configs_dir) {
            Ok(_) => {
                debug!("Agent configurations directory is accessible");
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!(
                "Cannot access agent configs directory {}: {}",
                agent_configs_dir.display(),
                e
            )),
        }
    }

    /// Get tasks.toml content for a specific agent (cached)
    pub fn get_agent_tasks_config(&mut self, agent_id: &str) -> Result<String> {
        self.ensure_cache_fresh(agent_id)?;
        Ok(self
            .agent_config_cache
            .get(agent_id)
            .expect("cache populated")
            .content
            .clone())
    }

    /// Calculate BLAKE3 hash of agent's tasks.toml config (cached)
    pub fn get_agent_tasks_config_hash(&mut self, agent_id: &str) -> Result<String> {
        self.ensure_cache_fresh(agent_id)?;
        Ok(self
            .agent_config_cache
            .get(agent_id)
            .expect("cache populated")
            .hash
            .clone())
    }

    /// Get gzipped and base64-encoded tasks.toml for an agent (cached)
    pub fn get_agent_tasks_config_compressed(&mut self, agent_id: &str) -> Result<String> {
        self.ensure_cache_fresh(agent_id)?;
        Ok(self
            .agent_config_cache
            .get(agent_id)
            .expect("cache populated")
            .compressed
            .clone())
    }

    /// Check if cache is fresh; reload if file changed
    fn ensure_cache_fresh(&mut self, agent_id: &str) -> Result<()> {
        let config = self
            .server_config
            .as_ref()
            .expect("Server configuration not loaded");
        let tasks_path = Path::new(&config.agent_configs_dir).join(format!("{agent_id}.toml"));

        if !tasks_path.exists() {
            if self.agent_config_cache.remove(agent_id).is_some() {
                debug!("Cleared stale cache for removed agent config: {}", agent_id);
            }
            return Err(anyhow::anyhow!(
                "Tasks configuration not found for agent {}: {}",
                agent_id,
                tasks_path.display()
            ));
        }

        let metadata = std::fs::metadata(&tasks_path)?;
        let mtime = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let size = metadata.len();

        if let Some(cached) = self.agent_config_cache.get(agent_id) {
            // Check both mtime and size to catch file replacements with same/older mtime
            if cached.mtime == mtime && cached.size == size {
                return Ok(()); // cache hit
            }
        }

        // Cache miss/stale -> reload
        let content = std::fs::read_to_string(&tasks_path)
            .with_context(|| format!("Failed to read {}.toml for agent {}", agent_id, agent_id))?;
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();

        let compressed = {
            use base64::Engine;
            use std::io::Write;
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(content.as_bytes())?;
            base64::engine::general_purpose::STANDARD.encode(encoder.finish()?)
        };

        self.agent_config_cache.insert(
            agent_id.to_string(),
            AgentConfigCache {
                content,
                hash,
                compressed,
                mtime,
                size,
            },
        );

        Ok(())
    }

    /// Override server configuration values and persist to disk
    /// Returns true if any values were changed
    pub fn override_and_persist_config(
        &mut self,
        listen_address: Option<String>,
        api_key: Option<String>,
        retention_days: Option<u32>,
        agent_configs_dir: Option<String>,
        bandwidth_size_mb: Option<u32>,
        reconfigure_interval: Option<u32>,
    ) -> Result<bool> {
        let mut config_changed = false;

        // Load current config if not already loaded
        if self.server_config.is_none() {
            self.load_config()?;
        }

        let mut server_config = self
            .server_config
            .clone()
            .expect("Server configuration must be loaded");

        // Apply overrides
        if let Some(addr) = listen_address {
            if server_config.listen_address != addr {
                info!(
                    "Overriding listen_address: {} -> {}",
                    server_config.listen_address, addr
                );
                server_config.listen_address = addr;
                config_changed = true;
            }
        }

        if let Some(key) = api_key {
            if server_config.api_key != key {
                info!("Overriding api_key (value hidden for security)");
                server_config.api_key = key;
                config_changed = true;
            }
        }

        if let Some(days) = retention_days {
            if server_config.data_retention_days != days {
                info!(
                    "Overriding data_retention_days: {} -> {}",
                    server_config.data_retention_days, days
                );
                server_config.data_retention_days = days;
                config_changed = true;
            }
        }

        if let Some(dir) = agent_configs_dir {
            if server_config.agent_configs_dir != dir {
                info!(
                    "Overriding agent_configs_dir: {} -> {}",
                    server_config.agent_configs_dir, dir
                );
                server_config.agent_configs_dir = dir;
                config_changed = true;
                // Clear cache since directory changed
                self.agent_config_cache.clear();
            }
        }

        if let Some(size) = bandwidth_size_mb {
            if server_config.bandwidth_test_size_mb != size {
                info!(
                    "Overriding bandwidth_test_size_mb: {} -> {}",
                    server_config.bandwidth_test_size_mb, size
                );
                server_config.bandwidth_test_size_mb = size;
                config_changed = true;
            }
        }

        if let Some(interval) = reconfigure_interval {
            if server_config.reconfigure_check_interval_seconds != interval {
                info!(
                    "Overriding reconfigure_check_interval_seconds: {} -> {}",
                    server_config.reconfigure_check_interval_seconds, interval
                );
                server_config.reconfigure_check_interval_seconds = interval;
                config_changed = true;
            }
        }

        // If changes were made, validate and persist
        if config_changed {
            server_config
                .validate()
                .context("Invalid configuration after applying command-line overrides")?;

            // Serialize to TOML
            let server_toml = toml::to_string_pretty(&server_config)
                .context("Failed to serialize server configuration")?;

            // Write to disk
            std::fs::write(&self.config_path, server_toml)
                .with_context(|| format!("Failed to write {}", self.config_path.display()))?;

            // Update in-memory config
            self.server_config = Some(server_config);

            info!("Server configuration updated and persisted to disk");
        }

        Ok(config_changed)
    }
}
