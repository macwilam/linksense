//! Tests for the REST API module

use crate::api::{create_router, AgentRateLimiter, ApiError, AppState};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use shared::api::{
    endpoints, headers, BandwidthTestAction, BandwidthTestRequest, BandwidthTestResponse,
    ConfigErrorRequest, ConfigStatus, ConfigUploadRequest, ConfigUploadResponse,
    ConfigVerifyRequest, ConfigVerifyResponse, MetricsRequest, MetricsResponse,
};
use shared::config::ServerConfig;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tower::ServiceExt; // for `oneshot`

/// Helper function to create a test instance of the app's router.
/// Returns (Router, TempDir) - the TempDir must be kept alive for the test duration
async fn create_test_app() -> (axum::Router, TempDir) {
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    // Create temporary directories and files for testing
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let config_dir = temp_dir.path().join("configs");
    std::fs::create_dir_all(&config_dir).unwrap();

    // Create a test agent config file
    let test_agent_config = config_dir.join("test.toml");
    writeln!(
        std::fs::File::create(&test_agent_config).unwrap(),
        r#"
[[tasks]]
type = "ping"
target = "8.8.8.8"
schedule_seconds = 60
"#
    )
    .unwrap();

    // Create a temporary server config file
    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"
listen_address = "127.0.0.1:8787"
api_key = "test-api-key"
data_retention_days = 30
agent_configs_dir = "{}"
bandwidth_test_size_mb = 10
reconfigure_check_interval_seconds = 10
"#,
        config_dir.display()
    )
    .unwrap();

    // Create a test server configuration
    let test_config = ServerConfig {
        listen_address: "127.0.0.1:8787".to_string(),
        api_key: "test-api-key".to_string(),
        data_retention_days: 30,
        agent_configs_dir: config_dir.to_string_lossy().to_string(),
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
    };

    // Initialize database for testing
    let mut database = crate::database::ServerDatabase::new(&data_dir).unwrap();
    database.initialize().await.unwrap();

    // Create config manager for testing
    let config_manager =
        crate::config::ConfigManager::new(config_file.path().to_path_buf()).unwrap();
    let config_manager = Arc::new(tokio::sync::Mutex::new(config_manager));

    // Create bandwidth manager for testing
    let bandwidth_manager = crate::bandwidth_state::BandwidthTestManager::new(120, 300, 30, 60, 30);

    let database_arc = Arc::new(tokio::sync::Mutex::new(database));
    let state = AppState::new(test_config, database_arc, config_manager, bandwidth_manager);
    let router = create_router(state);
    (router, temp_dir)
}

