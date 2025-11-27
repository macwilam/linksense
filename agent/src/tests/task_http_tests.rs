//! Tests for HTTP timing task implementation

use crate::task_http::from_string;
use crate::task_tls::{create_tls_connector_without_verification, error};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn test_non_tls_connection() {
    let connector =
        create_tls_connector_without_verification().expect("Failed to create TLS connector");
    // Note: neverssl.com sometimes redirects to http.
    // We will use a more stable http-only target.
    let url = "http://info.cern.ch"; // The first website
    let result = from_string(url, Some(TIMEOUT), &connector).await;
    assert!(result.is_ok());
    let response = result.expect("Expected successful HTTP response");
    assert_eq!(response.status, 200);
    assert!(response.timings.tls.is_none());
    assert!(response.timings.content_download < TIMEOUT);
}

#[tokio::test]
async fn test_popular_tls_connection() {
    let connector =
        create_tls_connector_without_verification().expect("Failed to create TLS connector");
    let url = "https://www.google.com";
    let result = from_string(url, Some(TIMEOUT), &connector).await;
    assert!(result.is_ok());
    let response = result.expect("Expected successful HTTPS response");
    // Google might return 301/302 for redirection based on location
    assert!(response.status >= 200 && response.status < 400);
    assert!(response.certificate_information.is_some());
    assert!(response.timings.tls.is_some());
    assert!(response.timings.content_download < TIMEOUT);
}

#[tokio::test]
async fn test_ip() {
    let connector =
        create_tls_connector_without_verification().expect("Failed to create TLS connector");
    let url = "1.1.1.1"; // This will default to https://1.1.1.1
    let result = from_string(url, Some(TIMEOUT), &connector).await;
    assert!(result.is_ok());
    let response = result.expect("Expected successful IP connection response");
    // Expect a redirect to the hostname
    assert!(response.status >= 300 && response.status < 400);
    assert!(response.timings.tls.is_some());
    assert!(response.timings.content_download < TIMEOUT);
}

#[tokio::test]
async fn test_timeout() {
    let connector =
        create_tls_connector_without_verification().expect("Failed to create TLS connector");
    // Use a non-routable address to force a timeout
    let url = "http://10.255.255.1";
    let result = from_string(url, Some(Duration::from_secs(1)), &connector).await;
    assert!(result.is_err());
    assert!(matches!(
        result.expect_err("Expected timeout error"),
        error::Error::Timeout(_)
    ));
}
