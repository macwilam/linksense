//! TCP connection monitoring task implementation
//!
//! This module provides the most fundamental network connectivity test - raw TCP socket connection.
//! It measures pure TCP connection establishment time without any protocol overhead.
//!
//! This is the base layer of the network monitoring pyramid:
//! - TCP (this module) - fundamental socket connection
//! - TLS (imports from TCP) - adds TLS handshake
//! - HTTP (imports from TLS) - adds HTTP protocol

use shared::config::TcpParams;
use shared::metrics::RawTcpMetric;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Establish TCP connection and measure timing
///
/// This is the fundamental building block used by higher-level tasks (TLS, HTTP).
///
/// # Arguments
/// * `addr` - Socket address to connect to
///
/// # Returns
/// * `Ok((Duration, TcpStream))` - Connection time and established stream
/// * `Err` - Connection failed
pub async fn get_tcp_timing(addr: &SocketAddr) -> Result<(Duration, TcpStream), std::io::Error> {
    let start = std::time::Instant::now();
    let stream = TcpStream::connect(addr).await?;
    Ok((start.elapsed(), stream))
}

/// Execute a TCP connection test
///
/// This function attempts to establish a TCP connection to the specified host:port,
/// measuring the connection time and reporting success or failure.
///
/// # Arguments
/// * `params` - TCP task parameters containing host, timeout, and target_id
///
/// # Returns
/// * `RawTcpMetric` - Connection timing and status information
pub async fn execute_tcp_task(params: &TcpParams) -> RawTcpMetric {
    let timeout_duration = Duration::from_secs(params.timeout_seconds as u64);

    // Parse host:port into socket address
    // Supports IPv6 addresses like "[2001:db8::1]:443" and automatic IPv4/IPv6 resolution
    // Uses first available address from DNS resolution (could be IPv4 or IPv6)
    let socket_addr = match params.host.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(addr) = addrs.next() {
                addr
            } else {
                return RawTcpMetric {
                    connect_time_ms: None,
                    success: false,
                    error: Some("Failed to parse host: no addresses found".to_string()),
                    host: params.host.clone(),
                    target_id: params.target_id.clone(),
                };
            }
        }
        Err(e) => {
            return RawTcpMetric {
                connect_time_ms: None,
                success: false,
                error: Some(format!("Failed to parse host: {}", e)),
                host: params.host.clone(),
                target_id: params.target_id.clone(),
            };
        }
    };

    // Attempt TCP connection with timeout
    match timeout(timeout_duration, get_tcp_timing(&socket_addr)).await {
        Ok(Ok((connect_time, stream))) => {
            // Connection successful - explicitly drop the stream to ensure cleanup
            // This prevents potential file descriptor accumulation in high-frequency monitoring
            drop(stream);

            RawTcpMetric {
                connect_time_ms: Some(connect_time.as_secs_f64() * 1000.0),
                success: true,
                error: None,
                host: params.host.clone(),
                target_id: params.target_id.clone(),
            }
        }
        Ok(Err(e)) => {
            // Connection failed
            RawTcpMetric {
                connect_time_ms: None,
                success: false,
                error: Some(format!("Connection error: {}", e)),
                host: params.host.clone(),
                target_id: params.target_id.clone(),
            }
        }
        Err(_) => {
            // Timeout
            RawTcpMetric {
                connect_time_ms: None,
                success: false,
                error: Some(format!(
                    "Connection timeout after {} seconds",
                    params.timeout_seconds
                )),
                host: params.host.clone(),
                target_id: params.target_id.clone(),
            }
        }
    }
}
