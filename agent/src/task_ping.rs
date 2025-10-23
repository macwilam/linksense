//! ICMP Ping task implementation for network monitoring
//!
//! This module provides async ICMP ping functionality with DNS resolution,
//! automatic IP fallback, and comprehensive error handling.

use anyhow::Result;
use ping_async::IcmpEchoRequestor;
use shared::config::{PingParams, TaskConfig, TaskType};
use shared::metrics::{MetricData, RawMetricData, RawPingMetric};
use std::net::IpAddr;
use std::time::Duration;
use tracing::{debug, warn};

/// Time-to-live for ICMP packets
const ICMP_TTL: u8 = 255;

/// Helper function to create a ping error metric
fn create_ping_error_metric(
    task_name: &str,
    error: String,
    ip_address: String,
    domain: Option<&str>,
    target_id: Option<&str>,
) -> MetricData {
    MetricData::new(
        task_name.to_string(),
        TaskType::Ping,
        RawMetricData::Ping(RawPingMetric {
            rtt_ms: None,
            success: false,
            error: Some(error),
            ip_address,
            domain: domain.map(|s| s.to_string()),
            target_id: target_id.map(|s| s.to_string()),
        }),
    )
}

/// Executes ping with DNS resolution and IP fallback
async fn execute_ping_with_resolution(task_config: &TaskConfig, params: &PingParams) -> MetricData {
    // Try to parse as IP address first
    let (target_ips, domain) = match params.host.parse::<IpAddr>() {
        Ok(ip) => (vec![ip], None),
        Err(_) => {
            // Not an IP address, try to resolve as hostname
            debug!(
                "Host '{}' is not an IP, attempting DNS resolution",
                params.host
            );
            match tokio::net::lookup_host(format!("{}:0", params.host)).await {
                Ok(addrs) => {
                    let mut ips: Vec<IpAddr> = addrs.map(|addr| addr.ip()).collect();

                    if ips.is_empty() {
                        return create_ping_error_metric(
                            &task_config.name,
                            format!("DNS resolution returned no addresses for: {}", params.host),
                            String::new(),
                            Some(&params.host),
                            params.target_id.as_deref(),
                        );
                    }

                    // Shuffle IPs for better load balancing across multiple runs
                    use rand::seq::SliceRandom;
                    let mut rng = rand::thread_rng();
                    ips.shuffle(&mut rng);

                    debug!(
                        "Resolved '{}' to {} address(es): {:?}",
                        params.host,
                        ips.len(),
                        ips
                    );
                    (ips, Some(params.host.as_str()))
                }
                Err(e) => {
                    return create_ping_error_metric(
                        &task_config.name,
                        format!("DNS resolution failed for {}: {}", params.host, e),
                        String::new(),
                        Some(&params.host),
                        params.target_id.as_deref(),
                    );
                }
            }
        }
    };

    // Try each resolved IP until one succeeds or all fail
    let mut last_error = String::new();
    for (i, target_ip) in target_ips.iter().enumerate() {
        if i > 0 {
            debug!("Trying fallback IP: {}", target_ip);
        }

        let pinger = match IcmpEchoRequestor::new(*target_ip, None, Some(ICMP_TTL), None) {
            Ok(req) => req,
            Err(e) => {
                let error_msg = format!("Cannot initiate ICMP ping: {}", e);

                // Provide helpful guidance for common permission issues
                let enhanced_error = if error_msg.contains("Operation not permitted")
                    || error_msg.contains("Permission denied")
                {
                    format!(
                        "{}. Hint: On Linux, add user to 'ping' group or set capabilities: \
                        sudo setcap cap_net_raw+ep /path/to/agent",
                        error_msg
                    )
                } else {
                    error_msg
                };

                last_error = enhanced_error.clone();

                // Only return error immediately if this is the last IP to try
                if i == target_ips.len() - 1 {
                    return create_ping_error_metric(
                        &task_config.name,
                        enhanced_error,
                        target_ip.to_string(),
                        domain,
                        params.target_id.as_deref(),
                    );
                }
                continue;
            }
        };

        // Send ping
        match pinger.send().await {
            Ok(reply) => {
                let rtt_ms = (reply.round_trip_time().as_micros() as f64) / 1000.0;

                if i > 0 {
                    debug!("Ping succeeded on fallback IP {}: {} ms", target_ip, rtt_ms);
                }

                return MetricData::new(
                    task_config.name.clone(),
                    TaskType::Ping,
                    RawMetricData::Ping(RawPingMetric {
                        rtt_ms: Some(rtt_ms),
                        success: true,
                        error: None,
                        ip_address: target_ip.to_string(),
                        domain: domain.map(|s| s.to_string()),
                        target_id: params.target_id.clone(),
                    }),
                );
            }
            Err(e) => {
                last_error = format!("Ping failed: {}", e);

                if i < target_ips.len() - 1 {
                    warn!("Ping to {} failed: {}, trying next address", target_ip, e);
                    continue;
                }
                // This was the last IP, fall through to error return
            }
        }
    }

    // All IPs failed
    create_ping_error_metric(
        &task_config.name,
        last_error,
        target_ips
            .first()
            .map(|ip| ip.to_string())
            .unwrap_or_default(),
        domain,
        params.target_id.as_deref(),
    )
}

/// Execute a ping task with timeout
///
/// This is the main entry point for executing ping tasks. It wraps the ping
/// execution with a timeout and handles DNS resolution and IP fallback.
pub async fn execute_ping_task(task_config: &TaskConfig) -> Result<MetricData> {
    debug!("Executing ping task: {}", task_config.name);

    if let shared::config::TaskParams::Ping(params) = &task_config.params {
        let timeout = Duration::from_secs(params.timeout_seconds as u64);

        // Wrap the entire operation (DNS + ping) in timeout
        let result = tokio::time::timeout(timeout, async {
            execute_ping_with_resolution(task_config, params).await
        })
        .await;

        match result {
            Ok(metric) => Ok(metric),
            Err(_) => {
                // Timeout occurred during DNS resolution or ping
                Ok(create_ping_error_metric(
                    &task_config.name,
                    format!("Operation timed out after {}s", params.timeout_seconds),
                    String::new(),
                    Some(&params.host),
                    params.target_id.as_deref(),
                ))
            }
        }
    } else {
        Err(anyhow::anyhow!("Invalid parameters for ping task"))
    }
}
