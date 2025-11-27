//! Tests for the server database management module

use crate::database::ServerDatabase;
use shared::config::TaskType;
use shared::metrics::{AggregatedMetricData, AggregatedMetrics, AggregatedPingMetric};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

/// Helper function to get the current Unix timestamp.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tokio::test]
async fn test_server_database_creation() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();

    let result = db.initialize().await;
    assert!(result.is_ok());
    assert!(temp_dir.path().join("server_metrics.db").exists());
}

#[tokio::test]
async fn test_agent_registration() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    let result = db
        .upsert_agent("test-agent-01", "checksum123", Some("0.7.6"))
        .await;
    assert!(result.is_ok());

    let agent_info = db.get_agent_info("test-agent-01").await.unwrap();
    assert!(agent_info.is_some());
    assert_eq!(agent_info.unwrap().agent_id, "test-agent-01");
}

#[tokio::test]
async fn test_metrics_storage() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    // An agent must be registered before its metrics can be stored, due to the foreign key constraint.
    db.upsert_agent("test-agent-01", "checksum123", Some("0.7.6"))
        .await
        .unwrap();

    let metric = AggregatedMetrics {
        task_name: "Test Ping".to_string(),
        task_type: TaskType::Ping,
        period_start: 1640995200,
        period_end: 1640995260,
        sample_count: 60,
        data: AggregatedMetricData::Ping(AggregatedPingMetric {
            avg_latency_ms: 15.0,
            max_latency_ms: 25.0,
            min_latency_ms: 10.0,
            packet_loss_percent: 0.0,
            successful_pings: 60,
            failed_pings: 0,
            domain: None,
            target_id: None,
        }),
    };

    let result = db.store_metrics("test-agent-01", &[metric]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_config_error_logging() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    db.upsert_agent("test-agent-01", "checksum123", Some("0.7.6"))
        .await
        .unwrap();

    let result = db
        .log_config_error(
            "test-agent-01",
            "2023-01-01T00:00:00Z",
            "Failed to parse tasks.toml",
        )
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_database_stats() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    let stats = db.get_stats().await.unwrap();
    assert_eq!(stats.agent_count, 0);
    assert_eq!(stats.metrics_count, 0);
    assert_eq!(stats.config_errors_count, 0);
    assert!(stats.database_size_bytes > 0);
}

#[tokio::test]
async fn test_get_all_agents() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    db.upsert_agent("agent-01", "checksum1", Some("0.7.6"))
        .await
        .unwrap();
    db.upsert_agent("agent-02", "checksum2", Some("0.7.5"))
        .await
        .unwrap();

    let agents = db.get_all_agents().await.unwrap();
    assert_eq!(agents.len(), 2);
}

#[tokio::test]
async fn test_cleanup_old_data_deletes_expired_metrics() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    // Register agent first
    db.upsert_agent("test-agent", "hash", Some("0.7.6"))
        .await
        .unwrap();

    // Insert old ping metrics (100 days ago)
    let old_timestamp = current_timestamp() - (100 * 24 * 60 * 60);
    {
        let conn = db.get_connection().unwrap();
        conn.execute(
            "INSERT INTO agg_metric_ping (agent_id, task_name, period_start, period_end, min_latency_ms, max_latency_ms, avg_latency_ms, packet_loss_percent, successful_pings, failed_pings, sample_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params!["test-agent", "old-ping", old_timestamp as i64, old_timestamp as i64, 10.0, 20.0, 15.0, 0.0, 10, 0, 10]
        ).unwrap();

        // Insert recent ping metrics (1 day ago)
        let recent_timestamp = current_timestamp() - (24 * 60 * 60);
        conn.execute(
            "INSERT INTO agg_metric_ping (agent_id, task_name, period_start, period_end, min_latency_ms, max_latency_ms, avg_latency_ms, packet_loss_percent, successful_pings, failed_pings, sample_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params!["test-agent", "recent-ping", recent_timestamp as i64, recent_timestamp as i64, 10.0, 20.0, 15.0, 0.0, 10, 0, 10]
        ).unwrap();
    }

    // Run cleanup with 30 day retention
    db.cleanup_old_data(30).await.unwrap();

    // Verify old metrics deleted and recent metrics remain
    {
        let conn = db.get_connection().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agg_metric_ping", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "Should only have recent metric remaining");
    }
}

