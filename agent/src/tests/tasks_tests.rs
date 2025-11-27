//! Tests for task executor implementation

use crate::tasks::TaskExecutor;
use shared::config::{
    BandwidthParams, DnsQueryDohParams, DnsQueryParams, DnsRecordType, HttpContentParams,
    HttpGetParams, PingParams, TaskConfig, TaskParams, TaskType, TcpParams, TlsHandshakeParams,
};
use std::collections::HashMap;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_task_executor_creation() {
    let (sender, _receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None);
    assert!(executor.is_ok());
}

#[tokio::test]
async fn test_ping_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
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

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    // Verify that a result was sent through the channel and it has the correct data.
    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test Ping");
    assert!(task_result.success);
    assert!(task_result.metric_data.is_some());
}

#[tokio::test]
async fn test_http_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
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

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test HTTP");
    assert!(task_result.success);
}

#[tokio::test]
async fn test_dns_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::DnsQuery,
        schedule_seconds: 60,
        name: "Test DNS".to_string(),
        timeout: None,
        params: TaskParams::DnsQuery(DnsQueryParams {
            server: "1.1.1.1:53".to_string(),
            domain: "google.com".to_string(),
            record_type: DnsRecordType::A,
            timeout_seconds: 5,
            expected_ip: None,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test DNS");
    if !task_result.success {
        eprintln!("DNS task failed with error: {:?}", task_result.error);
    }
    assert!(
        task_result.success,
        "DNS task failed: {:?}",
        task_result.error
    );
}

#[tokio::test]
async fn test_bandwidth_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::Bandwidth,
        schedule_seconds: 60, // Use minimum valid value
        name: "Test Bandwidth".to_string(),
        timeout: None,
        params: TaskParams::Bandwidth(BandwidthParams {
            timeout_seconds: 60,
            max_retries: 10,
            target_id: None,
        }),
    };

    // Validate the task config first
    assert!(task_config.validate().is_ok());

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test Bandwidth");
    // Note: The task may fail due to no server running, but it should at least try
    // and return a result with proper error handling
}

#[tokio::test]
async fn test_bandwidth_task_validation() {
    // Test that bandwidth tasks with schedule < 60 seconds fail validation
    let invalid_task_config = TaskConfig {
        task_type: TaskType::Bandwidth,
        schedule_seconds: 30, // Less than minimum 60 seconds
        name: "Invalid Bandwidth Test".to_string(),
        timeout: None,
        params: TaskParams::Bandwidth(BandwidthParams {
            timeout_seconds: 60,
            max_retries: 10,
            target_id: None,
        }),
    };

    assert!(invalid_task_config.validate().is_err());

    // Test that bandwidth tasks with schedule >= 60 seconds pass validation
    let valid_task_config = TaskConfig {
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

    assert!(valid_task_config.validate().is_ok());
}

#[tokio::test]
async fn test_ping_timeout() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    // Use a non-routable IP address that should timeout
    let task_config = TaskConfig {
        task_type: TaskType::Ping,
        schedule_seconds: 10,
        name: "Test Ping Timeout".to_string(),
        timeout: None,
        params: TaskParams::Ping(PingParams {
            host: "192.0.2.1".to_string(), // TEST-NET-1, reserved for documentation
            timeout_seconds: 1,
            target_id: None,
        }),
    };

    let start = std::time::Instant::now();
    let result = executor.execute_task(&task_config).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test Ping Timeout");

    // Verify timeout was respected (should be around 1 second, not hang indefinitely)
    assert!(
        elapsed.as_secs() <= 3,
        "Timeout took too long: {:?}",
        elapsed
    );

    // The key assertion: timeout was respected and task completed within reasonable time
    // This proves the timeout mechanism is working (task didn't hang indefinitely)
}

#[tokio::test]
async fn test_http_timeout() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    // Use a non-routable IP that will timeout
    let task_config = TaskConfig {
        task_type: TaskType::HttpGet,
        schedule_seconds: 30,
        name: "Test HTTP Timeout".to_string(),
        timeout: None,
        params: TaskParams::HttpGet(HttpGetParams {
            url: "http://192.0.2.1/test".to_string(), // TEST-NET-1
            timeout_seconds: 2,
            headers: HashMap::new(),
            verify_ssl: false,
            target_id: None,
        }),
    };

    let start = std::time::Instant::now();
    let result = executor.execute_task(&task_config).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test HTTP Timeout");

    // Verify timeout was respected (should be around 2 seconds)
    assert!(
        elapsed.as_secs() <= 5,
        "HTTP timeout took too long: {:?}",
        elapsed
    );

    // The key assertion: timeout was respected and task completed within reasonable time
    // This proves the timeout mechanism is working (task didn't hang indefinitely)
}

#[tokio::test]
async fn test_dns_timeout() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    // Use an IP that doesn't respond to DNS queries
    let task_config = TaskConfig {
        task_type: TaskType::DnsQuery,
        schedule_seconds: 60,
        name: "Test DNS Timeout".to_string(),
        timeout: None,
        params: TaskParams::DnsQuery(DnsQueryParams {
            server: "192.0.2.1:53".to_string(), // TEST-NET-1
            domain: "example.com".to_string(),
            record_type: DnsRecordType::A,
            timeout_seconds: 2,
            expected_ip: None,
            target_id: None,
        }),
    };

    let start = std::time::Instant::now();
    let result = executor.execute_task(&task_config).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test DNS Timeout");

    // Verify timeout was respected (should be around 2 seconds, not hang indefinitely)
    assert!(
        elapsed.as_secs() <= 5,
        "DNS timeout took too long: {:?}",
        elapsed
    );

    // The key assertion: timeout was respected and task completed within reasonable time
    // This proves the timeout mechanism is working (task didn't hang indefinitely)
}

