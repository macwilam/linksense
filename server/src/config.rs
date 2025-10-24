//! Configuration management for the network monitoring central server
//!
//! This module handles loading, validation, and management of server configuration
//! from a `server.toml` file.

use anyhow::{Context, Result};
use shared::config::ServerConfig;
use std::path::PathBuf;
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

    /// Get tasks.toml content for a specific agent
    pub fn get_agent_tasks_config(&self, agent_id: &str) -> Result<String> {
        let config = self.server_config.as_ref().expect(
            "Server configuration not loaded. This should not happen as config is loaded in new().",
        );
        let configs_dir = std::path::Path::new(&config.agent_configs_dir);
        let tasks_path = configs_dir.join(format!("{}.toml", agent_id));

        if !tasks_path.exists() {
            return Err(anyhow::anyhow!(
                "Tasks configuration not found for agent {}: {}",
                agent_id,
                tasks_path.display()
            ));
        }

        std::fs::read_to_string(&tasks_path)
            .with_context(|| format!("Failed to read {}.toml for agent {}", agent_id, agent_id))
    }

    /// Calculate BLAKE3 hash of agent's tasks.toml config
    pub fn get_agent_tasks_config_hash(&self, agent_id: &str) -> Result<String> {
        let tasks_content = self.get_agent_tasks_config(agent_id)?;
        let hash = blake3::hash(tasks_content.as_bytes());
        Ok(hash.to_hex().to_string())
    }

    /// Get gzipped and base64-encoded tasks.toml for an agent
    pub fn get_agent_tasks_config_compressed(&self, agent_id: &str) -> Result<String> {
        let tasks_content = self.get_agent_tasks_config(agent_id)?;

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
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&compressed_data);
        Ok(encoded)
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::config::ServerConfig;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    /// Helper function to create a default `ServerConfig` for tests.
    fn create_test_server_config() -> ServerConfig {
        ServerConfig {
            listen_address: "127.0.0.1:8787".to_string(),
            api_key: "test-api-key".to_string(),
            data_retention_days: 30,
            agent_configs_dir: "/tmp/test-configs".to_string(),
            bandwidth_test_size_mb: 10,
            reconfigure_check_interval_seconds: 10,
            agent_id_whitelist: vec![],
            cleanup_interval_hours: 24,
            rate_limit_enabled: true,
            rate_limit_window_seconds: 60,
            rate_limit_max_requests: 100,
            bandwidth_test_timeout_seconds: 120,
            bandwidth_queue_base_delay_seconds: 30,
            bandwidth_queue_current_test_delay_seconds: 60,
            bandwidth_queue_position_multiplier_seconds: 30,
            bandwidth_max_delay_seconds: 300,
            initial_cleanup_delay_seconds: 3600,
            graceful_shutdown_timeout_seconds: 30,
            wal_checkpoint_interval_seconds: 60,
        }
    }

    /// Helper function to write a `ServerConfig` to a file as TOML.
    fn write_config_to_file(config: &ServerConfig, file_path: &std::path::Path) -> Result<()> {
        let toml_content = toml::to_string(config)?;
        std::fs::write(file_path, toml_content)?;
        Ok(())
    }

    #[test]
    fn test_config_manager_with_file_path() {
        // `NamedTempFile` creates a file that is automatically deleted.
        let temp_file = NamedTempFile::new().unwrap();
        let config = create_test_server_config();
        write_config_to_file(&config, temp_file.path()).unwrap();

        let config_manager = ConfigManager::new(temp_file.path().to_path_buf());
        assert!(config_manager.is_ok());

        let manager = config_manager.unwrap();
        assert!(manager.is_loaded());
        assert_eq!(
            manager.server_config.as_ref().unwrap().listen_address,
            "127.0.0.1:8787"
        );
    }

    #[test]
    fn test_config_manager_with_directory() {
        let temp_dir = TempDir::new().unwrap();
        let config_file_path = temp_dir.path().join(SERVER_CONFIG_FILE);
        let config = create_test_server_config();
        write_config_to_file(&config, &config_file_path).unwrap();

        // Test that `new` can correctly find `server.toml` inside a directory.
        let config_manager = ConfigManager::new(temp_dir.path().to_path_buf());
        assert!(config_manager.is_ok());

        let manager = config_manager.unwrap();
        assert!(manager.is_loaded());
        assert_eq!(
            manager.server_config.as_ref().unwrap().data_retention_days,
            30
        );
    }

    #[test]
    fn test_config_manager_nonexistent_file() {
        let config_manager = ConfigManager::new(PathBuf::from("/nonexistent/server.toml"));
        assert!(config_manager.is_err());
    }

    #[test]
    fn test_config_manager_invalid_toml() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "this is not valid toml content [[[").unwrap();

        let config_manager = ConfigManager::new(temp_file.path().to_path_buf());
        // `new` should fail because `load_config` will fail to parse the TOML.
        assert!(config_manager.is_err());
    }

    #[test]
    fn test_config_reload_unchanged() {
        let temp_file = NamedTempFile::new().unwrap();
        let config = create_test_server_config();
        write_config_to_file(&config, temp_file.path()).unwrap();

        let mut config_manager = ConfigManager::new(temp_file.path().to_path_buf()).unwrap();
        let changed = config_manager.reload_config().unwrap();
        assert!(!changed); // Should return false as content is the same.
    }

    #[test]
    fn test_config_reload_changed() {
        let temp_file = NamedTempFile::new().unwrap();
        let config = create_test_server_config();
        write_config_to_file(&config, temp_file.path()).unwrap();

        let mut config_manager = ConfigManager::new(temp_file.path().to_path_buf()).unwrap();

        // Modify the configuration on disk.
        let mut modified_config = config;
        modified_config.data_retention_days = 60;
        write_config_to_file(&modified_config, temp_file.path()).unwrap();

        let changed = config_manager.reload_config().unwrap();
        assert!(changed); // Should detect the change and return true.
        assert_eq!(
            config_manager
                .server_config
                .as_ref()
                .unwrap()
                .data_retention_days,
            60
        );
    }

    #[test]
    fn test_agent_configs_dir_validation() {
        let temp_dir = TempDir::new().unwrap();
        let agent_configs_dir = temp_dir.path().join("agent-configs");

        let mut config = create_test_server_config();
        config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();

        let config_file = temp_dir.path().join(SERVER_CONFIG_FILE);
        write_config_to_file(&config, &config_file).unwrap();

        let config_manager = ConfigManager::new(config_file).unwrap();

        // The directory doesn't exist yet, but the validation function should create it.
        let result = config_manager.validate_agent_configs_dir();
        assert!(result.is_ok());
        assert!(agent_configs_dir.exists());
        assert!(agent_configs_dir.is_dir());
    }

    #[test]
    fn test_get_agent_tasks_config_existing() {
        let temp_dir = TempDir::new().unwrap();
        let agent_configs_dir = temp_dir.path().join("agent-configs");
        std::fs::create_dir_all(&agent_configs_dir).unwrap();

        // Create agent config file
        let tasks_toml = r#"
[[tasks]]
type = "ping"
name = "test-ping"
schedule_seconds = 30
host = "8.8.8.8"
"#;
        std::fs::write(agent_configs_dir.join("agent1.toml"), tasks_toml).unwrap();

        let mut config = create_test_server_config();
        config.api_key = "test-key".to_string();
        config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();

        let server_toml = toml::to_string(&config).unwrap();
        let config_path = temp_dir.path().join("server.toml");
        std::fs::write(&config_path, server_toml).unwrap();

        let config_manager = ConfigManager::new(config_path).unwrap();
        let result = config_manager.get_agent_tasks_config("agent1");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().trim(), tasks_toml.trim());
    }

    #[test]
    fn test_get_agent_tasks_config_missing() {
        let temp_dir = TempDir::new().unwrap();
        let agent_configs_dir = temp_dir.path().join("agent-configs");
        std::fs::create_dir_all(&agent_configs_dir).unwrap();

        let mut config = create_test_server_config();
        config.api_key = "test-key".to_string();
        config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();

        let server_toml = toml::to_string(&config).unwrap();
        let config_path = temp_dir.path().join("server.toml");
        std::fs::write(&config_path, server_toml).unwrap();

        let config_manager = ConfigManager::new(config_path).unwrap();
        let result = config_manager.get_agent_tasks_config("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_agent_tasks_config_hash_consistency() {
        let temp_dir = TempDir::new().unwrap();
        let agent_configs_dir = temp_dir.path().join("agent-configs");
        std::fs::create_dir_all(&agent_configs_dir).unwrap();

        let tasks_toml = "[[tasks]]\ntype = \"ping\"\nname = \"test\"\nschedule_seconds = 30\nhost = \"8.8.8.8\"\n";
        std::fs::write(agent_configs_dir.join("agent1.toml"), tasks_toml).unwrap();

        let config = create_test_server_config();
        let mut config = config;
        config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();
        let server_toml = toml::to_string(&config).unwrap();
        let config_path = temp_dir.path().join("server.toml");
        std::fs::write(&config_path, server_toml).unwrap();

        let config_manager = ConfigManager::new(config_path).unwrap();
        let hash1 = config_manager
            .get_agent_tasks_config_hash("agent1")
            .unwrap();
        let hash2 = config_manager
            .get_agent_tasks_config_hash("agent1")
            .unwrap();

        // Same config should produce same hash
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_get_agent_tasks_config_hash_changes() {
        let temp_dir = TempDir::new().unwrap();
        let agent_configs_dir = temp_dir.path().join("agent-configs");
        std::fs::create_dir_all(&agent_configs_dir).unwrap();

        let tasks_path = agent_configs_dir.join("agent1.toml");
        std::fs::write(&tasks_path, "[[tasks]]\ntype = \"ping\"\n").unwrap();

        let config = create_test_server_config();
        let mut config = config;
        config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();
        let server_toml = toml::to_string(&config).unwrap();
        let config_path = temp_dir.path().join("server.toml");
        std::fs::write(&config_path, server_toml).unwrap();

        let config_manager = ConfigManager::new(config_path).unwrap();
        let hash1 = config_manager
            .get_agent_tasks_config_hash("agent1")
            .unwrap();

        // Modify config
        std::fs::write(&tasks_path, "[[tasks]]\ntype = \"http_get\"\n").unwrap();
        let hash2 = config_manager
            .get_agent_tasks_config_hash("agent1")
            .unwrap();

        // Hash should change
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_get_agent_tasks_config_compressed_valid() {
        let temp_dir = TempDir::new().unwrap();
        let agent_configs_dir = temp_dir.path().join("agent-configs");
        std::fs::create_dir_all(&agent_configs_dir).unwrap();

        let tasks_toml = "[[tasks]]\ntype = \"ping\"\nname = \"test\"\nschedule_seconds = 30\nhost = \"8.8.8.8\"\n";
        std::fs::write(agent_configs_dir.join("agent1.toml"), tasks_toml).unwrap();

        let config = create_test_server_config();
        let mut config = config;
        config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();
        let server_toml = toml::to_string(&config).unwrap();
        let config_path = temp_dir.path().join("server.toml");
        std::fs::write(&config_path, server_toml).unwrap();

        let config_manager = ConfigManager::new(config_path).unwrap();
        let compressed = config_manager
            .get_agent_tasks_config_compressed("agent1")
            .unwrap();

        // Decode and decompress to verify
        use base64::Engine;
        let compressed_data = base64::engine::general_purpose::STANDARD
            .decode(&compressed)
            .unwrap();
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(&compressed_data[..]);
        let mut decompressed = String::new();
        decoder.read_to_string(&mut decompressed).unwrap();

        assert_eq!(decompressed, tasks_toml);
    }

    #[test]
    fn test_override_and_persist_config_single_field() {
        let temp_file = NamedTempFile::new().unwrap();
        let config = create_test_server_config();
        write_config_to_file(&config, temp_file.path()).unwrap();

        let mut config_manager = ConfigManager::new(temp_file.path().to_path_buf()).unwrap();
        assert_eq!(
            config_manager
                .server_config
                .as_ref()
                .unwrap()
                .listen_address,
            "127.0.0.1:8787"
        );

        // Override only listen_address
        let changed = config_manager
            .override_and_persist_config(
                Some("0.0.0.0:9090".to_string()),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        assert!(changed);
        assert_eq!(
            config_manager
                .server_config
                .as_ref()
                .unwrap()
                .listen_address,
            "0.0.0.0:9090"
        );

        // Verify persisted to disk
        let config_manager2 = ConfigManager::new(temp_file.path().to_path_buf()).unwrap();
        assert_eq!(
            config_manager2
                .server_config
                .as_ref()
                .unwrap()
                .listen_address,
            "0.0.0.0:9090"
        );
        assert_eq!(
            config_manager2.server_config.as_ref().unwrap().api_key,
            "test-api-key"
        );
    }

    #[test]
    fn test_override_and_persist_config_no_changes() {
        let temp_file = NamedTempFile::new().unwrap();
        let config = create_test_server_config();
        write_config_to_file(&config, temp_file.path()).unwrap();

        let mut config_manager = ConfigManager::new(temp_file.path().to_path_buf()).unwrap();

        // Override with same values
        let changed = config_manager
            .override_and_persist_config(
                Some("127.0.0.1:8787".to_string()),
                Some("test-api-key".to_string()),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // Should return false (no changes)
        assert!(!changed);
    }

    #[test]
    fn test_override_and_persist_config_multiple_fields() {
        let temp_file = NamedTempFile::new().unwrap();
        let config = create_test_server_config();
        write_config_to_file(&config, temp_file.path()).unwrap();

        let mut config_manager = ConfigManager::new(temp_file.path().to_path_buf()).unwrap();

        // Override multiple fields
        let changed = config_manager
            .override_and_persist_config(
                Some("0.0.0.0:9090".to_string()),
                Some("new-api-key".to_string()),
                Some(60),
                Some("/new/path".to_string()),
                Some(50),
                Some(20),
            )
            .unwrap();

        assert!(changed);
        let server_config = config_manager.server_config.as_ref().unwrap();
        assert_eq!(server_config.listen_address, "0.0.0.0:9090");
        assert_eq!(server_config.api_key, "new-api-key");
        assert_eq!(server_config.data_retention_days, 60);
        assert_eq!(server_config.agent_configs_dir, "/new/path");
        assert_eq!(server_config.bandwidth_test_size_mb, 50);
        assert_eq!(server_config.reconfigure_check_interval_seconds, 20);
    }
}
