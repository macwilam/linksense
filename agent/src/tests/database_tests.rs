//! Tests for database management implementation

use crate::database::AgentDatabase;
use rusqlite::params;
use shared::config::TaskType;
use shared::metrics::{AggregatedMetricData, MetricData, RawMetricData, RawPingMetric};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

/// Helper function to get the current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tokio::test]
async fn test_database_creation() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();

    let result = db.initialize().await;
    assert!(result.is_ok());

    // Verify that the database file was actually created on disk.
    assert!(temp_dir.path().join("agent_metrics.db").exists());
}

#[tokio::test]
async fn test_store_raw_ping_metric() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    let metric = MetricData::new(
        "test_ping".to_string(),
        TaskType::Ping,
        RawMetricData::Ping(RawPingMetric {
            rtt_ms: Some(15.5),
            success: true,
            error: None,
            ip_address: "8.8.8.8".to_string(),
            domain: None,
            target_id: None,
        }),
    );

    let result = db.store_raw_metric(&metric).await;
    assert!(result.is_ok());
    // The returned row ID should be greater than 0.
    assert!(result.unwrap() > 0);
}

#[tokio::test]
async fn test_generate_ping_aggregated_metrics() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    // Store some test ping metrics
    let metrics = vec![
        MetricData::new(
            "test_ping".to_string(),
            TaskType::Ping,
            RawMetricData::Ping(RawPingMetric {
                rtt_ms: Some(10.0),
                success: true,
                error: None,
                ip_address: "8.8.8.8".to_string(),
                domain: Some("google.com".to_string()),
                target_id: None,
            }),
        ),
        MetricData::new(
            "test_ping".to_string(),
            TaskType::Ping,
            RawMetricData::Ping(RawPingMetric {
                rtt_ms: Some(20.0),
                success: true,
                error: None,
                ip_address: "8.8.8.8".to_string(),
                domain: Some("google.com".to_string()),
                target_id: None,
            }),
        ),
        MetricData::new(
            "test_ping".to_string(),
            TaskType::Ping,
            RawMetricData::Ping(RawPingMetric {
                rtt_ms: None,
                success: false,
                error: Some("Timeout".to_string()),
                ip_address: "8.8.8.8".to_string(),
                domain: Some("google.com".to_string()),
                target_id: None,
            }),
        ),
    ];

    for metric in metrics {
        db.store_raw_metric(&metric).await.unwrap();
    }

    // Generate aggregated metrics
    let now = current_timestamp();
    let period_start = now - 60; // 1 minute ago to ensure we include the stored metrics
    let period_end = now + 60; // 1 minute in the future

    let aggregated = db
        .generate_aggregated_metrics("test_ping", &TaskType::Ping, period_start, period_end)
        .await
        .unwrap();

    assert!(aggregated.is_some());
    let agg = aggregated.unwrap();
    assert_eq!(agg.task_name, "test_ping");
    assert_eq!(agg.sample_count, 3);

    if let AggregatedMetricData::Ping(ping_data) = agg.data {
        assert_eq!(ping_data.successful_pings, 2);
        assert_eq!(ping_data.failed_pings, 1);
        assert_eq!(ping_data.avg_latency_ms, 15.0); // (10 + 20) / 2
        assert!((ping_data.packet_loss_percent - 33.333333333333336).abs() < 0.0001);
    // 1 failed out of 3, with floating point tolerance
    } else {
        panic!("Expected ping aggregated data");
    }
}

