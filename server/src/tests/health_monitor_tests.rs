//! Tests for the agent health monitoring system

use crate::config::ConfigManager;
use crate::database::ServerDatabase;
use crate::health_monitor::HealthMonitor;
use shared::config::{PingParams, TaskConfig, TaskParams, TaskType, TasksConfig};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use tokio::sync::Mutex;

/// Helper function to get current Unix timestamp
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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

    let monitor = HealthMonitor::new(db, config_manager, output_dir.clone(), "0.7.6".to_string());
    assert!(monitor.is_ok());
    assert!(output_dir.exists());
}

#[tokio::test]
async fn test_check_all_agents_no_agents() {
    let (db, config_manager, temp_dir) = setup_test_environment().await;
    let output_dir = temp_dir.path().join("output");

    let monitor = HealthMonitor::new(db, config_manager, output_dir, "0.7.6".to_string()).unwrap();
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

    // Load the agent config into cache
    {
        let cm = config_manager.lock().await;
        cm.reload_agent_config("test-agent").await.unwrap();
    }

    let monitor = HealthMonitor::new(db, config_manager, output_dir, "0.7.6".to_string()).unwrap();

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
