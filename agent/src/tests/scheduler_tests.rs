//! Tests for task scheduler implementation

use crate::database::AgentDatabase;
use crate::scheduler::{SchedulerState, TaskScheduler};
use shared::config::{PingParams, TaskConfig, TaskParams, TaskType, TasksConfig};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

/// Helper function to create a test `TasksConfig`.
fn create_test_config() -> TasksConfig {
    TasksConfig {
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
    }
}

#[tokio::test]
async fn test_scheduler_creation() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();
    let db = Arc::new(RwLock::new(db));

    let config = create_test_config();
    let scheduler = TaskScheduler::new(config, db, 5, 30, 1000, 3600, None, None, None);
    assert!(scheduler.is_ok());
}

#[tokio::test]
async fn test_scheduler_start_stop() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();
    let db = Arc::new(RwLock::new(db));

    let config = create_test_config();
    let mut scheduler =
        TaskScheduler::new(config, db, 5, 30, 1000, 3600, None, None, None).unwrap();

    assert_eq!(scheduler.state, SchedulerState::Stopped);

    scheduler.start().await.unwrap();
    assert_eq!(scheduler.state, SchedulerState::Running);

    scheduler.stop().await.unwrap();
    assert_eq!(scheduler.state, SchedulerState::Stopped);
}
