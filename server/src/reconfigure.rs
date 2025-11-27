//! Multi-agent reconfiguration functionality
//!
//! This module monitors the "reconfigure" folder and applies configuration
//! changes to multiple agents when triggered by the presence of configuration files.

use anyhow::{Context, Result};
use shared::{config::TasksConfig, utils::validate_agent_id};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::fs;
use tracing::{debug, error, info, warn};

/// File names in the reconfigure folder
const AGENT_LIST_FILE: &str = "agent_list.txt";
const TASKS_CONFIG_FILE: &str = "tasks.toml";
const ERROR_FILE: &str = "error.txt";

/// Marker for reconfiguring all agents
const ALL_AGENTS_MARKER: &str = "ALL AGENTS";

/// Maximum number of backup files to keep per agent
const MAX_BACKUP_FILES: usize = 10;

/// Manages the reconfiguration process for multiple agents
pub struct ReconfigureManager {
    /// Path to the reconfigure folder
    reconfigure_dir: PathBuf,
    /// Path to the agent configs directory
    agent_configs_dir: PathBuf,
}

impl ReconfigureManager {
    /// Create a new reconfigure manager.
    ///
    /// Ensures the reconfigure directory exists, creating it if necessary.
    ///
    /// # Parameters
    /// * `reconfigure_dir` - Path to the reconfigure folder
    /// * `agent_configs_dir` - Path to the agent configs directory
    ///
    /// # Returns
    /// `ReconfigureManager` instance or error if directory creation fails
    pub fn new(reconfigure_dir: PathBuf, agent_configs_dir: PathBuf) -> Result<Self> {
        // Ensure reconfigure directory exists
        if !reconfigure_dir.exists() {
            std::fs::create_dir_all(&reconfigure_dir).with_context(|| {
                format!(
                    "Failed to create reconfigure directory: {}",
                    reconfigure_dir.display()
                )
            })?;
            info!(
                "Created reconfigure directory: {}",
                reconfigure_dir.display()
            );
        }

        Ok(Self {
            reconfigure_dir,
            agent_configs_dir,
        })
    }

    /// Check for reconfiguration requests and process them.
    ///
    /// This is the main entry point called periodically. It checks if the
    /// reconfigure folder contains files, and if so, processes the reconfiguration
    /// request. Errors are written to error.txt and successful requests are cleaned up.
    ///
    /// # Returns
    /// `Ok(())` always - errors are caught and written to error.txt
    pub async fn check_and_process(&mut self) -> Result<()> {
        debug!("Checking reconfigure folder for new requests");

        // Check if folder is empty
        if self.is_reconfigure_folder_empty().await? {
            return Ok(());
        }

        info!("Reconfigure folder contains files, processing reconfiguration request");

        // Clear any previous error file
        self.clear_error_file().await?;

        // Process the reconfiguration
        match self.process_reconfiguration().await {
            Ok(()) => {
                info!("Reconfiguration completed successfully");
                self.cleanup_processed_files().await?;
            }
            Err(e) => {
                error!("Reconfiguration failed: {}", e);
                self.write_error(&e.to_string()).await?;
            }
        }

        Ok(())
    }

    /// Check if the reconfigure folder is empty.
    ///
    /// This is used to determine if there are any reconfiguration requests pending.
    ///
    /// # Returns
    /// `Ok(true)` if folder is empty, `Ok(false)` if files exist, error on I/O failure
    pub(crate) async fn is_reconfigure_folder_empty(&self) -> Result<bool> {
        let mut entries = fs::read_dir(&self.reconfigure_dir).await?;
        Ok(entries.next_entry().await?.is_none())
    }

    /// Process the reconfiguration request.
    ///
    /// This is the main orchestration method that:
    /// 1. Validates and reads tasks.toml
    /// 2. Validates and reads agent_list.txt
    /// 3. Applies configuration to all specified agents
    ///
    /// # Returns
    /// `Ok(())` on success, error if validation or application fails
    async fn process_reconfiguration(&self) -> Result<()> {
        // Step 1: Validate tasks.toml
        let tasks_content = self.read_and_validate_tasks_config().await?;

        // Step 2: Validate agent_list.txt
        let agent_ids = self.read_and_validate_agent_list().await?;

        // Step 3: Apply configuration to specified agents
        self.apply_configuration_to_agents(&tasks_content, &agent_ids)
            .await?;

        Ok(())
    }