#[tokio::test]
async fn test_cleanup_old_data_deletes_inactive_agents() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    // Insert old agent directly
    let old_timestamp = current_timestamp() - (100 * 24 * 60 * 60);
    {
        let conn = db.get_connection().unwrap();
        conn.execute(
            "INSERT INTO agents (agent_id, first_seen, last_seen) VALUES (?1, ?2, ?3)",
            rusqlite::params!["old-agent", old_timestamp as i64, old_timestamp as i64],
        )
        .unwrap();
    }

    // Insert recent agent
    db.upsert_agent("recent-agent", "hash", Some("0.7.6"))
        .await
        .unwrap();

    // Run cleanup with 30 day retention
    db.cleanup_old_data(30).await.unwrap();

    // Verify old agent deleted and recent agent remains
    let agents = db.get_all_agents().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_id, "recent-agent");
}

#[tokio::test]
async fn test_cleanup_old_data_preserves_recent_data() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    // Register agent and insert metrics within retention period
    db.upsert_agent("test-agent", "hash", Some("0.7.6"))
        .await
        .unwrap();

    let metric = AggregatedMetrics {
        task_name: "Recent Ping".to_string(),
        task_type: TaskType::Ping,
        period_start: current_timestamp() - (5 * 24 * 60 * 60), // 5 days ago
        period_end: current_timestamp() - (5 * 24 * 60 * 60),
        sample_count: 10,
        data: AggregatedMetricData::Ping(AggregatedPingMetric {
            avg_latency_ms: 15.0,
            max_latency_ms: 25.0,
            min_latency_ms: 10.0,
            packet_loss_percent: 0.0,
            successful_pings: 10,
            failed_pings: 0,
            domain: None,
            target_id: None,
        }),
    };

    db.store_metrics("test-agent", &[metric]).await.unwrap();

    // Run cleanup with 30 day retention
    db.cleanup_old_data(30).await.unwrap();

    // Verify data still exists
    let stats = db.get_stats().await.unwrap();
    assert_eq!(stats.agent_count, 1);
    assert_eq!(stats.metrics_count, 1);
}

#[tokio::test]
async fn test_cleanup_old_data_deletes_old_config_errors() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    db.upsert_agent("test-agent", "hash", Some("0.7.6"))
        .await
        .unwrap();

    // Insert old config error directly
    let old_timestamp = current_timestamp() - (100 * 24 * 60 * 60);
    {
        let conn = db.get_connection().unwrap();
        conn.execute(
            "INSERT INTO config_errors (agent_id, timestamp_utc, error_message, received_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["test-agent", "2023-01-01T00:00:00Z", "Old error", old_timestamp as i64],
        )
        .unwrap();
    }

    // Insert recent config error
    db.log_config_error("test-agent", "2024-01-01T00:00:00Z", "Recent error")
        .await
        .unwrap();

    // Run cleanup with 30 day retention
    db.cleanup_old_data(30).await.unwrap();

    // Verify only recent error remains
    let stats = db.get_stats().await.unwrap();
    assert_eq!(stats.config_errors_count, 1);
}

#[tokio::test]
async fn test_get_agent_info_existing_agent() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    db.upsert_agent("test-agent", "checksum123", Some("0.7.6"))
        .await
        .unwrap();

    let agent_info = db.get_agent_info("test-agent").await.unwrap();
    assert!(agent_info.is_some());

    let info = agent_info.unwrap();
    assert_eq!(info.agent_id, "test-agent");
    assert_eq!(info.last_config_checksum, Some("checksum123".to_string()));
}

#[tokio::test]
async fn test_get_agent_info_nonexistent_agent() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    let agent_info = db.get_agent_info("nonexistent-agent").await.unwrap();
    assert!(agent_info.is_none());
}

#[tokio::test]
async fn test_database_close() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = ServerDatabase::new(temp_dir.path()).unwrap();
    db.initialize().await.unwrap();

    db.close().await;
    // Connection should be None after close
    assert!(db.connection.is_none());
}
