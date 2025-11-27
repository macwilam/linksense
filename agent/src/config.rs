//! Configuration management for the network monitoring agent
//!
//! This module handles loading, validation, and management of agent configuration
//! files (agent.toml and tasks.toml).
// The primary responsibility of this module is to abstract away the details of
// configuration file handling, providing a clean interface for the rest of the
// agent to access configuration values. It also implements a mechanism to detect
// configuration changes and reload them dynamically.

use anyhow::{Context, Result};
use shared::{
    config::{AgentConfig, TasksConfig},
    utils::calculate_checksum,
};
use std::path::PathBuf;
use tracing::{debug, info};

/// Configuration file names are defined as constants to avoid magic strings
/// and make it easier to change them in one place if needed.
const AGENT_CONFIG_FILE: &str = "agent.toml";
const TASKS_CONFIG_FILE: &str = "tasks.toml";

/// Manages agent configuration loading and validation.
/// This struct holds the state of the agent's configuration, including the
/// loaded configuration data, the directory where configs are stored, and a
/// checksum to quickly verify if the configuration has changed.
pub struct ConfigManager {
    /// Directory containing configuration files. Using PathBuf for flexible path manipulation.
    pub config_dir: PathBuf,
    /// Loaded agent configuration, wrapped in an Option to represent the unloaded state.
    pub agent_config: Option<AgentConfig>,
    /// Loaded tasks configuration, also optional.
    pub tasks_config: Option<TasksConfig>,
    /// A checksum of the configuration files' content. This is used to efficiently
    /// check for changes without parsing the files every time. It's an Option
    /// to handle the initial state before any configuration is loaded.
    pub current_checksum: Option<String>,
}

impl ConfigManager {
    /// Create a new configuration manager.
    /// This constructor initializes the manager with the path to the configuration directory.
    /// It performs essential validations to ensure the path exists and is a directory,
    /// failing early if the basic requirements are not met.
    pub fn new(config_dir: PathBuf) -> Result<Self> {
        // It's crucial to check for the existence of the config directory at startup
        // to provide a clear error message if it's misconfigured.
        if !config_dir.exists() {
            return Err(anyhow::anyhow!(
                "Configuration directory does not exist: {}",
                config_dir.display()
            ));
        }

        // Equally important is to ensure that the provided path is a directory, not a file.
        if !config_dir.is_dir() {
            return Err(anyhow::anyhow!(
                "Configuration path is not a directory: {}",
                config_dir.display()
            ));
        }

        // If validations pass, return a new instance of ConfigManager in its initial state.
        Ok(Self {
            config_dir,
            agent_config: None,
            tasks_config: None,
            current_checksum: None,
        })
    }

