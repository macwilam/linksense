//! Metric data structures for network monitoring measurements
//!
//! This module defines the raw and aggregated metric types used to store
//! and transmit monitoring data between agent and server components.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Raw metric data from a single task execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricData {
    /// Name of the task that generated this metric
    pub task_name: String,
    /// Type of task
    pub task_type: crate::config::TaskType,
    /// Timestamp when the measurement was taken (Unix timestamp)
    pub timestamp: u64,
    /// Raw measurement data
    pub data: RawMetricData,
}

/// Aggregated metrics for a specific time period (typically 1 minute)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedMetrics {
    /// Name of the task
    pub task_name: String,
    /// Type of task
    #[serde(rename = "type")]
    pub task_type: crate::config::TaskType,
    /// Start timestamp of the aggregation period
    pub period_start: u64,
    /// End timestamp of the aggregation period
    pub period_end: u64,
    /// Number of raw measurements included in this aggregation
    pub sample_count: u32,
    /// Aggregated measurement data
    pub data: AggregatedMetricData,
}

/// Raw measurement data from individual task executions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RawMetricData {
    Ping(RawPingMetric),
    Tcp(RawTcpMetric),
    HttpGet(RawHttpMetric),
    HttpContent(RawHttpContentMetric),
    TlsHandshake(RawTlsMetric),
    DnsQuery(RawDnsMetric),
    Bandwidth(RawBandwidthMetric),
    #[cfg(feature = "sql-tasks")]
    SqlQuery(RawSqlQueryMetric),
}

/// Aggregated measurement data over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AggregatedMetricData {
    Ping(AggregatedPingMetric),
    Tcp(AggregatedTcpMetric),
    HttpGet(AggregatedHttpMetric),
    HttpContent(AggregatedHttpContentMetric),
    TlsHandshake(AggregatedTlsMetric),
    DnsQuery(AggregatedDnsMetric),
    Bandwidth(AggregatedBandwidthMetric),
    #[cfg(feature = "sql-tasks")]
    SqlQuery(AggregatedSqlQueryMetric),
}

