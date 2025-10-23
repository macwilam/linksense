//! Bandwidth measurement task implementation
//!
//! This module implements network bandwidth testing through server coordination.
//! The agent requests permission from the server to perform a bandwidth test,
//! then downloads test data to measure throughput.

use anyhow::Result;
use chrono::Utc;
use shared::config::BandwidthParams;
use shared::metrics::RawBandwidthMetric;
use std::time::Duration;
use tracing::debug;

/// Execute bandwidth measurement task with server coordination
///
/// Implements a retry loop to handle server queueing:
/// 1. Requests permission from server to start test
/// 2. If server says "Delay" → waits and retries
/// 3. If server says "Proceed" → downloads test data and measures throughput
/// 4. Respects task timeout and maximum retry attempts
///
/// Returns raw bandwidth metric with speed in Mbps.
pub async fn execute_bandwidth_task(
    params: &BandwidthParams,
    server_url: &str,
    api_key: &str,
    agent_id: &str,
) -> Result<RawBandwidthMetric> {
    let total_timeout_secs = params.timeout_seconds as u64;
    let max_retries = params.max_retries;
    let start_time = std::time::Instant::now();
    let permission_timeout = Duration::from_secs(10); // Fixed 10s timeout for permission requests

    let client = reqwest::Client::new();
    let mut retry_count = 0;
    let data_size_bytes;

    // Retry loop for requesting permission from server
    loop {
        // Check if we've exceeded total timeout
        let elapsed = start_time.elapsed().as_secs();
        if elapsed >= total_timeout_secs {
            return Err(anyhow::anyhow!(
                "Bandwidth test timed out after {} retry attempts ({} seconds elapsed)",
                retry_count,
                elapsed
            ));
        }

        if retry_count >= max_retries {
            return Err(anyhow::anyhow!(
                "Exceeded maximum retry attempts ({}) waiting for bandwidth test slot",
                max_retries
            ));
        }

        // Request permission from server
        debug!(
            "Requesting bandwidth test permission from server (attempt {}/{})",
            retry_count + 1,
            max_retries
        );

        let test_request = shared::api::BandwidthTestRequest {
            agent_id: agent_id.to_string(),
            timestamp_utc: Utc::now().to_rfc3339(),
        };

        let response = client
            .post(&format!(
                "{}{}",
                server_url,
                shared::api::endpoints::BANDWIDTH_TEST
            ))
            .header(shared::api::headers::API_KEY, api_key)
            .header("Content-Type", "application/json")
            .json(&test_request)
            .timeout(permission_timeout)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to request bandwidth test: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Server rejected bandwidth test request: {}",
                response.status()
            ));
        }

        let test_response: shared::api::BandwidthTestResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse bandwidth test response: {}", e))?;

        match test_response.action {
            shared::api::BandwidthTestAction::Delay => {
                let delay_seconds = test_response.delay_seconds.unwrap_or(60);

                // Check if waiting would exceed task timeout
                let time_after_delay = start_time.elapsed().as_secs() + delay_seconds as u64;
                if time_after_delay >= total_timeout_secs {
                    return Err(anyhow::anyhow!(
                        "Cannot wait {} seconds - would exceed task timeout ({} seconds remaining)",
                        delay_seconds,
                        total_timeout_secs.saturating_sub(start_time.elapsed().as_secs())
                    ));
                }

                debug!(
                    "Server requested delay of {}s, waiting before retry... (attempt {}/{})",
                    delay_seconds,
                    retry_count + 1,
                    max_retries
                );

                tokio::time::sleep(Duration::from_secs(delay_seconds as u64)).await;
                retry_count += 1;
                continue; // Loop back to request permission again
            }
            shared::api::BandwidthTestAction::Proceed => {
                // Got permission! Extract data size and break out of retry loop
                data_size_bytes = test_response
                    .data_size_bytes
                    .ok_or_else(|| anyhow::anyhow!("Server did not provide data size"))?;

                debug!(
                    "Server granted bandwidth test permission after {} attempts",
                    retry_count + 1
                );
                break;
            }
        }
    }

    // Step 2: Download test data and measure bandwidth
    // Note: The server controls the download size via its configuration.
    // We only send agent_id for identification, not size parameters.

    // Calculate remaining timeout for download (accounting for time spent in retry loop)
    let elapsed_in_retries = start_time.elapsed().as_secs();
    let remaining_timeout_secs = total_timeout_secs.saturating_sub(elapsed_in_retries);

    let download_timeout = Duration::from_secs(remaining_timeout_secs);
    let download_start = std::time::Instant::now();

    let download_response = client
        .get(&format!(
            "{}{}",
            server_url,
            shared::api::endpoints::BANDWIDTH_DOWNLOAD
        ))
        .query(&[("agent_id", agent_id)])
        .timeout(download_timeout)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start bandwidth download: {}", e))?;

    if !download_response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Bandwidth download failed: {}",
            download_response.status()
        ));
    }

    // Stream bytes without buffering entire response in memory
    // This prevents memory leaks for large bandwidth test files (e.g., 100MB+)
    use futures_util::StreamExt;

    let mut bytes_downloaded: u64 = 0;
    let mut stream = download_response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result
            .map_err(|e| anyhow::anyhow!("Failed to download bandwidth test data: {}", e))?;
        bytes_downloaded += chunk.len() as u64;
    }

    let duration = download_start.elapsed();
    let duration_ms = duration.as_millis() as f64;

    // Calculate bandwidth in Mbps: (bytes * 8) / (seconds) / 1,000,000
    let bandwidth_mbps = if duration_ms > 0.0 {
        (bytes_downloaded as f64 * 8.0) / (duration_ms / 1000.0) / 1_000_000.0
    } else {
        0.0
    };

    debug!(
        "Bandwidth test completed: {} bytes in {:.2}ms = {:.2} Mbps",
        bytes_downloaded, duration_ms, bandwidth_mbps
    );

    Ok(RawBandwidthMetric {
        bandwidth_mbps: Some(bandwidth_mbps),
        duration_ms: Some(duration_ms),
        bytes_downloaded: Some(bytes_downloaded),
        success: true,
        error: None,
        target_id: params.target_id.clone(),
    })
}
