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
    let socket_addr = match params.host.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(addr) = addrs.next() {
                addr
            } else {
                return RawTcpMetric {
                    connect_time_ms: None,
                    success: false,
                    error: Some("Failed to resolve host".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
