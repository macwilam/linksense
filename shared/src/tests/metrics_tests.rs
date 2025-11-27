//! Tests for metric data structures

use crate::config::TaskType;
use crate::metrics::{
    calculate_percentage, AggregatedHttpMetric, AggregatedMetricData, AggregatedMetrics,
    AggregatedPingMetric, MetricData, RawMetricData, RawPingMetric,
};
use std::collections::HashMap;

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

    let metric = MetricData::new("Test Ping".to_string(), TaskType::Ping, ping_data);

    assert_eq!(metric.task_name, "Test Ping");
    assert_eq!(metric.task_type, TaskType::Ping);
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
        TaskType::Ping,
        RawMetricData::Ping(successful_ping),
    );

    let failed_metric = MetricData::new(
        "Test".to_string(),
        TaskType::Ping,
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
        TaskType::Ping,
        1640995200, // 2022-01-01 00:00:00 UTC
        1640995260, // 2022-01-01 00:01:00 UTC
        60,
        AggregatedMetricData::Ping(ping_metric),
    );

    let json = serde_json::to_string(&aggregated).unwrap();
    let deserialized: AggregatedMetrics = serde_json::from_str(&json).unwrap();
    assert_eq!(aggregated, deserialized);
}

#[test]
fn test_http_status_code_distribution_serialization() {
    // Create HTTP metric with status code distribution
    let mut status_codes = HashMap::new();
    status_codes.insert(200, 10);
    status_codes.insert(404, 2);
    status_codes.insert(500, 1);

    let http_metric = AggregatedHttpMetric {
        success_rate_percent: 76.9,
        avg_tcp_timing_ms: 15.5,
        avg_tls_timing_ms: 45.2,
        avg_ttfb_timing_ms: 120.3,
        avg_content_download_timing_ms: 50.1,
        avg_total_time_ms: 231.1,
        max_total_time_ms: 450.0,
        successful_requests: 10,
        failed_requests: 3,
        status_code_distribution: status_codes.clone(),
        ssl_valid_percent: Some(100.0),
        avg_ssl_cert_days_until_expiry: Some(90.0),
        target_id: None,
    };

    let aggregated = AggregatedMetrics::new(
        "Test HTTP".to_string(),
        TaskType::HttpGet,
        1640995200,
        1640995260,
        13,
        AggregatedMetricData::HttpGet(http_metric),
    );

    // Serialize to JSON
    let json = serde_json::to_string(&aggregated).unwrap();

    // Verify JSON format - should be array of tuples, not object with string keys
    assert!(json.contains("[[200,10]") || json.contains("[[404,2]") || json.contains("[[500,1]"));
    assert!(!json.contains("\"200\":"));
    assert!(!json.contains("\"404\":"));
    assert!(!json.contains("\"500\":"));

    // Deserialize back
    let deserialized: AggregatedMetrics = serde_json::from_str(&json).unwrap();

    // Verify equality
    assert_eq!(aggregated, deserialized);

    // Verify status code distribution is intact
    if let AggregatedMetricData::HttpGet(http_data) = deserialized.data {
        assert_eq!(http_data.status_code_distribution.get(&200), Some(&10));
        assert_eq!(http_data.status_code_distribution.get(&404), Some(&2));
        assert_eq!(http_data.status_code_distribution.get(&500), Some(&1));
        assert_eq!(http_data.status_code_distribution.len(), 3);
    } else {
        panic!("Expected HttpGet metric data");
    }
}
