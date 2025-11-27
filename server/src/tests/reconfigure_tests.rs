//! Tests for the multi-agent reconfiguration module

use crate::reconfigure::ReconfigureManager;
use tempfile::TempDir;
use tokio::fs;

/// File names in the reconfigure folder
const AGENT_LIST_FILE: &str = "agent_list.txt";
const TASKS_CONFIG_FILE: &str = "tasks.toml";
const ERROR_FILE: &str = "error.txt";

/// Maximum number of backup files to keep per agent
const MAX_BACKUP_FILES: usize = 10;

async fn create_test_setup() -> (TempDir, TempDir, ReconfigureManager) {
    let reconfigure_dir = TempDir::new().unwrap();
    let agent_configs_dir = TempDir::new().unwrap();

    // Create some test agent config files
    fs::write(
        agent_configs_dir.path().join("agent1.toml"),
        "[[tasks]]\ntype = \"ping\"\nname = \"test\"\nschedule_seconds = 10\nhost = \"8.8.8.8\"\ntimeout_seconds = 5"
    ).await.unwrap();

    fs::write(
        agent_configs_dir.path().join("agent2.toml"),
        "[[tasks]]\ntype = \"ping\"\nname = \"test\"\nschedule_seconds = 10\nhost = \"8.8.8.8\"\ntimeout_seconds = 5"
    ).await.unwrap();

    let manager = ReconfigureManager::new(
        reconfigure_dir.path().to_path_buf(),
        agent_configs_dir.path().to_path_buf(),
    )
    .unwrap();

    (reconfigure_dir, agent_configs_dir, manager)
}

#[tokio::test]
async fn test_empty_folder_check() {
    let (_reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;
    assert!(manager.is_reconfigure_folder_empty().await.unwrap());
}

#[tokio::test]
async fn test_agent_list_validation() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Create valid agent list
    fs::write(
        reconfigure_dir.path().join(AGENT_LIST_FILE),
        "agent1\nagent2\n",
    )
    .await
    .unwrap();

    let agent_ids = manager.read_and_validate_agent_list().await.unwrap();
    assert_eq!(agent_ids, vec!["agent1", "agent2"]);
}

#[tokio::test]
async fn test_all_agents_marker() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Create ALL AGENTS marker
    fs::write(reconfigure_dir.path().join(AGENT_LIST_FILE), "ALL AGENTS\n")
        .await
        .unwrap();

    let agent_ids = manager.read_and_validate_agent_list().await.unwrap();
    assert_eq!(agent_ids.len(), 2); // Should find agent1 and agent2
    assert!(agent_ids.contains(&"agent1".to_string()));
    assert!(agent_ids.contains(&"agent2".to_string()));
}

#[tokio::test]
async fn test_tasks_config_validation() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Create valid tasks config
    let valid_tasks = r#"
[[tasks]]
type = "ping"
name = "test-ping"
schedule_seconds = 10
host = "8.8.8.8"
timeout_seconds = 5
"#;

    fs::write(reconfigure_dir.path().join(TASKS_CONFIG_FILE), valid_tasks)
        .await
        .unwrap();

    let result = manager.read_and_validate_tasks_config().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_backup_creation() {
    let (_reconfigure_dir, agent_configs_dir, manager) = create_test_setup().await;

    let agent_config_path = agent_configs_dir.path().join("agent1.toml");

    // Create backup
    manager
        .backup_agent_config(&agent_config_path)
        .await
        .unwrap();

    // Check backup file exists
    let mut entries = fs::read_dir(agent_configs_dir.path()).await.unwrap();
    let mut backup_found = false;

    while let Some(entry) = entries.next_entry().await.unwrap() {
        let file_name = entry.file_name();
        if file_name
            .to_string_lossy()
            .starts_with("agent1.toml.backup.")
        {
            backup_found = true;
            break;
        }
    }

    assert!(backup_found, "Backup file not created");
}

#[tokio::test]
async fn test_backup_retention() {
    let (_reconfigure_dir, agent_configs_dir, manager) = create_test_setup().await;
    let agent_config_path = agent_configs_dir.path().join("agent1.toml");

    // Create more than MAX_BACKUP_FILES backups
    for _ in 0..15 {
        manager
            .backup_agent_config(&agent_config_path)
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Count backup files
    let mut entries = fs::read_dir(agent_configs_dir.path()).await.unwrap();
    let mut backup_count = 0;

    while let Some(entry) = entries.next_entry().await.unwrap() {
        let file_name = entry.file_name();
        if file_name
            .to_string_lossy()
            .starts_with("agent1.toml.backup.")
        {
            backup_count += 1;
        }
    }

    assert_eq!(
        backup_count, MAX_BACKUP_FILES,
        "Should keep exactly {} backups",
        MAX_BACKUP_FILES
    );
}

#[tokio::test]
async fn test_error_file_writing() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Write error
    manager.write_error("Test error message").await.unwrap();

    // Check error file exists and contains message
    let error_path = reconfigure_dir.path().join(ERROR_FILE);
    assert!(error_path.exists());

    let error_content = fs::read_to_string(&error_path).await.unwrap();
    assert!(error_content.contains("Test error message"));
}

#[tokio::test]
async fn test_error_file_append() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Write multiple errors
    manager.write_error("First error").await.unwrap();
    manager.write_error("Second error").await.unwrap();

    // Check both errors are in file
    let error_path = reconfigure_dir.path().join(ERROR_FILE);
    let error_content = fs::read_to_string(&error_path).await.unwrap();

    assert!(error_content.contains("First error"));
    assert!(error_content.contains("Second error"));

    // Should have 2 timestamped lines
    assert_eq!(error_content.lines().count(), 2);
}

#[tokio::test]
async fn test_invalid_tasks_config() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Create agent list
    fs::write(reconfigure_dir.path().join(AGENT_LIST_FILE), "agent1\n")
        .await
        .unwrap();

    // Create invalid tasks config (missing required fields)
    let invalid_tasks = r#"
[[tasks]]
type = "ping"
# Missing required fields
"#;

    fs::write(
        reconfigure_dir.path().join(TASKS_CONFIG_FILE),
        invalid_tasks,
    )
    .await
    .unwrap();

    let result = manager.read_and_validate_tasks_config().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_duplicate_agent_ids() {
    let (reconfigure_dir, _agent_configs_dir, manager) = create_test_setup().await;

    // Create agent list with duplicates
    fs::write(
        reconfigure_dir.path().join(AGENT_LIST_FILE),
        "agent1\nagent2\nagent1\n",
    )
    .await
    .unwrap();

    let result = manager.read_and_validate_agent_list().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Duplicate"));
}
