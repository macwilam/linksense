//! Tests for configuration types and validation

use crate::config::{
    AgentConfig, BandwidthParams, HttpGetParams, PingParams, TaskConfig, TaskParams, TaskType,
    TasksConfig, TcpParams, TlsHandshakeParams,
};
use std::collections::HashMap;

#[test]
fn test_agent_config_validation() {
    let mut config = AgentConfig {
        agent_id: "test-agent-01".to_string(),
        central_server_url: "https://monitoring.example.com".to_string(),
        api_key: "secret-key".to_string(),
        local_data_retention_days: 7,
        auto_update_tasks: true,
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

    assert!(config.validate().is_ok());

    // Test empty agent_id
    config.agent_id = "".to_string();
    assert!(config.validate().is_err());

    config.agent_id = "test-agent-01".to_string();

    // Test invalid URL
    config.central_server_url = "not-a-url".to_string();
    assert!(config.validate().is_err());

    config.central_server_url = "https://monitoring.example.com".to_string();

    // Test zero retention days
    config.local_data_retention_days = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_task_config_validation() {
    let ping_task = TaskConfig {
        task_type: TaskType::Ping,
        schedule_seconds: 10,
        name: "Test Ping".to_string(),
        timeout: None,
        params: TaskParams::Ping(PingParams {
            host: "8.8.8.8".to_string(),
            timeout_seconds: 1,
            target_id: None,
        }),
    };

    assert!(ping_task.validate().is_ok());

    let http_task = TaskConfig {
        task_type: TaskType::HttpGet,
        schedule_seconds: 30,
        name: "Test HTTP".to_string(),
        timeout: Some(15),
        params: TaskParams::HttpGet(HttpGetParams {
            url: "https://example.com".to_string(),
            timeout_seconds: 10,
            headers: HashMap::new(),
            verify_ssl: false,
            target_id: None,
        }),
    };

    assert!(http_task.validate().is_ok());
}

#[test]
fn test_get_effective_timeout() {
    let ping_task = TaskConfig {
        task_type: TaskType::Ping,
        schedule_seconds: 10,
        name: "Test Ping".to_string(),
        timeout: Some(3),
        params: TaskParams::Ping(PingParams {
            host: "8.8.8.8".to_string(),
            timeout_seconds: 1,
            target_id: None,
        }),
    };

    // Should use task-level timeout when specified
    assert_eq!(ping_task.get_effective_timeout(), 3);

    let ping_task_no_override = TaskConfig {
        task_type: TaskType::Ping,
        schedule_seconds: 10,
        name: "Test Ping No Override".to_string(),
        timeout: None,
        params: TaskParams::Ping(PingParams {
            host: "8.8.8.8".to_string(),
            timeout_seconds: 1,
            target_id: None,
        }),
    };

    // Should use default timeout when task-level timeout is None
    assert_eq!(ping_task_no_override.get_effective_timeout(), 1);

    let http_task = TaskConfig {
        task_type: TaskType::HttpGet,
        schedule_seconds: 30,
        name: "Test HTTP".to_string(),
        timeout: None,
        params: TaskParams::HttpGet(HttpGetParams {
            url: "https://example.com".to_string(),
            timeout_seconds: 10,
            headers: HashMap::new(),
            verify_ssl: false,
            target_id: None,
        }),
    };

    // Should use HTTP default timeout
    assert_eq!(http_task.get_effective_timeout(), 10);
}

#[test]
fn test_bandwidth_task_minimum_schedule() {
    let valid_bandwidth_task = TaskConfig {
        task_type: TaskType::Bandwidth,
        schedule_seconds: 60,
        name: "Valid Bandwidth Test".to_string(),
        timeout: None,
        params: TaskParams::Bandwidth(BandwidthParams {
            timeout_seconds: 60,
            max_retries: 10,
            target_id: None,
        }),
    };

    // Should pass validation with 60 second schedule
    assert!(valid_bandwidth_task.validate().is_ok());

    let invalid_bandwidth_task = TaskConfig {
        task_type: TaskType::Bandwidth,
        schedule_seconds: 30, // Less than 60 seconds
        name: "Invalid Bandwidth Test".to_string(),
        timeout: None,
        params: TaskParams::Bandwidth(BandwidthParams {
            timeout_seconds: 60,
            max_retries: 10,
            target_id: None,
        }),
    };

    // Should fail validation with less than 60 second schedule
    assert!(invalid_bandwidth_task.validate().is_err());

    let error_msg = invalid_bandwidth_task.validate().unwrap_err().to_string();
    assert!(error_msg.contains("Bandwidth tasks must have schedule_seconds >= 60"));
}

#[test]
fn test_task_params_ordering() {
    // Test that HttpContent (more specific) is correctly deserialized
    // even when it comes before HttpGet in the untagged enum
    let toml_str = r#"
[[tasks]]
type = "http_content"
name = "Content Check"
schedule_seconds = 30
url = "https://example.com"
regexp = "test"
timeout_seconds = 30

[[tasks]]
type = "http_get"
name = "GET Request"
schedule_seconds = 30
url = "https://example.com"
timeout_seconds = 30
"#;

    let config: TasksConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.tasks.len(), 2);

    // Verify first task is HttpContent
    assert_eq!(config.tasks[0].task_type, TaskType::HttpContent);
    match &config.tasks[0].params {
        TaskParams::HttpContent(p) => {
            assert_eq!(p.url, "https://example.com");
            assert_eq!(p.regexp, "test");
        }
        _ => panic!("Expected HttpContent params"),
    }

    // Verify second task is HttpGet
    assert_eq!(config.tasks[1].task_type, TaskType::HttpGet);
    match &config.tasks[1].params {
        TaskParams::HttpGet(p) => {
            assert_eq!(p.url, "https://example.com");
        }
        _ => panic!("Expected HttpGet params"),
    }

    // Validate both tasks
    assert!(config.tasks[0].validate().is_ok());
    assert!(config.tasks[1].validate().is_ok());
}

#[test]
fn test_type_field_is_respected() {
    // With the custom deserializer, the type field determines which params
    // struct to use, regardless of which fields are present

    // Test 1: type=http_get with extra regexp field
    // The regexp field should be ignored since HttpGetParams doesn't have it
    let toml_str = r#"
[[tasks]]
type = "http_get"
name = "HttpGet with extra field"
schedule_seconds = 30
url = "https://example.com"
regexp = "ignored"
"#;

    let config: TasksConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.tasks.len(), 1);
    assert_eq!(config.tasks[0].task_type, TaskType::HttpGet);

    // Should deserialize as HttpGet (extra field ignored)
    match &config.tasks[0].params {
        TaskParams::HttpGet(_) => {} // Expected
        _ => panic!("Expected HttpGet params"),
    }

    // Validation should pass (type and params match)
    assert!(config.tasks[0].validate().is_ok());

    // Test 2: type=http_content without required regexp field
    // Should fail at deserialization
    let toml_str2 = r#"
[[tasks]]
type = "http_content"
name = "HttpContent missing regexp"
schedule_seconds = 30
url = "https://example.com"
"#;

    let result = toml::from_str::<TasksConfig>(toml_str2);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("regexp") || err_msg.contains("missing field"));
}