#[tokio::test]
async fn test_store_raw_http_metric() {
    use shared::metrics::RawHttpMetric;

    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    let metric = MetricData::new(
        "test_http".to_string(),
        TaskType::HttpGet,
        RawMetricData::HttpGet(RawHttpMetric {
            status_code: Some(200),
            tcp_timing_ms: Some(15.2),
            tls_timing_ms: Some(20.3),
            ttfb_timing_ms: Some(50.1),
            content_download_timing_ms: Some(100.5),
            total_time_ms: Some(186.1),
            success: true,
            error: None,
            ssl_valid: Some(true),
            ssl_cert_days_until_expiry: Some(90),
            target_id: None,
        }),
    );

    let result = db.store_raw_metric(&metric).await;
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[tokio::test]
async fn test_store_raw_http_content_metric() {
    use shared::metrics::RawHttpContentMetric;

    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    let metric = MetricData::new(
        "test_http_content".to_string(),
        TaskType::HttpContent,
        RawMetricData::HttpContent(RawHttpContentMetric {
            status_code: Some(200),
            total_time_ms: Some(150.5),
            total_size: Some(1024),
            regexp_match: Some(true),
            success: true,
            error: None,
            target_id: None,
        }),
    );

    let result = db.store_raw_metric(&metric).await;
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[tokio::test]
async fn test_store_raw_dns_metric() {
    use shared::metrics::RawDnsMetric;

    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    let resolved_addresses = vec!["142.250.185.46".to_string()];

    let metric = MetricData::new(
        "test_dns".to_string(),
        TaskType::DnsQuery,
        RawMetricData::DnsQuery(RawDnsMetric {
            query_time_ms: Some(25.3),
            resolved_addresses: Some(resolved_addresses.clone()),
            record_count: Some(1),
            domain_queried: "google.com".to_string(),
            success: true,
            error: None,
            expected_ip: None,
            resolved_ip: Some("142.250.185.46".to_string()),
            correct_resolution: true,
            target_id: None,
        }),
    );

    let result = db.store_raw_metric(&metric).await;
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[tokio::test]
async fn test_store_raw_bandwidth_metric() {
    use shared::metrics::RawBandwidthMetric;

    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    let metric = MetricData::new(
        "test_bandwidth".to_string(),
        TaskType::Bandwidth,
        RawMetricData::Bandwidth(RawBandwidthMetric {
            bandwidth_mbps: Some(100.5),
            duration_ms: Some(5000.0),
            bytes_downloaded: Some(62_500_000),
            success: true,
            error: None,
            target_id: None,
        }),
    );

    let result = db.store_raw_metric(&metric).await;
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[tokio::test]
#[cfg(feature = "sql-tasks")]
async fn test_store_raw_sql_query_metric() {
    use shared::config::SqlQueryMode;
    use shared::metrics::RawSqlQueryMetric;

    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    let metric = MetricData::new(
        "test_sql".to_string(),
        TaskType::SqlQuery,
        RawMetricData::SqlQuery(RawSqlQueryMetric {
            total_time_ms: Some(125.7),
            row_count: Some(42),
            success: true,
            error: None,
            target_id: None,
            mode: SqlQueryMode::Value,
            value: Some(42.0),
            json_result: None,
            json_truncated: false,
            column_count: Some(1),
        }),
    );

    let result = db.store_raw_metric(&metric).await;
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[tokio::test]
async fn test_generate_http_aggregated_metrics() {
    use shared::metrics::RawHttpMetric;

    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    // Store some test HTTP metrics
    for i in 0..3 {
        let metric = MetricData::new(
            "test_http".to_string(),
            TaskType::HttpGet,
            RawMetricData::HttpGet(RawHttpMetric {
                status_code: Some(200),
                tcp_timing_ms: Some(15.0 + i as f64),
                tls_timing_ms: Some(20.0),
                ttfb_timing_ms: Some(50.0),
                content_download_timing_ms: Some(100.0),
                total_time_ms: Some(185.0 + i as f64),
                success: true,
                error: None,
                ssl_valid: Some(true),
                ssl_cert_days_until_expiry: Some(90),
                target_id: None,
            }),
        );
        db.store_raw_metric(&metric).await.unwrap();
    }

    let now = current_timestamp();
    let aggregated = db
        .generate_aggregated_metrics("test_http", &TaskType::HttpGet, now - 60, now + 60)
        .await
        .unwrap();

    assert!(aggregated.is_some());
    let agg = aggregated.unwrap();
    assert_eq!(agg.sample_count, 3);

    if let AggregatedMetricData::HttpGet(http_data) = agg.data {
        assert_eq!(http_data.successful_requests, 3);
        assert_eq!(http_data.failed_requests, 0);
        assert_eq!(http_data.success_rate_percent, 100.0);
        // Average of 185.0, 186.0, 187.0 = 186.0
        assert!((http_data.avg_total_time_ms - 186.0).abs() < 0.1);
    } else {
        panic!("Expected HTTP aggregated data");
    }
}

#[tokio::test]
async fn test_cleanup_old_data() {
    let temp_dir = TempDir::new().unwrap();
    let mut db = AgentDatabase::new(temp_dir.path(), 5).unwrap();
    db.initialize().await.unwrap();

    // Calculate timestamps: one old (35 days ago) and one recent (1 day ago)
    let now = current_timestamp();
    let old_timestamp = now - (35 * 24 * 60 * 60); // 35 days ago
    let recent_timestamp = now - (24 * 60 * 60); // 1 day ago

    // Insert test data and verify before cleanup
    {
        let conn = db.get_connection().unwrap();
        conn.execute(
            r#"
            INSERT INTO raw_metric_ping (task_name, timestamp, rtt_ms, success, error, ip_address, domain, target_id)
            VALUES ('old_ping', ?1, 15.5, 1, NULL, '8.8.8.8', NULL, NULL)
            "#,
            params![old_timestamp as i64],
        ).unwrap();

        // Insert recent metric directly via SQL (1 day old)
        conn.execute(
            r#"
            INSERT INTO raw_metric_ping (task_name, timestamp, rtt_ms, success, error, ip_address, domain, target_id)
            VALUES ('recent_ping', ?1, 20.0, 1, NULL, '1.1.1.1', NULL, NULL)
            "#,
            params![recent_timestamp as i64],
        ).unwrap();

        // Insert old aggregated metric directly via SQL
        let period_start = old_timestamp - 60;
        let period_end = old_timestamp;
        conn.execute(
            r#"
            INSERT INTO agg_metric_ping (task_name, period_start, period_end, sample_count, avg_latency_ms, max_latency_ms, min_latency_ms, packet_loss_percent, successful_pings, failed_pings, domain, target_id)
            VALUES ('old_ping', ?1, ?2, 1, 15.5, 15.5, 15.5, 0.0, 1, 0, NULL, NULL)
            "#,
            params![period_start as i64, period_end as i64],
        ).unwrap();

        // Verify old entries exist via SQL
        let old_raw_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_metric_ping WHERE timestamp < ?1",
                params![(now - 30 * 24 * 60 * 60) as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            old_raw_count, 1,
            "Should have 1 old raw metric before cleanup"
        );

        let old_agg_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agg_metric_ping WHERE period_end < ?1",
                params![(now - 30 * 24 * 60 * 60) as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            old_agg_count, 1,
            "Should have 1 old aggregated metric before cleanup"
        );

        let total_raw_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_metric_ping", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            total_raw_count, 2,
            "Should have 2 total raw metrics before cleanup"
        );
    } // Drop connection before cleanup

    // Run cleanup with 30 day retention
    db.cleanup_old_data(30).await.unwrap();

    // Verify old entries were deleted via SQL
    {
        let conn = db.get_connection().unwrap();
        let old_raw_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_metric_ping WHERE timestamp < ?1",
                params![(now - 30 * 24 * 60 * 60) as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            old_raw_count_after, 0,
            "Should have 0 old raw metrics after cleanup"
        );

        let old_agg_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agg_metric_ping WHERE period_end < ?1",
                params![(now - 30 * 24 * 60 * 60) as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            old_agg_count_after, 0,
            "Should have 0 old aggregated metrics after cleanup"
        );

        // Verify recent metric still exists
        let recent_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_metric_ping WHERE timestamp >= ?1",
                params![(now - 30 * 24 * 60 * 60) as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            recent_count, 1,
            "Should still have 1 recent metric after cleanup"
        );
    } // Drop connection
}