    /// Read and validate the tasks.toml file.
    ///
    /// Reads the tasks configuration from the reconfigure folder and validates
    /// it against the TasksConfig schema.
    ///
    /// # Returns
    /// The validated tasks configuration as a string, or error if file missing or invalid
    pub(crate) async fn read_and_validate_tasks_config(&self) -> Result<String> {
        let tasks_path = self.reconfigure_dir.join(TASKS_CONFIG_FILE);

        if !tasks_path.exists() {
            return Err(anyhow::anyhow!(
                "Required file '{}' not found in reconfigure folder",
                TASKS_CONFIG_FILE
            ));
        }

        let tasks_content = fs::read_to_string(&tasks_path)
            .await
            .with_context(|| format!("Failed to read {}", tasks_path.display()))?;

        // Validate the tasks configuration
        TasksConfig::validate_from_toml(&tasks_content)
            .with_context(|| "Tasks configuration validation failed")?;

        info!("Tasks configuration validated successfully");
        Ok(tasks_content)
    }

    /// Read and validate the agent_list.txt file.
    ///
    /// Reads the agent list, validates agent ID format, checks for duplicates,
    /// and handles the special "ALL AGENTS" marker.
    ///
    /// # Returns
    /// Vector of validated agent IDs, or error if file missing/invalid
    pub(crate) async fn read_and_validate_agent_list(&self) -> Result<Vec<String>> {
        let agent_list_path = self.reconfigure_dir.join(AGENT_LIST_FILE);

        if !agent_list_path.exists() {
            return Err(anyhow::anyhow!(
                "Required file '{}' not found in reconfigure folder",
                AGENT_LIST_FILE
            ));
        }

        let agent_list_content = fs::read_to_string(&agent_list_path)
            .await
            .with_context(|| format!("Failed to read {}", agent_list_path.display()))?;

        let lines: Vec<String> = agent_list_content
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();

        if lines.is_empty() {
            return Err(anyhow::anyhow!("Agent list file is empty"));
        }

        // Check for "ALL AGENTS" marker
        if lines.len() == 1 && lines[0] == ALL_AGENTS_MARKER {
            info!("Reconfiguration requested for ALL AGENTS");
            return self.get_all_agent_ids().await;
        }

        // Validate individual agent IDs
        let mut agent_ids = Vec::new();
        let mut seen_ids = HashSet::new();

        for (line_num, line) in lines.iter().enumerate() {
            // Check for duplicate agent IDs
            if !seen_ids.insert(line.clone()) {
                return Err(anyhow::anyhow!(
                    "Duplicate agent ID '{}' found at line {}",
                    line,
                    line_num + 1
                ));
            }

            // Validate agent ID format
            validate_agent_id(line)
                .with_context(|| format!("Invalid agent ID '{}' at line {}", line, line_num + 1))?;

            agent_ids.push(line.clone());
        }

        info!(
            "Agent list validated successfully with {} agents",
            agent_ids.len()
        );
        Ok(agent_ids)
    }