    /// Load configuration files from disk.
    /// This is the initial loading process. It reads both `agent.toml` and `tasks.toml`,
    /// parses them, validates their contents, and if successful, stores them in the
    /// ConfigManager instance along with a calculated checksum.
    pub async fn load_config(&mut self) -> Result<()> {
        info!(
            "Loading agent configuration from {}",
            self.config_dir.display()
        );

        // Load agent.toml
        let agent_config_path = self.config_dir.join(AGENT_CONFIG_FILE);
        // Asynchronous file read is used here, suitable for a tokio-based application.
        let agent_toml_content = tokio::fs::read_to_string(&agent_config_path)
            .await
            // `with_context` from `anyhow` provides better error messages, explaining what failed.
            .with_context(|| format!("Failed to read {}", agent_config_path.display()))?;

        // Parse the TOML content into the AgentConfig struct.
        let agent_config: AgentConfig = toml::from_str(&agent_toml_content).with_context(|| {
            format!(
                "Failed to parse {} - TOML syntax error in agent configuration file",
                agent_config_path.display()
            )
        })?;

        // After parsing, it's important to perform semantic validation of the configuration.
        // This is handled by the `validate` method on the AgentConfig struct itself.
        agent_config.validate().with_context(|| {
            format!(
                "Validation failed for agent configuration in {}",
                agent_config_path.display()
            )
        })?;

        // Load tasks.toml, following the same pattern as for agent.toml.
        let tasks_config_path = self.config_dir.join(TASKS_CONFIG_FILE);
        let tasks_toml_content = tokio::fs::read_to_string(&tasks_config_path)
            .await
            .with_context(|| format!("Failed to read {}", tasks_config_path.display()))?;

        let tasks_config: TasksConfig = toml::from_str(&tasks_toml_content).with_context(|| {
            format!(
                "Failed to parse {} - TOML syntax error in tasks configuration file",
                tasks_config_path.display()
            )
        })?;

        // Validate the tasks configuration.
        tasks_config.validate().with_context(|| {
            format!(
                "Validation failed for tasks configuration in {}",
                tasks_config_path.display()
            )
        })?;

        // Calculate a checksum from the raw content of both configuration files.
        // This checksum will be used for quick change detection in `reload_config`.
        let checksum = calculate_checksum(&agent_toml_content, &tasks_toml_content);

        // If all steps are successful, store the loaded configurations and checksum.
        self.agent_config = Some(agent_config.clone());
        self.tasks_config = Some(tasks_config);
        self.current_checksum = Some(checksum);

        // Log all agent configuration parameters at debug level
        debug!("Agent configuration parameters (including defaults):");
        debug!("  agent_id: {}", agent_config.agent_id);
        debug!("  central_server_url: {}", agent_config.central_server_url);
        debug!(
            "  api_key: {}",
            if agent_config.api_key.is_empty() {
                "<empty>"
            } else {
                "<redacted>"
            }
        );
        debug!(
            "  local_data_retention_days: {}",
            agent_config.local_data_retention_days
        );
        debug!("  auto_update_tasks: {}", agent_config.auto_update_tasks);
        debug!("  local_only: {}", agent_config.local_only);
        debug!(
            "  metrics_flush_interval_seconds: {}",
            agent_config.metrics_flush_interval_seconds
        );
        debug!(
            "  metrics_send_interval_seconds: {}",
            agent_config.metrics_send_interval_seconds
        );
        debug!("  metrics_batch_size: {}", agent_config.metrics_batch_size);
        debug!(
            "  metrics_max_retries: {}",
            agent_config.metrics_max_retries
        );
        debug!(
            "  queue_cleanup_interval_seconds: {}",
            agent_config.queue_cleanup_interval_seconds
        );
        debug!(
            "  data_cleanup_interval_seconds: {}",
            agent_config.data_cleanup_interval_seconds
        );
        debug!(
            "  max_concurrent_tasks: {}",
            agent_config.max_concurrent_tasks
        );
        debug!(
            "  http_response_max_size_mb: {}",
            agent_config.http_response_max_size_mb
        );
        debug!(
            "  http_client_timeout_seconds: {}",
            agent_config.http_client_timeout_seconds
        );
        debug!(
            "  database_busy_timeout_seconds: {}",
            agent_config.database_busy_timeout_seconds
        );
        debug!(
            "  graceful_shutdown_timeout_seconds: {}",
            agent_config.graceful_shutdown_timeout_seconds
        );
        debug!(
            "  channel_buffer_size: {}",
            agent_config.channel_buffer_size
        );

        info!(
            // Structured logging provides better machine-readable logs.
            agent_id = %self.agent_config.as_ref().unwrap().agent_id,
            task_count = self.tasks_config.as_ref().unwrap().tasks.len(),
            checksum = %self.current_checksum.as_ref().unwrap(),
            "Configuration loaded successfully"
        );

        Ok(())
    }

