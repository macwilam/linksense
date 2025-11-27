//! Tests for HTTP content check task implementation

use crate::task_http_content::execute_http_content_check;
use reqwest::Client;
use shared::config::HttpContentParams;

/// Helper function to create a test client
fn create_test_client() -> Client {
    Client::builder()
        .build()
        .expect("Failed to create test client")
}

#[tokio::test]
async fn test_http_content_check_invalid_url() {
    // Test with invalid URL to verify error handling
    let client = create_test_client();
    let params = HttpContentParams {
        url: "http://this-domain-should-not-exist-12345.invalid".to_string(),
        regexp: "test".to_string(),
        timeout_seconds: 2,
        target_id: None,
    };

    let result = execute_http_content_check(&client, &params).await;
    assert!(result.is_ok());

    let metric = result.unwrap();
    assert!(!metric.success);
    assert!(metric.error.is_some());
}

#[tokio::test]
async fn test_http_content_check_invalid_regex() {
    // Test with invalid regex pattern
    let client = create_test_client();
    let params = HttpContentParams {
        url: "http://example.com".to_string(),
        regexp: "[invalid(regex".to_string(),
        timeout_seconds: 5,
        target_id: None,
    };

    let result = execute_http_content_check(&client, &params);
    assert!(result.await.is_err());
}

#[tokio::test]
async fn test_http_content_check_regex_compilation() {
    // Test that valid regex patterns work correctly
    let client = create_test_client();
    let params = HttpContentParams {
        url: "http://example.com".to_string(),
        regexp: r"^\d{3}-\d{2}-\d{4}$".to_string(),
        timeout_seconds: 5,
        target_id: None,
    };

    // This should compile the regex successfully and attempt the request
    // The request itself may fail (network dependent), but regex compilation should work
    let result = execute_http_content_check(&client, &params).await;
    assert!(result.is_ok());
}