    /// Get all existing agent IDs from the agent configs directory.
    ///
    /// Scans the agent configs directory for .toml files and extracts
    /// valid agent IDs from filenames. Only includes IDs that pass validation.
    ///
    /// # Returns
    /// Vector of agent IDs, or error if no valid agents found
    async fn get_all_agent_ids(&self) -> Result<Vec<String>> {
        let mut agent_ids = Vec::new();
        let mut entries = fs::read_dir(&self.agent_configs_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Skip if not a file
            if !path.is_file() {
                continue;
            }

            // Extract file name, skip if invalid UTF-8
            let Some(file_name) = path.file_name() else {
                continue;
            };
            let Some(file_name_str) = file_name.to_str() else {
                continue;
            };

            // Only process .toml files
            if !file_name_str.ends_with(".toml") {
                continue;
            }

            // Extract agent ID and validate
            let agent_id = file_name_str.trim_end_matches(".toml");
            if validate_agent_id(agent_id).is_ok() {
                agent_ids.push(agent_id.to_string());
            }
        }

        if agent_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "No agent configuration files found in {}",
                self.agent_configs_dir.display()
            ));
        }

        info!("Found {} existing agent configurations", agent_ids.len());
        Ok(agent_ids)
    }

    /// Apply configuration to the specified agents.
    ///
    /// Attempts to update all agents in the list. If any agents fail, collects
    /// all errors and returns a combined error message with partial success count.
    ///
    /// # Parameters
    /// * `tasks_content` - The validated tasks configuration content
    /// * `agent_ids` - List of agent IDs to update
    ///
    /// # Returns
    /// `Ok(())` if all agents updated successfully, error with details on partial failure
    async fn apply_configuration_to_agents(
        &self,
        tasks_content: &str,
        agent_ids: &[String],
    ) -> Result<()> {
        let mut success_count = 0;
        let mut errors = Vec::new();

        for agent_id in agent_ids {
            match self
                .apply_configuration_to_agent(agent_id, tasks_content)
                .await
            {
                Ok(()) => {
                    success_count += 1;
                    info!(
                        "Successfully updated configuration for agent '{}'",
                        agent_id
                    );
                }
                Err(e) => {
                    let error_msg = format!("Failed to update agent '{}': {}", agent_id, e);
                    error!("{}", error_msg);
                    errors.push(error_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Reconfiguration partially failed. Updated {} agents, failed for {} agents. Errors: {}",
                success_count,
                errors.len(),
                errors.join("; ")
            ));
        }

        info!(
            "Successfully updated configuration for all {} agents",
            success_count
        );
        Ok(())
    }

    /// Apply configuration to a single agent.
    ///
    /// Backs up the existing configuration and writes the new configuration
    /// to the agent's config file.
    ///
    /// # Parameters
    /// * `agent_id` - The agent ID to update
    /// * `tasks_content` - The new tasks configuration content
    ///
    /// # Returns
    /// `Ok(())` on success, error if agent not found or write fails
    async fn apply_configuration_to_agent(
        &self,
        agent_id: &str,
        tasks_content: &str,
    ) -> Result<()> {
        let agent_config_path = self.agent_configs_dir.join(format!("{}.toml", agent_id));

        // Check if agent config file exists
        if !agent_config_path.exists() {
            return Err(anyhow::anyhow!(
                "Agent configuration file not found: {}",
                agent_config_path.display()
            ));
        }

        // Create backup of existing config
        self.backup_agent_config(&agent_config_path).await?;

        // Write new configuration
        fs::write(&agent_config_path, tasks_content)
            .await
            .with_context(|| format!("Failed to write configuration for agent '{}'", agent_id))?;

        debug!(
            "Updated configuration file: {}",
            agent_config_path.display()
        );
        Ok(())
    }

    /// Create backup of agent configuration.
    ///
    /// Creates a timestamped backup copy of the agent's configuration file
    /// before it is modified.
    ///
    /// # Parameters
    /// * `config_path` - Path to the configuration file to backup
    ///
    /// # Returns
    /// `Ok(())` on success, error if backup creation fails
    pub(crate) async fn backup_agent_config(&self, config_path: &Path) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        // Safe error handling instead of unwrap()
        let file_name = config_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid config path: no file name"))?
            .to_string_lossy();

        let backup_name = format!("{}.backup.{}", file_name, timestamp);

        let parent_dir = config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid config path: no parent directory"))?;

        let backup_path = parent_dir.join(backup_name);

        fs::copy(config_path, &backup_path).await.with_context(|| {
            format!(
                "Failed to backup configuration to {}",
                backup_path.display()
            )
        })?;

        debug!("Created backup: {}", backup_path.display());

        // Clean up old backups
        self.cleanup_old_backups(parent_dir, &file_name).await?;

        Ok(())
    }

    /// Remove old backup files, keeping only the most recent MAX_BACKUP_FILES.
    ///
    /// # Parameters
    /// * `dir` - Directory containing backup files
    /// * `base_name` - Base filename to match (e.g., "agent1.toml")
    ///
    /// # Returns
    /// `Ok(())` on success, logs warnings on individual file removal failures
    async fn cleanup_old_backups(&self, dir: &Path, base_name: &str) -> Result<()> {
        let backup_prefix = format!("{}.backup.", base_name);
        let mut backups = Vec::new();

        // Find all backup files for this config
        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with(&backup_prefix) {
                    // Extract timestamp from backup filename
                    if let Some(timestamp_str) = file_name.strip_prefix(&backup_prefix) {
                        if let Ok(timestamp) = timestamp_str.parse::<u128>() {
                            backups.push((timestamp, path));
                        }
                    }
                }
            }
        }

        // Sort by timestamp (oldest first)
        backups.sort_by_key(|(ts, _)| *ts);

        // Remove old backups if we exceed the limit
        if backups.len() > MAX_BACKUP_FILES {
            let to_remove = backups.len() - MAX_BACKUP_FILES;
            for (_, path) in backups.iter().take(to_remove) {
                match fs::remove_file(path).await {
                    Ok(()) => debug!("Removed old backup: {}", path.display()),
                    Err(e) => warn!("Failed to remove old backup {}: {}", path.display(), e),
                }
            }
            info!("Cleaned up {} old backup files", to_remove);
        }

        Ok(())
    }

    /// Write error message to error.txt file.
    ///
    /// Appends the error with a timestamp to the error file, preserving
    /// history of reconfiguration failures.
    ///
    /// # Parameters
    /// * `error_message` - The error message to write
    ///
    /// # Returns
    /// `Ok(())` on success, error if file write fails
    pub(crate) async fn write_error(&self, error_message: &str) -> Result<()> {
        use tokio::fs::OpenOptions;
        use tokio::io::AsyncWriteExt;

        let error_path = self.reconfigure_dir.join(ERROR_FILE);
        let timestamp = chrono::Utc::now().to_rfc3339();
        let error_content = format!("[{}] {}\n", timestamp, error_message);

        // Append to error file instead of overwriting
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&error_path)
            .await
            .with_context(|| format!("Failed to open error file: {}", error_path.display()))?;

        file.write_all(error_content.as_bytes())
            .await
            .with_context(|| format!("Failed to write error file: {}", error_path.display()))?;

        warn!("Error appended to: {}", error_path.display());
        Ok(())
    }

    /// Clear any existing error file.
    ///
    /// Removes the error.txt file if it exists, typically called at the start
    /// of a new reconfiguration attempt.
    ///
    /// # Returns
    /// `Ok(())` on success or if file doesn't exist, error if removal fails
    async fn clear_error_file(&self) -> Result<()> {
        let error_path = self.reconfigure_dir.join(ERROR_FILE);
        if error_path.exists() {
            fs::remove_file(&error_path).await.with_context(|| {
                format!("Failed to remove error file: {}", error_path.display())
            })?;
            debug!("Cleared previous error file");
        }
        Ok(())
    }

    /// Clean up processed files after successful reconfiguration.
    ///
    /// Removes agent_list.txt and tasks.toml from the reconfigure folder
    /// after successful processing.
    ///
    /// # Returns
    /// `Ok(())` on success, error if file removal fails
    async fn cleanup_processed_files(&self) -> Result<()> {
        let files_to_remove = [AGENT_LIST_FILE, TASKS_CONFIG_FILE];

        for file_name in &files_to_remove {
            let file_path = self.reconfigure_dir.join(file_name);
            if file_path.exists() {
                fs::remove_file(&file_path).await.with_context(|| {
                    format!("Failed to remove processed file: {}", file_path.display())
                })?;
                debug!("Removed processed file: {}", file_path.display());
            }
        }

        info!("Cleaned up processed reconfiguration files");
        Ok(())
    }

    /// Get the reconfigure directory path.
    ///
    /// # Returns
    /// Reference to the reconfigure directory path
    pub fn reconfigure_dir(&self) -> &Path {
        &self.reconfigure_dir
    }
}