/// Raw ping measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPingMetric {
    /// Round-trip time in milliseconds (None if packet was lost)
    pub rtt_ms: Option<f64>,
    /// Whether the ping was successful
    pub success: bool,
    /// Error message if the ping failed
    pub error: Option<String>,
    /// IP address that was actually pinged
    pub ip_address: String,
    /// Domain/hostname if the host in config was a domain (None if it was an IP)
    pub domain: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated ping metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedPingMetric {
    /// Average latency in milliseconds (only successful pings)
    pub avg_latency_ms: f64,
    /// Maximum latency in milliseconds (only successful pings)
    pub max_latency_ms: f64,
    /// Minimum latency in milliseconds (only successful pings)
    pub min_latency_ms: f64,
    /// Packet loss percentage (0.0 to 100.0)
    pub packet_loss_percent: f64,
    /// Number of successful pings
    pub successful_pings: u32,
    /// Number of failed pings
    pub failed_pings: u32,
    /// Domain/hostname if the host in config was a domain (first occurrence)
    pub domain: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw TCP connection measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTcpMetric {
    /// TCP connection time in milliseconds (None if connection failed)
    pub connect_time_ms: Option<f64>,
    /// Whether the connection was successful
    pub success: bool,
    /// Error message if the connection failed
    pub error: Option<String>,
    /// Host:port that was connected to
    pub host: String,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated TCP connection metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedTcpMetric {
    /// Average connection time in milliseconds (only successful connections)
    pub avg_connect_time_ms: f64,
    /// Maximum connection time in milliseconds (only successful connections)
    pub max_connect_time_ms: f64,
    /// Minimum connection time in milliseconds (only successful connections)
    pub min_connect_time_ms: f64,
    /// Connection failure percentage (0.0 to 100.0)
    pub failure_percent: f64,
    /// Number of successful connections
    pub successful_connections: u32,
    /// Number of failed connections
    pub failed_connections: u32,
    /// Host:port that was connected to (from first occurrence)
    pub host: String,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw HTTP GET measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawHttpMetric {
    /// HTTP status code (None if request failed)
    pub status_code: Option<u16>,
    /// TCP connection time in milliseconds
    pub tcp_timing_ms: Option<f64>,
    /// TLS handshake duration in milliseconds (if applicable)
    pub tls_timing_ms: Option<f64>,
    /// Time to first byte in milliseconds
    pub ttfb_timing_ms: Option<f64>,
    /// Content download time in milliseconds
    pub content_download_timing_ms: Option<f64>,
    /// Total request duration in milliseconds (excludes DNS resolution)
    pub total_time_ms: Option<f64>,
    /// Whether the request was successful (2xx status code)
    pub success: bool,
    /// Error message if the request failed
    pub error: Option<String>,
    /// Whether SSL certificate is valid (None if not HTTPS or not checked)
    pub ssl_valid: Option<bool>,
    /// Days until SSL certificate expires (None if not HTTPS or invalid cert)
    pub ssl_cert_days_until_expiry: Option<i64>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated HTTP metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedHttpMetric {
    /// Success rate as a percentage (0.0 to 100.0)
    pub success_rate_percent: f64,
    /// Average TCP timing in milliseconds
    pub avg_tcp_timing_ms: f64,
    /// Average TLS timing in milliseconds
    pub avg_tls_timing_ms: f64,
    /// Average time to first byte in milliseconds
    pub avg_ttfb_timing_ms: f64,
    /// Average content download timing in milliseconds
    pub avg_content_download_timing_ms: f64,
    /// Average total request time in milliseconds (excludes DNS resolution)
    pub avg_total_time_ms: f64,
    /// Maximum total request time in milliseconds (excludes DNS resolution)
    pub max_total_time_ms: f64,
    /// Number of successful requests (2xx status codes)
    pub successful_requests: u32,
    /// Number of failed requests
    pub failed_requests: u32,
    /// Distribution of HTTP status codes
    pub status_code_distribution: std::collections::HashMap<u16, u32>,
    /// Percentage of requests with valid SSL certificates (0-100, or None if not HTTPS)
    pub ssl_valid_percent: Option<f64>,
    /// Average days until SSL certificate expiry (None if not HTTPS)
    pub avg_ssl_cert_days_until_expiry: Option<f64>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw TLS handshake measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTlsMetric {
    /// TCP connection time in milliseconds
    pub tcp_timing_ms: Option<f64>,
    /// TLS handshake duration in milliseconds
    pub tls_timing_ms: Option<f64>,
    /// Whether SSL certificate is valid
    pub ssl_valid: Option<bool>,
    /// Days until SSL certificate expires
    pub ssl_cert_days_until_expiry: Option<i64>,
    /// Whether the check was successful
    pub success: bool,
    /// Error message if the check failed
    pub error: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated TLS handshake metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedTlsMetric {
    /// Success rate as a percentage (0.0 to 100.0)
    pub success_rate_percent: f64,
    /// Average TCP timing in milliseconds
    pub avg_tcp_timing_ms: f64,
    /// Average TLS timing in milliseconds
    pub avg_tls_timing_ms: f64,
    /// Number of successful checks
    pub successful_checks: u32,
    /// Number of failed checks
    pub failed_checks: u32,
    /// Percentage of checks with valid SSL certificates (0-100)
    pub ssl_valid_percent: f64,
    /// Average days until SSL certificate expiry
    pub avg_ssl_cert_days_until_expiry: f64,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw HTTP content check measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawHttpContentMetric {
    /// HTTP status code (None if request failed)
    pub status_code: Option<u16>,
    /// Total request duration in milliseconds
    pub total_time_ms: Option<f64>,
    /// Size of response body in bytes
    pub total_size: Option<u64>,
    /// Whether the regexp matched the content
    pub regexp_match: Option<bool>,
    /// Whether the request was successful
    pub success: bool,
    /// Error message if the request failed
    pub error: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated HTTP content check metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedHttpContentMetric {
    /// Success rate as a percentage (0.0 to 100.0)
    pub success_rate_percent: f64,
    /// Average total request time in milliseconds
    pub avg_total_time_ms: f64,
    /// Maximum total request time in milliseconds
    pub max_total_time_ms: f64,
    /// Average response size in bytes
    pub avg_total_size: f64,
    /// Regexp match rate as a percentage (0.0 to 100.0)
    pub regexp_match_rate_percent: f64,
    /// Number of successful requests
    pub successful_requests: u32,
    /// Number of failed requests
    pub failed_requests: u32,
    /// Number of requests where regexp matched
    pub regexp_matched_count: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw DNS query measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawDnsMetric {
    /// Query response time in milliseconds
    pub query_time_ms: Option<f64>,
    /// Whether the query was successful
    pub success: bool,
    /// Number of records returned (if successful)
    pub record_count: Option<u32>,
    /// Resolved IP addresses as a JSON array string
    pub resolved_addresses: Option<Vec<String>>,
    /// Domain name that was queried
    pub domain_queried: String,
    /// Error message if the query failed
    pub error: Option<String>,
    /// Expected IP address if configured (for validation)
    pub expected_ip: Option<String>,
    /// First resolved IP address (primary result)
    pub resolved_ip: Option<String>,
    /// Whether the resolution matches expected IP (true if no expected_ip configured)
    pub correct_resolution: bool,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated DNS metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedDnsMetric {
    /// Success rate as a percentage (0.0 to 100.0)
    pub success_rate_percent: f64,
    /// Average query time in milliseconds
    pub avg_query_time_ms: f64,
    /// Maximum query time in milliseconds
    pub max_query_time_ms: f64,
    /// Number of successful queries
    pub successful_queries: u32,
    /// Number of failed queries
    pub failed_queries: u32,
    /// Set of all unique addresses resolved during this period
    pub all_resolved_addresses: std::collections::HashSet<String>,
    /// Domain name that was queried for this task
    pub domain_queried: String,
    /// Percentage of resolutions matching expected IP (0-100, or 100 if no expected_ip)
    pub correct_resolution_percent: f64,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw bandwidth measurement data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawBandwidthMetric {
    /// Measured bandwidth in Mbps
    pub bandwidth_mbps: Option<f64>,
    /// Test duration in milliseconds
    pub duration_ms: Option<f64>,
    /// Number of bytes downloaded
    pub bytes_downloaded: Option<u64>,
    /// Whether the test was successful
    pub success: bool,
    /// Error message if the test failed
    pub error: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated bandwidth metrics over a time period
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedBandwidthMetric {
    /// Average bandwidth in Mbps
    pub avg_bandwidth_mbps: f64,
    /// Maximum bandwidth in Mbps
    pub max_bandwidth_mbps: f64,
    /// Minimum bandwidth in Mbps
    pub min_bandwidth_mbps: f64,
    /// Number of successful tests
    pub successful_tests: u32,
    /// Number of failed tests
    pub failed_tests: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Raw SQL query measurement data
#[cfg(feature = "sql-tasks")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawSqlQueryMetric {
    /// Total query execution time in milliseconds
    pub total_time_ms: Option<f64>,
    /// Number of rows returned by the query
    pub row_count: Option<u64>,
    /// Whether the query was successful
    pub success: bool,
    /// Error message if the query failed
    pub error: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Aggregated SQL query metrics over a time period
#[cfg(feature = "sql-tasks")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedSqlQueryMetric {
    /// Success rate as a percentage (0.0 to 100.0)
    pub success_rate_percent: f64,
    /// Average query execution time in milliseconds
    pub avg_total_time_ms: f64,
    /// Maximum query execution time in milliseconds
    pub max_total_time_ms: f64,
    /// Average number of rows returned
    pub avg_row_count: f64,
    /// Maximum number of rows returned
    pub max_row_count: u64,
    /// Number of successful queries
    pub successful_queries: u32,
    /// Number of failed queries
    pub failed_queries: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

impl MetricData {
    /// Create a new metric data entry with current timestamp
    pub fn new(task_name: String, task_type: crate::config::TaskType, data: RawMetricData) -> Self {
        Self {
            task_name,
            task_type,
            timestamp: current_timestamp(),
            data,
        }
    }

    /// Check if this metric represents a successful measurement
    pub fn is_successful(&self) -> bool {
        match &self.data {
            RawMetricData::Ping(metric) => metric.success,
            RawMetricData::Tcp(metric) => metric.success,
            RawMetricData::HttpGet(metric) => metric.success,
            RawMetricData::TlsHandshake(metric) => metric.success,
            RawMetricData::HttpContent(metric) => metric.success,
            RawMetricData::DnsQuery(metric) => metric.success,
            RawMetricData::Bandwidth(metric) => metric.success,
            #[cfg(feature = "sql-tasks")]
            RawMetricData::SqlQuery(metric) => metric.success,
        }
    }
}

impl AggregatedMetrics {
    /// Create a new aggregated metrics entry
    pub fn new(
        task_name: String,
        task_type: crate::config::TaskType,
        period_start: u64,
        period_end: u64,
        sample_count: u32,
        data: AggregatedMetricData,
    ) -> Self {
        Self {
            task_name,
            task_type,
            period_start,
            period_end,
            sample_count,
            data,
        }
    }

    /// Get the duration of the aggregation period in seconds
    pub fn period_duration_seconds(&self) -> u64 {
        self.period_end - self.period_start
    }
}

/// Get current Unix timestamp in seconds
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Calculate percentage from part and total
///
/// Returns 0.0 if total is 0 to avoid division by zero.
pub fn calculate_percentage(part: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        (part as f64 / total as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metric_data_creation() {
        let ping_data = RawMetricData::Ping(RawPingMetric {
            rtt_ms: Some(15.5),
            success: true,
            error: None,
            ip_address: "8.8.8.8".to_string(),
            domain: None,
            target_id: None,
        });

        let metric = MetricData::new(
            "Test Ping".to_string(),
            crate::config::TaskType::Ping,
            ping_data,
        );

        assert_eq!(metric.task_name, "Test Ping");
        assert_eq!(metric.task_type, crate::config::TaskType::Ping);
        assert!(metric.is_successful());
    }

    #[test]
    fn test_ping_metric_success_detection() {
        let successful_ping = RawPingMetric {
            rtt_ms: Some(10.0),
            success: true,
            error: None,
            ip_address: "8.8.8.8".to_string(),
            domain: None,
            target_id: None,
        };

        let failed_ping = RawPingMetric {
            rtt_ms: None,
            success: false,
            error: Some("Timeout".to_string()),
            ip_address: "8.8.8.8".to_string(),
            domain: None,
            target_id: None,
        };

        let successful_metric = MetricData::new(
            "Test".to_string(),
            crate::config::TaskType::Ping,
            RawMetricData::Ping(successful_ping),
        );

        let failed_metric = MetricData::new(
            "Test".to_string(),
            crate::config::TaskType::Ping,
            RawMetricData::Ping(failed_ping),
        );

        assert!(successful_metric.is_successful());
        assert!(!failed_metric.is_successful());
    }

    #[test]
    fn test_percentage_calculation() {
        assert_eq!(calculate_percentage(50, 100), 50.0);
        assert_eq!(calculate_percentage(0, 100), 0.0);
        assert_eq!(calculate_percentage(100, 100), 100.0);
        assert_eq!(calculate_percentage(1, 0), 0.0); // Edge case: division by zero
    }

    #[test]
    fn test_serialization() {
        let ping_metric = AggregatedPingMetric {
            avg_latency_ms: 15.5,
            max_latency_ms: 25.1,
            min_latency_ms: 10.2,
            packet_loss_percent: 5.0,
            successful_pings: 57,
            failed_pings: 3,
            domain: Some("example.com".to_string()),
            target_id: None,
        };

        let aggregated = AggregatedMetrics::new(
            "Test Ping".to_string(),
            crate::config::TaskType::Ping,
            1640995200, // 2022-01-01 00:00:00 UTC
            1640995260, // 2022-01-01 00:01:00 UTC
            60,
            AggregatedMetricData::Ping(ping_metric),
        );

        let json = serde_json::to_string(&aggregated).unwrap();
        let deserialized: AggregatedMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(aggregated, deserialized);
    }
}