#[tokio::test]
async fn test_http_task_failed_domain() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::HttpGet,
        schedule_seconds: 30,
        name: "Test HTTP Failure".to_string(),
        timeout: None,
        params: TaskParams::HttpGet(HttpGetParams {
            url: "https://non-existent-domain-12345.com".to_string(),
            timeout_seconds: 5,
            headers: HashMap::new(),
            verify_ssl: false,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test HTTP Failure");
    assert!(!task_result.success);
    assert!(task_result.error.is_some());
}

#[tokio::test]
async fn test_dns_task_expected_ip_match() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::DnsQuery,
        schedule_seconds: 60,
        name: "Test DNS Expected IP Match".to_string(),
        timeout: None,
        params: TaskParams::DnsQuery(DnsQueryParams {
            server: "1.1.1.1:53".to_string(),
            domain: "one.one.one.one".to_string(),
            record_type: DnsRecordType::A,
            timeout_seconds: 5,
            expected_ip: Some("1.1.1.1".to_string()),
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test DNS Expected IP Match");
    assert!(
        task_result.success,
        "Task failed when it should have succeeded: {:?}",
        task_result.error
    );
}

#[tokio::test]
async fn test_dns_task_expected_ip_mismatch() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::DnsQuery,
        schedule_seconds: 60,
        name: "Test DNS Expected IP Mismatch".to_string(),
        timeout: None,
        params: TaskParams::DnsQuery(DnsQueryParams {
            server: "1.1.1.1:53".to_string(),
            domain: "one.one.one.one".to_string(),
            record_type: DnsRecordType::A,
            timeout_seconds: 5,
            expected_ip: Some("1.2.3.4".to_string()),
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test DNS Expected IP Mismatch");
    assert!(!task_result.success);
    assert!(task_result.error.is_some());
    assert!(task_result
        .error
        .unwrap()
        .contains("does not match expected IP"));
}

#[tokio::test]
async fn test_tcp_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::Tcp,
        schedule_seconds: 10,
        name: "Test TCP".to_string(),
        timeout: None,
        params: TaskParams::Tcp(TcpParams {
            host: "google.com:80".to_string(),
            timeout_seconds: 5,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test TCP");
    assert!(task_result.success);
}

#[tokio::test]
async fn test_tls_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    // Test with SSL verification enabled (should fail)
    let task_config_verify = TaskConfig {
        task_type: TaskType::TlsHandshake,
        schedule_seconds: 30,
        name: "Test TLS Verify".to_string(),
        timeout: None,
        params: TaskParams::TlsHandshake(TlsHandshakeParams {
            host: "expired.badssl.com:443".to_string(),
            verify_ssl: true,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config_verify).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test TLS Verify");
    assert!(!task_result.success);

    // Test with SSL verification disabled (should succeed)
    let task_config_no_verify = TaskConfig {
        task_type: TaskType::TlsHandshake,
        schedule_seconds: 30,
        name: "Test TLS No Verify".to_string(),
        timeout: None,
        params: TaskParams::TlsHandshake(TlsHandshakeParams {
            host: "expired.badssl.com:443".to_string(),
            verify_ssl: false,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config_no_verify).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test TLS No Verify");
    assert!(task_result.success);
}

#[tokio::test]
async fn test_http_content_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::HttpContent,
        schedule_seconds: 30,
        name: "Test HTTP Content".to_string(),
        timeout: None,
        params: TaskParams::HttpContent(HttpContentParams {
            url: "https://example.com".to_string(),
            regexp: "Example Domain".to_string(),
            timeout_seconds: 10,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test HTTP Content");
    assert!(task_result.success);
}

#[tokio::test]
async fn test_dns_doh_task_execution() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    let task_config = TaskConfig {
        task_type: TaskType::DnsQueryDoh,
        schedule_seconds: 60,
        name: "Test DoH".to_string(),
        timeout: None,
        params: TaskParams::DnsQueryDoh(DnsQueryDohParams {
            server_url: "https://chrome.cloudflare-dns.com/dns-query".to_string(),
            domain: "google.com".to_string(),
            record_type: DnsRecordType::A,
            timeout_seconds: 5,
            expected_ip: None,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test DoH");
    assert!(task_result.success);
}

#[tokio::test]
async fn test_http_get_ssl_verification() {
    let (sender, mut receiver) = mpsc::channel(100);
    let executor = TaskExecutor::new(sender, None, None, None).unwrap();

    // Test with SSL verification enabled (should fail)
    let task_config_verify = TaskConfig {
        task_type: TaskType::HttpGet,
        schedule_seconds: 30,
        name: "Test HTTP Verify SSL".to_string(),
        timeout: None,
        params: TaskParams::HttpGet(HttpGetParams {
            url: "https://expired.badssl.com/".to_string(),
            timeout_seconds: 10,
            headers: HashMap::new(),
            verify_ssl: true,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config_verify).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test HTTP Verify SSL");
    assert!(!task_result.success);

    // Test with SSL verification disabled (should succeed)
    let task_config_no_verify = TaskConfig {
        task_type: TaskType::HttpGet,
        schedule_seconds: 30,
        name: "Test HTTP No Verify SSL".to_string(),
        timeout: None,
        params: TaskParams::HttpGet(HttpGetParams {
            url: "https://expired.badssl.com/".to_string(),
            timeout_seconds: 10,
            headers: HashMap::new(),
            verify_ssl: false,
            target_id: None,
        }),
    };

    let result = executor.execute_task(&task_config_no_verify).await;
    assert!(result.is_ok());

    let task_result = receiver.recv().await.unwrap();
    assert_eq!(task_result.task_name, "Test HTTP No Verify SSL");
    assert!(task_result.success);
}
