//! Default values for configuration parameters
//!
//! This module centralizes all default value functions used by the configuration
//! structures. These functions are used by serde when deserializing configuration
//! files that don't specify certain optional fields.

// Task-specific defaults

/// Default ping task timeout (1 second)
pub fn default_ping_timeout() -> u32 {
    1
}

/// Default HTTP task timeout (10 seconds)
pub fn default_http_timeout() -> u32 {
    10
}

/// Default DNS query timeout (5 seconds)
pub fn default_dns_timeout() -> u32 {
    5
}

/// Default bandwidth test timeout (60 seconds)
pub fn default_bandwidth_timeout() -> u32 {
    60
}

/// Default maximum retry attempts for bandwidth test queuing (10 attempts)
pub fn default_bandwidth_max_retries() -> u32 {
    10
}

/// Default SQL query timeout (30 seconds)
#[cfg(feature = "sql-tasks")]
pub fn default_sql_timeout() -> u32 {
    30
}

/// Default SNMP query timeout (5 seconds)
pub fn default_snmp_timeout() -> u32 {
    5
}

/// Default SNMP community string
pub fn default_snmp_community() -> String {
    "public".to_string()
}

/// Default maximum JSON result size for SQL queries (64 KB)
#[cfg(feature = "sql-tasks")]
pub fn default_sql_json_max_size() -> usize {
    65536
}

/// Default maximum rows for SQL JSON mode (1000 rows)
#[cfg(feature = "sql-tasks")]
pub fn default_sql_max_rows() -> usize {
    1000
}

// Server configuration defaults

/// Default bandwidth test size (10 MB)
pub fn default_bandwidth_size() -> u32 {
    10
}

/// Default agent configuration directory path
pub fn default_config_dir() -> String {
    "/etc/monitoring-server/agent-configs".to_string()
}

/// Default reconfiguration check interval (10 seconds)
pub fn default_reconfigure_interval() -> u32 {
    10
}

/// Default data cleanup interval (24 hours)
pub fn default_cleanup_interval() -> u32 {
    24
}

/// Default rate limiting enabled flag
pub fn default_rate_limit_enabled() -> bool {
    true
}

/// Default rate limit window (60 seconds)
pub fn default_rate_limit_window() -> u32 {
    60
}

/// Default maximum requests per rate limit window
pub fn default_rate_limit_max_requests() -> usize {
    100
}

/// Default bandwidth test timeout on server (120 seconds / 2 minutes)
pub fn default_bandwidth_test_timeout() -> u64 {
    120
}

/// Default base delay before bandwidth test can start (30 seconds)
pub fn default_bandwidth_queue_base_delay() -> u64 {
    30
}

/// Default delay when another test is currently running (60 seconds)
pub fn default_bandwidth_queue_current_test_delay() -> u64 {
    60
}

/// Default multiplier for queue position delay (30 seconds per position)
pub fn default_bandwidth_queue_position_multiplier() -> u64 {
    30
}

/// Default maximum delay for bandwidth test queue (300 seconds / 5 minutes)
pub fn default_bandwidth_max_delay() -> u64 {
    300
}

/// Default initial cleanup delay on server startup (3600 seconds / 1 hour)
pub fn default_initial_cleanup_delay() -> u64 {
    3600
}

/// Default graceful shutdown timeout for server (30 seconds)
pub fn default_server_graceful_shutdown_timeout() -> u64 {
    30
}

// Agent configuration defaults

/// Default metrics flush interval (5 seconds)
pub fn default_metrics_flush_interval() -> u32 {
    5
}

/// Default metrics send interval to server (30 seconds)
pub fn default_metrics_send_interval() -> u32 {
    30
}

/// Default number of metrics per batch
pub fn default_metrics_batch_size() -> usize {
    50
}

/// Default maximum retry attempts for failed metric sends
pub fn default_metrics_max_retries() -> usize {
    10
}

/// Default queue cleanup interval (3600 seconds / 1 hour)
pub fn default_queue_cleanup_interval() -> u64 {
    3600
}

/// Default data cleanup interval (86400 seconds / 24 hours)
pub fn default_data_cleanup_interval() -> u64 {
    86400
}

/// Default maximum concurrent tasks
pub fn default_max_concurrent_tasks() -> usize {
    50
}

/// Default maximum HTTP response size (100 MB)
pub fn default_http_response_max_size_mb() -> usize {
    100
}

/// Default HTTP client timeout (30 seconds)
pub fn default_http_client_timeout() -> u64 {
    30
}

/// Default SQLite database busy timeout (5 seconds)
pub fn default_database_busy_timeout() -> u64 {
    5
}

/// Default graceful shutdown timeout for agent (30 seconds)
pub fn default_graceful_shutdown_timeout() -> u64 {
    30
}

/// Default channel buffer size for task communication
pub fn default_channel_buffer_size() -> usize {
    1000
}

/// Default WAL checkpoint interval (60 seconds / 1 minute)
pub fn default_wal_checkpoint_interval() -> u64 {
    60
}

/// Default HTTP client refresh interval (3600 seconds / 1 hour)
pub fn default_http_client_refresh_interval() -> u64 {
    3600
}

/// Default agent health monitoring enabled flag
pub fn default_monitor_agents_health() -> bool {
    false
}

/// Default health check interval (300 seconds / 5 minutes)
pub fn default_health_check_interval() -> u64 {
    300
}

/// Default health check success ratio threshold (0.9 = 90%)
pub fn default_health_check_threshold() -> f64 {
    0.9
}

/// Default health check data retention (30 days)
pub fn default_health_check_retention_days() -> u32 {
    30
}