#[test]
fn test_tcp_task_validation() {
    // Test valid TCP task
    let tcp_task = TaskConfig {
        task_type: TaskType::Tcp,
        schedule_seconds: 60,
        name: "SSH Port Check".to_string(),
        timeout: None,
        params: TaskParams::Tcp(TcpParams {
            host: "example.com:22".to_string(),
            timeout_seconds: 5,
            target_id: Some("ssh-server".to_string()),
        }),
    };

    assert!(tcp_task.validate().is_ok());

    // Test TCP task with missing host
    let invalid_tcp_task = TaskConfig {
        task_type: TaskType::Tcp,
        schedule_seconds: 60,
        name: "Invalid TCP".to_string(),
        timeout: None,
        params: TaskParams::Tcp(TcpParams {
            host: "".to_string(),
            timeout_seconds: 5,
            target_id: None,
        }),
    };

    assert!(invalid_tcp_task.validate().is_err());

    // Test TCP task from TOML
    let toml_str = r#"
[[tasks]]
type = "tcp"
name = "wilam SSH Port Check"
schedule_seconds = 1
host = "wilam.ovh:22"
timeout_seconds = 5
"#;

    let config: TasksConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.tasks.len(), 1);
    assert_eq!(config.tasks[0].task_type, TaskType::Tcp);
    assert!(config.tasks[0].validate().is_ok());

    match &config.tasks[0].params {
        TaskParams::Tcp(params) => {
            assert_eq!(params.host, "wilam.ovh:22");
            assert_eq!(params.timeout_seconds, 5);
        }
        _ => panic!("Expected Tcp params"),
    }
}

#[test]
fn test_tls_handshake_task_validation() {
    // Test valid TLS handshake task
    let tls_task = TaskConfig {
        task_type: TaskType::TlsHandshake,
        schedule_seconds: 60,
        name: "TLS Check".to_string(),
        timeout: None,
        params: TaskParams::TlsHandshake(TlsHandshakeParams {
            host: "example.com:443".to_string(),
            verify_ssl: true,
            target_id: None,
        }),
    };

    assert!(tls_task.validate().is_ok());

    // Test TLS task with missing host
    let invalid_tls_task = TaskConfig {
        task_type: TaskType::TlsHandshake,
        schedule_seconds: 60,
        name: "Invalid TLS".to_string(),
        timeout: None,
        params: TaskParams::TlsHandshake(TlsHandshakeParams {
            host: "".to_string(),
            verify_ssl: true,
            target_id: None,
        }),
    };

    assert!(invalid_tls_task.validate().is_err());
}

#[test]
fn test_toml_serialization() {
    let config = AgentConfig {
        agent_id: "test-agent".to_string(),
        central_server_url: "https://example.com".to_string(),
        api_key: "secret".to_string(),
        local_data_retention_days: 30,
        auto_update_tasks: true,
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

    let toml_str = toml::to_string(&config).unwrap();
    let parsed: AgentConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(config, parsed);
}
