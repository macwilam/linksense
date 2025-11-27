//! Tests for configuration management implementation

use crate::config::ConfigManager;
use shared::config::{AgentConfig, PingParams, TaskConfig, TaskParams, TaskType, TasksConfig};
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;

/// Helper function to create dummy configuration files for testing.
/// This encapsulates the setup logic for tests, making them cleaner.
async fn create_test_config_files(dir: &std::path::Path) -> anyhow::Result<()> {
    let agent_config = AgentConfig {
        agent_id: "test-agent-01".to_string(),
        central_server_url: "https://monitoring.example.com".to_string(),
        api_key: "test-api-key".to_string(),
        local_data_retention_days: 7,
        auto_update_tasks: false,
        local_only: false,
        metrics_flush_interval_seconds: 5,
        metrics_send_interval_seconds: 30,
        metrics_batch_size: 50,
        metrics_max_retries: 10,
        queue_cleanup_interval_seconds: 3600,
        data_cleanup_interval_seconds: 86400,
        max_concurrent_tasks: 50,
        http_response_max_size_mb: 100,
        http_client_timeout_seconds: 30,
        database_busy_timeout_seconds: 5,
        graceful_shutdown_timeout_seconds: 30,
        channel_buffer_size: 1000,
        http_client_refresh_interval_seconds: 3600,
    };

    let tasks_config = TasksConfig {
        tasks: vec![TaskConfig {
            task_type: TaskType::Ping,
            schedule_seconds: 10,
            name: "Test Ping".to_string(),
            timeout: None,
            params: TaskParams::Ping(PingParams {
                host: "8.8.8.8".to_string(),
                timeout_seconds: 1,
                target_id: None,
            }),
        }],
    };

    let agent_toml = toml::to_string(&agent_config)?;
    let tasks_toml = toml::to_string(&tasks_config)?;

    fs::write(dir.join("agent.toml"), agent_toml).await?;
    fs::write(dir.join("tasks.toml"), tasks_toml).await?;

    Ok(())
}

#[tokio::test]
async fn test_config_manager_creation() {
    // `TempDir` provides a temporary directory that is automatically cleaned up.
    let temp_dir = TempDir::new().unwrap();
    let config_manager = ConfigManager::new(temp_dir.path().to_path_buf());
    assert!(config_manager.is_ok());
}

#[tokio::test]
async fn test_config_manager_nonexistent_dir() {
    // Test that `new` fails correctly if the directory does not exist.
    let config_manager = ConfigManager::new(PathBuf::from("/nonexistent/path"));
    assert!(config_manager.is_err());
}

#[tokio::test]
async fn test_config_loading() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    let result = config_manager.load_config().await;
    assert!(result.is_ok());

    // Assert that the configuration is loaded correctly and accessible.
    assert!(config_manager.is_loaded());
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().agent_id,
        "test-agent-01"
    );
    assert_eq!(config_manager.tasks_config.as_ref().unwrap().tasks.len(), 1);
    assert!(!config_manager.current_checksum.as_ref().unwrap().is_empty());
}

#[tokio::test]
async fn test_config_loading_missing_files() {
    let temp_dir = TempDir::new().unwrap();
    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();

    // `load_config` should fail if the config files are not present.
    let result = config_manager.load_config().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_override_and_persist_agent_config() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // Verify initial values
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .central_server_url,
        "https://monitoring.example.com"
    );
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().api_key,
        "test-api-key"
    );

    // Override with new values
    let changed = config_manager
        .override_and_persist_agent_config(
            None,
            Some("https://new-server.example.com".to_string()),
            Some("new-api-key".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    assert!(changed);

    // Verify in-memory config was updated
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .central_server_url,
        "https://new-server.example.com"
    );
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().api_key,
        "new-api-key"
    );

    // Verify the file was updated by creating a new ConfigManager
    let mut config_manager2 = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager2.load_config().await.unwrap();

    assert_eq!(
        config_manager2
            .agent_config
            .as_ref()
            .unwrap()
            .central_server_url,
        "https://new-server.example.com"
    );
    assert_eq!(
        config_manager2.agent_config.as_ref().unwrap().api_key,
        "new-api-key"
    );
}