    /// Reload configuration from disk if it has changed.
    /// This method is designed to be called periodically to check for configuration updates.
    /// It recalculates the checksum of the config files and compares it with the stored one.
    /// If they differ, it reloads and validates the configuration.
    /// Returns `Ok(true)` if the configuration was changed, `Ok(false)` otherwise.
    pub async fn reload_config(&mut self) -> Result<bool> {
        debug!("Checking for configuration changes");

        // Read the current content of the configuration files.
        let agent_config_path = self.config_dir.join(AGENT_CONFIG_FILE);
        let tasks_config_path = self.config_dir.join(TASKS_CONFIG_FILE);

        let agent_toml_content = tokio::fs::read_to_string(&agent_config_path)
            .await
            .with_context(|| format!("Failed to read {}", agent_config_path.display()))?;

        let tasks_toml_content = tokio::fs::read_to_string(&tasks_config_path)
            .await
            .with_context(|| format!("Failed to read {}", tasks_config_path.display()))?;

        // Calculate a new checksum from the current file contents.
        let new_checksum = calculate_checksum(&agent_toml_content, &tasks_toml_content);

        // Compare the new checksum with the existing one.
        if let Some(current_checksum) = &self.current_checksum {
            if &new_checksum == current_checksum {
                // If checksums match, no changes are needed.
                debug!("Configuration unchanged");
                return Ok(false);
            }
        }

        info!("Configuration change detected, reloading");

        // If checksums differ, parse and validate the new configuration.
        // This process is identical to `load_config`.
        let agent_config: AgentConfig = toml::from_str(&agent_toml_content)
            .with_context(|| format!("Failed to parse {}", agent_config_path.display()))?;

        agent_config.validate().with_context(|| {
            format!(
                "Invalid agent configuration in {}",
                agent_config_path.display()
            )
        })?;

        let tasks_config: TasksConfig = toml::from_str(&tasks_toml_content)
            .with_context(|| format!("Failed to parse {}", tasks_config_path.display()))?;

        tasks_config.validate().with_context(|| {
            format!(
                "Invalid tasks configuration in {}",
                tasks_config_path.display()
            )
        })?;

        // Update the stored configuration with the newly loaded data.
        self.agent_config = Some(agent_config);
        self.tasks_config = Some(tasks_config);
        self.current_checksum = Some(new_checksum);

        info!(
            agent_id = %self.agent_config.as_ref().unwrap().agent_id,
            task_count = self.tasks_config.as_ref().unwrap().tasks.len(),
            "Configuration reloaded successfully"
        );

        Ok(true)
    }

    /// Calculate BLAKE3 hash of just the tasks.toml content
    pub fn get_tasks_config_hash(&self) -> Result<String> {
        let tasks_path = self.config_dir.join("tasks.toml");
        if !tasks_path.exists() {
            return Err(anyhow::anyhow!("tasks.toml not found"));
        }

        let tasks_content = std::fs::read_to_string(&tasks_path)
            .with_context(|| format!("Failed to read tasks.toml from {}", tasks_path.display()))?;

        let hash = blake3::hash(tasks_content.as_bytes());
        Ok(hash.to_hex().to_string())
    }

    /// Get the raw tasks.toml content as a string
    pub fn get_tasks_config_content(&self) -> Result<String> {
        let tasks_path = self.config_dir.join("tasks.toml");
        if !tasks_path.exists() {
            return Err(anyhow::anyhow!("tasks.toml not found"));
        }

        let tasks_content = std::fs::read_to_string(&tasks_path)
            .with_context(|| format!("Failed to read tasks.toml from {}", tasks_path.display()))?;

        Ok(tasks_content)
    }

    /// Check if the configuration has been loaded.
    /// This provides a non-panicking way to check if the configuration is available.
    pub fn is_loaded(&self) -> bool {
        self.agent_config.is_some() && self.tasks_config.is_some()
    }

