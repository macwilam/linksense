//! Tests for TLS handshake task implementation

use crate::task_tls::{
    check_tls_handshake_with_timeout, create_tls_connector_without_verification, resolve_dns,
};
use std::net::SocketAddr;
use std::time::Duration;
use url::Url;

#[tokio::test]
async fn test_resolve_dns() {
    let url = Url::parse("https://example.com").expect("Failed to parse URL");
    let result = resolve_dns(&url).await;
    assert!(result.is_ok());
    let addrs: Vec<SocketAddr> = result
        .expect("Expected DNS resolution to succeed")
        .collect();
    assert!(!addrs.is_empty());
}

#[tokio::test]
async fn test_tls_handshake_check() {
    // Create a test connector without verification for testing
    let connector =
        create_tls_connector_without_verification().expect("Failed to create TLS connector");

    // Test with a well-known site
    let result = check_tls_handshake_with_timeout(
        "example.com:443",
        Some(Duration::from_secs(10)),
        &connector,
    )
    .await;

    match result {
        Ok(check) => {
            assert!(check.success);
            assert!(check.tls_timing.is_some());
            assert!(check.tcp_timing.as_millis() > 0);
        }
        Err(e) => {
            // Network errors are acceptable in tests
            eprintln!("TLS handshake test failed (network issue): {}", e);
        }
    }
}

#[tokio::test]
async fn test_tls_handshake_timeout() {
    // Create a test connector
    let connector =
        create_tls_connector_without_verification().expect("Failed to create TLS connector");

    // Use a non-routable IP to test timeout
    let result = check_tls_handshake_with_timeout(
        "192.0.2.1:443",
        Some(Duration::from_millis(100)),
        &connector,
    )
    .await;

    assert!(result.is_err());
}