#[tokio::test]
async fn test_override_partial_config() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // Override only server URL
    let changed = config_manager
        .override_and_persist_agent_config(
            None,
            Some("https://another-server.example.com".to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    assert!(changed);

    // Server URL should be updated, API key should remain unchanged
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .central_server_url,
        "https://another-server.example.com"
    );
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().api_key,
        "test-api-key"
    );
}

#[tokio::test]
async fn test_override_with_same_values() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // Override with same values
    let changed = config_manager
        .override_and_persist_agent_config(
            None,
            Some("https://monitoring.example.com".to_string()),
            Some("test-api-key".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Should return false as nothing changed
    assert!(!changed);
}

#[tokio::test]
async fn test_override_all_config_fields() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // Verify initial values
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().agent_id,
        "test-agent-01"
    );
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .central_server_url,
        "https://monitoring.example.com"
    );
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().api_key,
        "test-api-key"
    );
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .local_data_retention_days,
        7
    );
    assert!(
        !config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .auto_update_tasks
    );

    // Override all fields
    let changed = config_manager
        .override_and_persist_agent_config(
            Some("new-agent-id".to_string()),
            Some("https://new-server.example.com".to_string()),
            Some("new-api-key".to_string()),
            Some(30),
            Some(true),
            None,
        )
        .await
        .unwrap();

    assert!(changed);

    // Verify all fields were updated
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().agent_id,
        "new-agent-id"
    );
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .central_server_url,
        "https://new-server.example.com"
    );
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().api_key,
        "new-api-key"
    );
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .local_data_retention_days,
        30
    );
    assert!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .auto_update_tasks
    );

    // Verify persistence by loading again
    let mut config_manager2 = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager2.load_config().await.unwrap();
    assert_eq!(
        config_manager2.agent_config.as_ref().unwrap().agent_id,
        "new-agent-id"
    );
    assert_eq!(
        config_manager2
            .agent_config
            .as_ref()
            .unwrap()
            .local_data_retention_days,
        30
    );
    assert!(
        config_manager2
            .agent_config
            .as_ref()
            .unwrap()
            .auto_update_tasks
    );
}

#[tokio::test]
async fn test_override_retention_days_only() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // Override only retention days
    let changed = config_manager
        .override_and_persist_agent_config(None, None, None, Some(14), None, None)
        .await
        .unwrap();

    assert!(changed);
    assert_eq!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .local_data_retention_days,
        14
    );
    // Other fields should remain unchanged
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().agent_id,
        "test-agent-01"
    );
    assert_eq!(
        config_manager.agent_config.as_ref().unwrap().api_key,
        "test-api-key"
    );
}

#[tokio::test]
async fn test_override_auto_update_tasks_only() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // Initial value is false
    assert!(
        !config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .auto_update_tasks
    );

    // Override to true
    let changed = config_manager
        .override_and_persist_agent_config(None, None, None, None, Some(true), None)
        .await
        .unwrap();

    assert!(changed);
    assert!(
        config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .auto_update_tasks
    );

    // Override back to false
    let changed2 = config_manager
        .override_and_persist_agent_config(None, None, None, None, Some(false), None)
        .await
        .unwrap();

    assert!(changed2);
    assert!(
        !config_manager
            .agent_config
            .as_ref()
            .unwrap()
            .auto_update_tasks
    );
}

#[tokio::test]
async fn test_config_reload_unchanged() {
    let temp_dir = TempDir::new().unwrap();
    create_test_config_files(temp_dir.path()).await.unwrap();

    let mut config_manager = ConfigManager::new(temp_dir.path().to_path_buf()).unwrap();
    config_manager.load_config().await.unwrap();

    // `reload_config` should return `false` if the files haven't changed.
    let changed = config_manager.reload_config().await.unwrap();
    assert!(!changed);
}