    /// Override agent configuration values and persist to disk
    /// Returns true if any values were changed
    pub async fn override_and_persist_agent_config(
        &mut self,
        agent_id: Option<String>,
        server_url: Option<String>,
        api_key: Option<String>,
        retention_days: Option<u32>,
        auto_update_tasks: Option<bool>,
        local_only: Option<bool>,
    ) -> Result<bool> {
        let mut config_changed = false;
        let agent_config_path = self.config_dir.join(AGENT_CONFIG_FILE);

        // Load current config if not already loaded
        if self.agent_config.is_none() {
            self.load_config().await?;
        }

        let mut agent_config = self
            .agent_config
            .clone()
            .expect("Agent configuration must be loaded");

        // Apply overrides
        if let Some(id) = agent_id {
            if agent_config.agent_id != id {
                info!("Overriding agent_id: {} -> {}", agent_config.agent_id, id);
                agent_config.agent_id = id;
                config_changed = true;
            }
        }

        if let Some(url) = server_url {
            if agent_config.central_server_url != url {
                info!(
                    "Overriding central_server_url: {} -> {}",
                    agent_config.central_server_url, url
                );
                agent_config.central_server_url = url;
                config_changed = true;
            }
        }

        if let Some(key) = api_key {
            if agent_config.api_key != key {
                info!("Overriding api_key (value hidden for security)");
                agent_config.api_key = key;
                config_changed = true;
            }
        }

        if let Some(days) = retention_days {
            if agent_config.local_data_retention_days != days {
                info!(
                    "Overriding local_data_retention_days: {} -> {}",
                    agent_config.local_data_retention_days, days
                );
                agent_config.local_data_retention_days = days;
                config_changed = true;
            }
        }

        if let Some(auto_update) = auto_update_tasks {
            if agent_config.auto_update_tasks != auto_update {
                info!(
                    "Overriding auto_update_tasks: {} -> {}",
                    agent_config.auto_update_tasks, auto_update
                );
                agent_config.auto_update_tasks = auto_update;
                config_changed = true;
            }
        }

        if let Some(local) = local_only {
            if agent_config.local_only != local {
                info!(
                    "Overriding local_only: {} -> {}",
                    agent_config.local_only, local
                );
                agent_config.local_only = local;
                config_changed = true;
            }
        }

        // If changes were made, validate and persist
        if config_changed {
            agent_config
                .validate()
                .context("Invalid configuration after applying command-line overrides")?;

            // Serialize to TOML
            let agent_toml = toml::to_string_pretty(&agent_config)
                .context("Failed to serialize agent configuration")?;

            // Write to disk
            tokio::fs::write(&agent_config_path, agent_toml)
                .await
                .with_context(|| format!("Failed to write {}", agent_config_path.display()))?;

            // Update in-memory config
            self.agent_config = Some(agent_config);

            info!("Agent configuration updated and persisted to disk");
        }

        Ok(config_changed)
    }

    /// Replace tasks.toml with new content, backing up the old one
    pub async fn update_tasks_config(&mut self, new_tasks_toml: &str) -> Result<()> {
        // First, validate that the new config is parseable
        let _: TasksConfig =
            toml::from_str(new_tasks_toml).context("New tasks.toml content is not valid TOML")?;

        // Ensure previous_configs directory exists
        let previous_configs_dir = self.config_dir.join("previous_configs");
        if !previous_configs_dir.exists() {
            std::fs::create_dir_all(&previous_configs_dir).with_context(|| {
                format!(
                    "Failed to create previous_configs directory: {}",
                    previous_configs_dir.display()
                )
            })?;
        }

        let tasks_path = self.config_dir.join("tasks.toml");

        // If current tasks.toml exists, back it up
        if tasks_path.exists() {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let backup_name = format!("tasks.toml.{}", timestamp);
            let backup_path = previous_configs_dir.join(backup_name);

            std::fs::copy(&tasks_path, &backup_path).with_context(|| {
                format!(
                    "Failed to backup current tasks.toml to {}",
                    backup_path.display()
                )
            })?;

            info!("Backed up current tasks.toml to {}", backup_path.display());
        }

        // Write new config
        std::fs::write(&tasks_path, new_tasks_toml).with_context(|| {
            format!("Failed to write new tasks.toml to {}", tasks_path.display())
        })?;

        info!("Updated tasks.toml with new configuration");

        // Reload configuration
        self.load_config().await?;

        Ok(())
    }
}
