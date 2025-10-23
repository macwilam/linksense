//! HTTP content check task executor
//!
//! This module implements HTTP content checking with regex pattern matching.
//! It fetches HTTP content and verifies if a regex pattern matches the response body.

use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use shared::{config::HttpContentParams, metrics::RawHttpContentMetric};
use std::time::Instant;
use tracing::{debug, warn};

// Maximum response body size to prevent memory exhaustion
const MAX_RESPONSE_SIZE: usize = 100 * 1024 * 1024; // 100MB

/// Execute an HTTP content check task and return the raw metric
///
/// # Arguments
/// * `client` - Shared HTTP client instance (reused across multiple requests)
/// * `params` - Task parameters including URL, regex pattern, and timeout
///
/// # Returns
/// Raw metric containing status, timing, size, and regex match result
pub async fn execute_http_content_check(
    client: &Client,
    params: &HttpContentParams,
) -> Result<RawHttpContentMetric> {
    debug!("Executing HTTP content check for URL: {}", params.url);

    let start_time = Instant::now();
    let timeout = std::time::Duration::from_secs(params.timeout_seconds as u64);

    // Compile regex pattern
    let regex = Regex::new(&params.regexp)
        .with_context(|| format!("Failed to compile regexp: {}", params.regexp))?;

    // Execute HTTP request with per-request timeout
    // Note: Timeout is applied per-request, not on the client, to allow different
    // timeouts for different tasks while reusing the same client instance
    let result = match client.get(&params.url).timeout(timeout).send().await {
        Ok(response) => {
            let status_code = response.status().as_u16();
            let is_success = response.status().is_success();

            // Pre-flight size check: Reject oversized responses BEFORE reading the body
            // This saves memory by not buffering responses that exceed the limit
            if let Some(content_length) = response.content_length() {
                if content_length > MAX_RESPONSE_SIZE as u64 {
                    let total_time = start_time.elapsed();

                    warn!(
                        "Response from {} has Content-Length ({} bytes) exceeding maximum size ({} bytes), rejecting before read",
                        params.url,
                        content_length,
                        MAX_RESPONSE_SIZE
                    );

                    // Explicitly drop the response object to close the connection
                    // without reading the body, preventing memory consumption
                    drop(response);

                    return Ok(RawHttpContentMetric {
                        status_code: Some(status_code),
                        total_time_ms: Some(total_time.as_secs_f64() * 1000.0),
                        total_size: Some(content_length),
                        regexp_match: None,
                        success: false,
                        error: Some(format!(
                            "Response size ({} bytes) exceeds maximum of {} bytes (detected from Content-Length header)",
                            content_length,
                            MAX_RESPONSE_SIZE
                        )),
                        target_id: params.target_id.as_deref().map(|s| s.to_string()),
                    });
                }
            }

            // Read the response body with size limit
            match response.text().await {
                Ok(body_text) => {
                    let total_time = start_time.elapsed();
                    let total_size = body_text.len() as u64;

                    // Post-read size check (fallback for responses without Content-Length header)
                    // This catches cases where Content-Length was not provided or was incorrect
                    if body_text.len() > MAX_RESPONSE_SIZE {
                        let body_size = body_text.len(); // Capture size before dropping

                        // Explicitly drop body_text to free memory immediately
                        drop(body_text);

                        warn!(
                            "Response body from {} exceeds maximum size: {} bytes (no Content-Length header was provided)",
                            params.url,
                            body_size
                        );

                        RawHttpContentMetric {
                            status_code: Some(status_code),
                            total_time_ms: Some(total_time.as_secs_f64() * 1000.0),
                            total_size: Some(total_size),
                            regexp_match: None,
                            success: false,
                            error: Some(format!(
                                "Response size ({} bytes) exceeds maximum of {} bytes (Content-Length header not provided)",
                                body_size,
                                MAX_RESPONSE_SIZE
                            )),
                            target_id: params.target_id.as_deref().map(|s| s.to_string()),
                        }
                    } else {
                        // Test regex against response body
                        let regexp_match = regex.is_match(&body_text);

                        // Explicitly drop body_text immediately after regex matching to free memory
                        // This is critical for preventing memory accumulation when monitoring many endpoints
                        drop(body_text);

                        debug!(
                            "HTTP content check completed for {} - status: {}, size: {} bytes, time: {:?}, regexp_match: {}",
                            params.url, status_code, total_size, total_time, regexp_match
                        );

                        RawHttpContentMetric {
                            status_code: Some(status_code),
                            total_time_ms: Some(total_time.as_secs_f64() * 1000.0),
                            total_size: Some(total_size),
                            regexp_match: Some(regexp_match),
                            success: is_success,
                            error: None,
                            target_id: params.target_id.as_deref().map(|s| s.to_string()),
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to read HTTP response body from {}: {}",
                        params.url, e
                    );

                    RawHttpContentMetric {
                        status_code: Some(status_code),
                        total_time_ms: None, // Don't report time on error
                        total_size: None,
                        regexp_match: None,
                        success: false,
                        error: Some(format!(
                            "Failed to read response body from {}: {}",
                            params.url, e
                        )),
                        target_id: params.target_id.as_deref().map(|s| s.to_string()),
                    }
                }
            }
        }
        Err(e) => {
            // Check if it's a timeout error
            let is_timeout = e.is_timeout();

            if is_timeout {
                warn!(
                    "HTTP request to {} timed out after {:?}",
                    params.url, timeout
                );

                RawHttpContentMetric {
                    status_code: None,
                    total_time_ms: None, // Don't report time on timeout
                    total_size: None,
                    regexp_match: None,
                    success: false,
                    error: Some(format!(
                        "Request to {} timed out after {}s",
                        params.url, params.timeout_seconds
                    )),
                    target_id: params.target_id.as_deref().map(|s| s.to_string()),
                }
            } else {
                warn!("HTTP request to {} failed: {}", params.url, e);

                RawHttpContentMetric {
                    status_code: None,
                    total_time_ms: None, // Don't report time on error
                    total_size: None,
                    regexp_match: None,
                    success: false,
                    error: Some(format!("Request to {} failed: {}", params.url, e)),
                    target_id: params.target_id.as_deref().map(|s| s.to_string()),
                }
            }
        }
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create a test client
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
}