#[tokio::test]
async fn test_health_check() {
    let (app, _temp_dir) = create_test_app().await;

    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    // `oneshot` is used to send a single request to the app and get a response.
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let (app, _temp_dir) = create_test_app().await;

    let test_request = MetricsRequest {
        agent_id: "test-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "checksum123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_configs_endpoint() {
    let (app, _temp_dir) = create_test_app().await;

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{}?agent_id=test", endpoints::CONFIGS))
        .header(headers::API_KEY, "test-api-key")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();

    // If test fails, print response body for debugging
    if status != StatusCode::OK {
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body_bytes);
        eprintln!("Response status: {}, body: {}", status, body_str);
    }

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_configs_endpoint_requires_api_key() {
    let (app, _temp_dir) = create_test_app().await;

    // Request without API key should be rejected
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{}?agent_id=test", endpoints::CONFIGS))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_bandwidth_test_endpoint() {
    let (app, _temp_dir) = create_test_app().await;

    let test_request = BandwidthTestRequest {
        agent_id: "test-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::BANDWIDTH_TEST)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_bandwidth_download_endpoint() {
    let (app, _temp_dir) = create_test_app().await;

    // Test that bandwidth download without active test returns error
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "{}?agent_id=test&size_mb=1",
            endpoints::BANDWIDTH_DOWNLOAD
        ))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    // Should return BAD_REQUEST (400) because no active bandwidth test
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_bandwidth_queue_five_concurrent_agents() {
    let (app, _temp_dir) = create_test_app().await;

    // Simulate 5 agents requesting bandwidth test simultaneously
    let agents = vec!["agent1", "agent2", "agent3", "agent4", "agent5"];
    let mut responses = Vec::new();

    // Send all 5 requests
    for agent_id in &agents {
        let test_request = BandwidthTestRequest {
            agent_id: agent_id.to_string(),
            timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        };

        let request = Request::builder()
            .method(Method::POST)
            .uri(endpoints::BANDWIDTH_TEST)
            .header(headers::API_KEY, "test-api-key")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&test_request).unwrap()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let bandwidth_response: BandwidthTestResponse =
            serde_json::from_slice(&body_bytes).unwrap();
        responses.push(bandwidth_response);
    }

    // Verify first agent can proceed
    assert_eq!(responses[0].action, BandwidthTestAction::Proceed);
    assert!(responses[0].data_size_bytes.is_some());

    // Verify other 4 agents are told to delay
    for (i, response) in responses.iter().enumerate().take(5).skip(1) {
        assert_eq!(
            response.action,
            BandwidthTestAction::Delay,
            "Agent {} should be told to delay",
            i + 1
        );
        assert!(
            responses[i].delay_seconds.is_some(),
            "Agent {} should have delay_seconds",
            i + 1
        );
        assert!(
            responses[i].data_size_bytes.is_none(),
            "Agent {} should not have data_size_bytes when delayed",
            i + 1
        );
    }

    // Verify delays increase for agents further in queue
    assert!(
        responses[1].delay_seconds.unwrap() < responses[2].delay_seconds.unwrap(),
        "Agent 2 should have shorter delay than agent 3"
    );
    assert!(
        responses[2].delay_seconds.unwrap() < responses[3].delay_seconds.unwrap(),
        "Agent 3 should have shorter delay than agent 4"
    );
}

#[tokio::test]
async fn test_metrics_endpoint_invalid_agent_id_empty() {
    let (app, _temp_dir) = create_test_app().await;

    // Test empty agent ID
    let test_request = MetricsRequest {
        agent_id: "".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "checksum123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_metrics_endpoint_invalid_agent_id_special_chars() {
    let (app, _temp_dir) = create_test_app().await;

    // Test agent ID with invalid characters
    let test_request = MetricsRequest {
        agent_id: "agent@invalid!".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "checksum123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_metrics_endpoint_invalid_agent_id_starts_with_hyphen() {
    let (app, _temp_dir) = create_test_app().await;

    // Test agent ID starting with hyphen
    let test_request = MetricsRequest {
        agent_id: "-invalid-start".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "checksum123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_config_verify_invalid_agent_id() {
    let (app, _temp_dir) = create_test_app().await;

    let test_request = ConfigVerifyRequest {
        agent_id: "_invalid_end_".to_string(),
        tasks_config_hash: "hash123".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::CONFIG_VERIFY)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_bandwidth_test_invalid_agent_id_too_long() {
    let (app, _temp_dir) = create_test_app().await;

    // Test agent ID that's too long (>128 characters)
    let long_id = "a".repeat(129);
    let test_request = BandwidthTestRequest {
        agent_id: long_id,
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::BANDWIDTH_TEST)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_configs_endpoint_invalid_agent_id() {
    let (app, _temp_dir) = create_test_app().await;

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{}?agent_id=", endpoints::CONFIGS))
        .header(headers::API_KEY, "test-api-key")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_bandwidth_download_invalid_agent_id() {
    let (app, _temp_dir) = create_test_app().await;

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "{}?agent_id=invalid@chars",
            endpoints::BANDWIDTH_DOWNLOAD
        ))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Helper function to create a test app with whitelist configured
async fn create_test_app_with_whitelist(whitelist: Vec<String>) -> (axum::Router, TempDir) {
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let config_dir = temp_dir.path().join("configs");
    std::fs::create_dir_all(&config_dir).unwrap();

    let test_agent_config = config_dir.join("test.toml");
    writeln!(
        std::fs::File::create(&test_agent_config).unwrap(),
        r#"
[[tasks]]
type = "ping"
target = "8.8.8.8"
schedule_seconds = 60
"#
    )
    .unwrap();

    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"
listen_address = "127.0.0.1:8787"
api_key = "test-api-key"
data_retention_days = 30
agent_configs_dir = "{}"
bandwidth_test_size_mb = 10
reconfigure_check_interval_seconds = 10
"#,
        config_dir.display()
    )
    .unwrap();

    let test_config = ServerConfig {
        listen_address: "127.0.0.1:8787".to_string(),
        api_key: "test-api-key".to_string(),
        data_retention_days: 30,
        agent_configs_dir: config_dir.to_string_lossy().to_string(),
        bandwidth_test_size_mb: 10,
        reconfigure_check_interval_seconds: 10,
        agent_id_whitelist: whitelist,
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
    };

    let mut database = crate::database::ServerDatabase::new(&data_dir).unwrap();
    database.initialize().await.unwrap();

    let config_manager =
        crate::config::ConfigManager::new(config_file.path().to_path_buf()).unwrap();
    let config_manager = Arc::new(tokio::sync::Mutex::new(config_manager));

    let bandwidth_manager = crate::bandwidth_state::BandwidthTestManager::new(120, 300, 30, 60, 30);

    let database_arc = Arc::new(tokio::sync::Mutex::new(database));
    let state = AppState::new(test_config, database_arc, config_manager, bandwidth_manager);
    let router = create_router(state);
    (router, temp_dir)
}

#[tokio::test]
async fn test_whitelist_empty_allows_all_agents() {
    // Empty whitelist = all agents allowed
    let (app, _temp_dir) = create_test_app_with_whitelist(vec![]).await;

    let test_request = MetricsRequest {
        agent_id: "any-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "abc123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_whitelist_allows_listed_agent() {
    // Whitelist contains "agent-1"
    let (app, _temp_dir) =
        create_test_app_with_whitelist(vec!["agent-1".to_string(), "agent-2".to_string()]).await;

    let test_request = MetricsRequest {
        agent_id: "agent-1".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "abc123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_whitelist_blocks_unlisted_agent() {
    // Whitelist contains "agent-1" and "agent-2", but not "agent-3"
    let (app, _temp_dir) =
        create_test_app_with_whitelist(vec!["agent-1".to_string(), "agent-2".to_string()]).await;

    let test_request = MetricsRequest {
        agent_id: "agent-3".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "abc123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_whitelist_applies_to_config_verify() {
    let (app, _temp_dir) = create_test_app_with_whitelist(vec!["allowed-agent".to_string()]).await;

    let test_request = ConfigVerifyRequest {
        agent_id: "blocked-agent".to_string(),
        tasks_config_hash: "abc123".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::CONFIG_VERIFY)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_whitelist_applies_to_config_upload() {
    let (app, _temp_dir) = create_test_app_with_whitelist(vec!["allowed-agent".to_string()]).await;

    let test_request = ConfigUploadRequest {
        agent_id: "blocked-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        tasks_toml: "test".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::CONFIG_UPLOAD)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_whitelist_applies_to_bandwidth_test() {
    let (app, _temp_dir) = create_test_app_with_whitelist(vec!["allowed-agent".to_string()]).await;

    let test_request = BandwidthTestRequest {
        agent_id: "blocked-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::BANDWIDTH_TEST)
        .header(headers::API_KEY, "test-api-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_whitelist_applies_to_configs_endpoint() {
    let (app, _temp_dir) = create_test_app_with_whitelist(vec!["allowed-agent".to_string()]).await;

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{}?agent_id=blocked-agent", endpoints::CONFIGS))
        .header(headers::API_KEY, "test-api-key")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_whitelist_applies_to_bandwidth_download() {
    let (app, _temp_dir) = create_test_app_with_whitelist(vec!["allowed-agent".to_string()]).await;

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "{}?agent_id=blocked-agent",
            endpoints::BANDWIDTH_DOWNLOAD
        ))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_whitelist_applies_to_config_error() {
    let (app, _temp_dir) = create_test_app_with_whitelist(vec!["allowed-agent".to_string()]).await;

    let test_request = ConfigErrorRequest {
        agent_id: "blocked-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        error_message: "test error".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::CONFIG_ERROR)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// Rate Limiter Tests
#[tokio::test]
async fn test_rate_limiter_allows_under_limit() {
    let limiter = AgentRateLimiter::new(Duration::from_secs(60), 3);

    // Allow first 3 requests
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_ok());

    // Block 4th request
    let result = limiter.check_rate_limit("agent1").await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ApiError::TooManyRequests));
}

#[tokio::test]
async fn test_rate_limiter_window_expiration() {
    let limiter = AgentRateLimiter::new(Duration::from_millis(100), 2);

    // Use up limit
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_err());

    // Wait for window to expire
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Should be allowed again
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
}

#[tokio::test]
async fn test_rate_limiter_multiple_agents() {
    let limiter = AgentRateLimiter::new(Duration::from_secs(60), 2);

    // Agent1 uses their limit
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_err());

    // Agent2 should have their own independent limit
    assert!(limiter.check_rate_limit("agent2").await.is_ok());
    assert!(limiter.check_rate_limit("agent2").await.is_ok());
    assert!(limiter.check_rate_limit("agent2").await.is_err());
}

#[tokio::test]
async fn test_rate_limiter_concurrent_requests() {
    let limiter = AgentRateLimiter::new(Duration::from_secs(60), 10);

    // Spawn multiple concurrent requests
    let mut handles = vec![];
    for _ in 0..5 {
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move { limiter_clone.check_rate_limit("agent1").await });
        handles.push(handle);
    }

    // All should succeed (5 requests < 10 limit)
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_rate_limiter_reset_after_window() {
    let limiter = AgentRateLimiter::new(Duration::from_millis(50), 1);

    // Use the limit
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_err());

    // Wait for full window to pass
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Should reset completely
    assert!(limiter.check_rate_limit("agent1").await.is_ok());
    assert!(limiter.check_rate_limit("agent1").await.is_err());
}

#[tokio::test]
async fn test_metrics_endpoint_requests_config_upload_when_server_has_no_config() {
    let (app, _temp_dir) = create_test_app().await;

    // Send metrics for an agent that doesn't have a config file on the server
    let test_request = MetricsRequest {
        agent_id: "new-agent-without-config".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        config_checksum: "some-checksum-123".to_string(),
        metrics: vec![],
        agent_version: Some("0.7.6".to_string()),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::METRICS)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics_response: MetricsResponse = serde_json::from_slice(&body_bytes).unwrap();

    // Server should respond with Stale status, requesting the agent to upload its config
    assert_eq!(metrics_response.config_status, ConfigStatus::Stale);
}

#[tokio::test]
async fn test_config_verify_requests_upload_when_server_has_no_config() {
    let (app, _temp_dir) = create_test_app().await;

    // Agent verifies config for a non-existent config file
    let test_request = ConfigVerifyRequest {
        agent_id: "new-agent-without-config".to_string(),
        tasks_config_hash: "some-hash-456".to_string(),
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::CONFIG_VERIFY)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let verify_response: ConfigVerifyResponse = serde_json::from_slice(&body_bytes).unwrap();

    // Server should respond with Stale status and no config (requesting upload)
    assert_eq!(verify_response.config_status, ConfigStatus::Stale);
    assert!(verify_response.tasks_toml.is_none());
}

#[tokio::test]
async fn test_config_upload_saves_when_server_has_no_config() {
    use base64::Engine;
    use std::io::Write;
    let (app, temp_dir) = create_test_app().await;

    // Create valid tasks config
    let tasks_toml = r#"
[[tasks]]
name = "test-ping"
type = "ping"
host = "8.8.8.8"
schedule_seconds = 60
"#;

    // Compress and encode
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(tasks_toml.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&compressed);

    let test_request = ConfigUploadRequest {
        agent_id: "new-agent".to_string(),
        timestamp_utc: "2023-01-01T00:00:00Z".to_string(),
        tasks_toml: encoded,
    };

    let request = Request::builder()
        .method(Method::POST)
        .uri(endpoints::CONFIG_UPLOAD)
        .header("content-type", "application/json")
        .header(headers::API_KEY, "test-api-key")
        .body(Body::from(serde_json::to_string(&test_request).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload_response: ConfigUploadResponse = serde_json::from_slice(&body_bytes).unwrap();

    // Server should accept the config
    assert_eq!(upload_response.status, "success");
    assert!(upload_response.accepted);

    // Verify config was saved to disk
    let config_path = temp_dir.path().join("configs/new-agent.toml");
    assert!(config_path.exists());
    let saved_content = std::fs::read_to_string(config_path).unwrap();
    assert_eq!(saved_content.trim(), tasks_toml.trim());
}
