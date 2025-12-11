//! Tests for the server configuration management module

use crate::config::ConfigManager;
use anyhow::Result;
use shared::config::ServerConfig;
use std::io::Write;
use std::path::PathBuf;
use tempfile::{NamedTempFile, TempDir};

/// The expected name of the configuration file.
const SERVER_CONFIG_FILE: &str = "server.toml";

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
        monitor_agents_health: false,
        health_check_interval_seconds: 300,
        health_check_success_ratio_threshold: 0.9,
        health_check_retention_days: 30,
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

#[tokio::test]
async fn test_get_agent_config_existing() {
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
    let result = config_manager.get_agent_config("agent1").await;
    assert!(result.is_ok());
    let cached = result.unwrap();
    assert_eq!(cached.content.trim(), tasks_toml.trim());
    assert!(!cached.hash.is_empty());
    assert!(!cached.compressed.is_empty());
}

#[tokio::test]
async fn test_get_agent_config_missing() {
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
    let result = config_manager.get_agent_config("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_agent_config_hash_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let agent_configs_dir = temp_dir.path().join("agent-configs");
    std::fs::create_dir_all(&agent_configs_dir).unwrap();

    let tasks_toml =
        "[[tasks]]\ntype = \"ping\"\nname = \"test\"\nschedule_seconds = 30\nhost = \"8.8.8.8\"\n";
    std::fs::write(agent_configs_dir.join("agent1.toml"), tasks_toml).unwrap();

    let config = create_test_server_config();
    let mut config = config;
    config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();
    let server_toml = toml::to_string(&config).unwrap();
    let config_path = temp_dir.path().join("server.toml");
    std::fs::write(&config_path, server_toml).unwrap();

    let config_manager = ConfigManager::new(config_path).unwrap();
    let hash1 = config_manager
        .get_agent_config("agent1")
        .await
        .unwrap()
        .hash;
    let hash2 = config_manager
        .get_agent_config("agent1")
        .await
        .unwrap()
        .hash;

    // Same config should produce same hash
    assert_eq!(hash1, hash2);
    assert!(!hash1.is_empty());
}

#[tokio::test]
async fn test_get_agent_config_hash_changes() {
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
        .get_agent_config("agent1")
        .await
        .unwrap()
        .hash;

    // Modify config and invalidate cache
    std::fs::write(&tasks_path, "[[tasks]]\ntype = \"http_get\"\n").unwrap();
    config_manager.invalidate_agent_cache("agent1").await;

    let hash2 = config_manager
        .get_agent_config("agent1")
        .await
        .unwrap()
        .hash;

    // Hash should change
    assert_ne!(hash1, hash2);
}

#[tokio::test]
async fn test_get_agent_config_compressed_valid() {
    let temp_dir = TempDir::new().unwrap();
    let agent_configs_dir = temp_dir.path().join("agent-configs");
    std::fs::create_dir_all(&agent_configs_dir).unwrap();

    let tasks_toml =
        "[[tasks]]\ntype = \"ping\"\nname = \"test\"\nschedule_seconds = 30\nhost = \"8.8.8.8\"\n";
    std::fs::write(agent_configs_dir.join("agent1.toml"), tasks_toml).unwrap();

    let config = create_test_server_config();
    let mut config = config;
    config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();
    let server_toml = toml::to_string(&config).unwrap();
    let config_path = temp_dir.path().join("server.toml");
    std::fs::write(&config_path, server_toml).unwrap();

    let config_manager = ConfigManager::new(config_path).unwrap();
    let cached = config_manager.get_agent_config("agent1").await.unwrap();

    // Decode and decompress to verify
    use base64::Engine;
    let compressed_data = base64::engine::general_purpose::STANDARD
        .decode(&cached.compressed)
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

#[tokio::test]
async fn test_load_all_agent_configs() {
    let temp_dir = TempDir::new().unwrap();
    let agent_configs_dir = temp_dir.path().join("agent-configs");
    std::fs::create_dir_all(&agent_configs_dir).unwrap();

    // Create multiple agent config files
    std::fs::write(
        agent_configs_dir.join("agent1.toml"),
        "[[tasks]]\ntype = \"ping\"\n",
    )
    .unwrap();
    std::fs::write(
        agent_configs_dir.join("agent2.toml"),
        "[[tasks]]\ntype = \"http_get\"\n",
    )
    .unwrap();

    let mut config = create_test_server_config();
    config.agent_configs_dir = agent_configs_dir.to_string_lossy().to_string();
    let server_toml = toml::to_string(&config).unwrap();
    let config_path = temp_dir.path().join("server.toml");
    std::fs::write(&config_path, server_toml).unwrap();

    let config_manager = ConfigManager::new(config_path).unwrap();
    let count = config_manager.load_all_agent_configs().await.unwrap();

    assert_eq!(count, 2);

    // Verify both configs are cached
    let cached1 = config_manager.get_agent_config("agent1").await.unwrap();
    let cached2 = config_manager.get_agent_config("agent2").await.unwrap();

    assert!(cached1.content.contains("ping"));
    assert!(cached2.content.contains("http_get"));
}
