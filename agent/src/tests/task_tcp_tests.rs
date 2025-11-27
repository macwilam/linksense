//! Tests for TCP connection task implementation

use crate::task_tcp::execute_tcp_task;
use shared::config::TcpParams;

#[tokio::test]
async fn test_tcp_task_success() {
    // Test connection to a well-known public DNS server (Google DNS on port 53)
    let params = TcpParams {
        host: "8.8.8.8:53".to_string(),
        timeout_seconds: 5,
        target_id: Some("google-dns".to_string()),
    };

    let result = execute_tcp_task(&params).await;
    assert!(result.success, "Connection to 8.8.8.8:53 should succeed");
    assert!(result.connect_time_ms.is_some());
    assert!(result.error.is_none());
    assert_eq!(result.host, "8.8.8.8:53");
    assert_eq!(result.target_id, Some("google-dns".to_string()));
}

#[tokio::test]
async fn test_tcp_task_connection_refused() {
    // Test connection to localhost on a port that's likely not listening
    let params = TcpParams {
        host: "127.0.0.1:65534".to_string(),
        timeout_seconds: 2,
        target_id: None,
    };

    let result = execute_tcp_task(&params).await;
    assert!(!result.success, "Connection should fail");
    assert!(result.connect_time_ms.is_none());
    assert!(result.error.is_some());
    assert_eq!(result.host, "127.0.0.1:65534");
}

#[tokio::test]
async fn test_tcp_task_invalid_host() {
    let params = TcpParams {
        host: "invalid-host-that-does-not-exist.invalid:80".to_string(),
        timeout_seconds: 2,
        target_id: Some("invalid".to_string()),
    };

    let result = execute_tcp_task(&params).await;
    assert!(!result.success);
    assert!(result.connect_time_ms.is_none());
    assert!(result.error.is_some());
    assert!(result
        .error
        .as_ref()
        .unwrap()
        .contains("Failed to parse host"));
}

#[tokio::test]
async fn test_tcp_task_timeout() {
    // Use a non-routable IP to trigger timeout (192.0.2.0/24 is TEST-NET-1)
    let params = TcpParams {
        host: "192.0.2.1:80".to_string(),
        timeout_seconds: 1,
        target_id: None,
    };

    let result = execute_tcp_task(&params).await;
    assert!(!result.success);
    assert!(result.connect_time_ms.is_none());
    assert!(result.error.is_some());
    assert!(result.error.as_ref().unwrap().contains("timeout"));
}
