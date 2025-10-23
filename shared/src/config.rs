//! Configuration types and validation for the network monitoring system
//!
//! This module defines the configuration structures used by both agent and server
//! components, including validation logic and serialization support.

use crate::defaults::*;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

/// Main agent configuration loaded from agent.toml
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    /// Unique identifier for this agent
    pub agent_id: String,
    /// Base URL of the central server API
    #[serde(default)]
    pub central_server_url: String,
    /// Pre-shared secret key for authentication
    #[serde(default)]
    pub api_key: String,
    /// Number of days to retain local data before purging
    pub local_data_retention_days: u32,
    /// Whether to automatically update tasks configuration from server
    #[serde(default)]
    pub auto_update_tasks: bool,
    /// Whether to run in local-only mode (no server communication)
    #[serde(default)]
    pub local_only: bool,
    /// Interval in seconds between database flushes for buffered metrics (default: 5, min: 1, max: 60)
    #[serde(default = "default_metrics_flush_interval")]
    pub metrics_flush_interval_seconds: u32,

    // Metrics submission settings
    /// How often to push metrics to server in seconds (default: 30)
    #[serde(default = "default_metrics_send_interval")]
    pub metrics_send_interval_seconds: u32,
    /// Number of metrics to send per batch (default: 50)
    #[serde(default = "default_metrics_batch_size")]
    pub metrics_batch_size: usize,
    /// Maximum retry attempts for failed metric sends (default: 10)
    #[serde(default = "default_metrics_max_retries")]
    pub metrics_max_retries: usize,
    /// Cleanup interval for sent metrics queue in seconds (default: 3600)
    #[serde(default = "default_queue_cleanup_interval")]
    pub queue_cleanup_interval_seconds: u64,

    // Data management
    /// Daily cleanup of old data in seconds (default: 86400)
    #[serde(default = "default_data_cleanup_interval")]
    pub data_cleanup_interval_seconds: u64,

    // Performance tuning
    /// Maximum number of concurrent tasks (default: 50)
    #[serde(default = "default_max_concurrent_tasks")]
    pub max_concurrent_tasks: usize,
    /// Maximum HTTP response body size in MB (default: 100)
    #[serde(default = "default_http_response_max_size_mb")]
    pub http_response_max_size_mb: usize,
    /// HTTP client timeout for server communication in seconds (default: 30)
    #[serde(default = "default_http_client_timeout")]
    pub http_client_timeout_seconds: u64,
    /// SQLite database busy timeout in seconds (default: 5)
    #[serde(default = "default_database_busy_timeout")]
    pub database_busy_timeout_seconds: u64,

    // Shutdown behavior
    /// Wait time for in-flight tasks during shutdown in seconds (default: 30)
    #[serde(default = "default_graceful_shutdown_timeout")]
    pub graceful_shutdown_timeout_seconds: u64,
    /// Result channel capacity for task results (default: 1000)
    #[serde(default = "default_channel_buffer_size")]
    pub channel_buffer_size: usize,
}

/// Task configuration loaded from tasks.toml
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TasksConfig {
    /// Array of monitoring tasks to execute
    pub tasks: Vec<TaskConfig>,
}

/// Individual task configuration
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TaskConfig {
    /// Type of monitoring task
    #[serde(rename = "type")]
    pub task_type: TaskType,
    /// How often to run this task (in seconds)
    pub schedule_seconds: u32,
    /// Human-readable name for this task
    pub name: String,
    /// Optional timeout in seconds (overrides task-specific defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// Task-specific parameters
    #[serde(flatten)]
    pub params: TaskParams,
}

// Custom deserializer implementation for TaskConfig that uses the 'type' field
// to determine which params variant to deserialize, rather than relying on
// untagged enum field matching.
impl<'de> Deserialize<'de> for TaskConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, MapAccess, Visitor};
        use std::fmt;

        struct TaskConfigVisitor;

        impl<'de> Visitor<'de> for TaskConfigVisitor {
            type Value = TaskConfig;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a task configuration object")
            }

            fn visit_map<V>(self, mut map: V) -> Result<TaskConfig, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut task_type: Option<TaskType> = None;
                let mut schedule_seconds: Option<u32> = None;
                let mut name: Option<String> = None;
                let mut timeout: Option<u32> = None;
                let mut params_map = toml::map::Map::new();

                // Read all fields from the map
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => {
                            if task_type.is_some() {
                                return Err(Error::duplicate_field("type"));
                            }
                            task_type = Some(map.next_value()?);
                        }
                        "schedule_seconds" => {
                            if schedule_seconds.is_some() {
                                return Err(Error::duplicate_field("schedule_seconds"));
                            }
                            schedule_seconds = Some(map.next_value()?);
                        }
                        "name" => {
                            if name.is_some() {
                                return Err(Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        "timeout" => {
                            if timeout.is_some() {
                                return Err(Error::duplicate_field("timeout"));
                            }
                            timeout = Some(map.next_value()?);
                        }
                        _ => {
                            // Collect all other fields for params deserialization
                            let value: toml::Value = map.next_value()?;
                            params_map.insert(key, value);
                        }
                    }
                }

                // Validate required fields
                let task_type = task_type.ok_or_else(|| Error::missing_field("type"))?;
                let schedule_seconds =
                    schedule_seconds.ok_or_else(|| Error::missing_field("schedule_seconds"))?;
                let name = name.ok_or_else(|| Error::missing_field("name"))?;

                // Deserialize params based on task_type (NOT based on which fields are present)
                let params_value = toml::Value::Table(params_map);
                let params = match task_type {
                    TaskType::Ping => {
                        let params: PingParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!("Failed to parse Ping task parameters: {}", e))
                        })?;
                        TaskParams::Ping(params)
                    }
                    TaskType::Tcp => {
                        let params: TcpParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!("Failed to parse Tcp task parameters: {}", e))
                        })?;
                        TaskParams::Tcp(params)
                    }
                    TaskType::HttpGet => {
                        let params: HttpGetParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!("Failed to parse HttpGet task parameters: {}", e))
                        })?;
                        TaskParams::HttpGet(params)
                    }
                    TaskType::HttpContent => {
                        let params: HttpContentParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!(
                                "Failed to parse HttpContent task parameters: {}",
                                e
                            ))
                        })?;
                        TaskParams::HttpContent(params)
                    }
                    TaskType::TlsHandshake => {
                        let params: TlsHandshakeParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!(
                                "Failed to parse TlsHandshake task parameters: {}",
                                e
                            ))
                        })?;
                        TaskParams::TlsHandshake(params)
                    }
                    TaskType::DnsQuery => {
                        let params: DnsQueryParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!(
                                "Failed to parse DnsQuery task parameters: {}",
                                e
                            ))
                        })?;
                        TaskParams::DnsQuery(params)
                    }
                    TaskType::DnsQueryDoh => {
                        let params: DnsQueryDohParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!(
                                "Failed to parse DnsQueryDoh task parameters: {}",
                                e
                            ))
                        })?;
                        TaskParams::DnsQueryDoh(params)
                    }
                    TaskType::Bandwidth => {
                        let params: BandwidthParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!(
                                "Failed to parse Bandwidth task parameters: {}",
                                e
                            ))
                        })?;
                        TaskParams::Bandwidth(params)
                    }
                    #[cfg(feature = "sql-tasks")]
                    TaskType::SqlQuery => {
                        let params: SqlQueryParams = params_value.try_into().map_err(|e| {
                            Error::custom(format!(
                                "Failed to parse SqlQuery task parameters: {}",
                                e
                            ))
                        })?;
                        TaskParams::SqlQuery(params)
                    }
                };

                Ok(TaskConfig {
                    task_type,
                    schedule_seconds,
                    name,
                    timeout,
                    params,
                })
            }
        }

        deserializer.deserialize_map(TaskConfigVisitor)
    }
}

/// Different types of monitoring tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// ICMP ping test
    Ping,
    /// TCP connection test
    Tcp,
    /// HTTP GET request test
    HttpGet,
    /// HTTP content check with regex matching
    HttpContent,
    /// TLS handshake test
    TlsHandshake,
    /// DNS query test
    DnsQuery,
    /// DNS over HTTPS query test
    DnsQueryDoh,
    /// Bandwidth measurement test
    Bandwidth,
    /// SQL query test (requires sql-tasks feature)
    #[cfg(feature = "sql-tasks")]
    SqlQuery,
}

/// Task-specific parameters
///
/// IMPORTANT: This enum uses #[serde(untagged)] which means serde will try
/// each variant in order until one successfully deserializes. Variants MUST
/// be ordered from most specific to least specific based on required fields:
/// - HttpContent (requires url + regexp) before HttpGet (requires only url)
/// - More specific variants should always come before less specific ones
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TaskParams {
    // Most specific first - has both url AND regexp required
    HttpContent(HttpContentParams),

    // Less specific - only url required
    HttpGet(HttpGetParams),

    // Others with unique required fields
    Ping(PingParams),
    Tcp(TcpParams),
    TlsHandshake(TlsHandshakeParams),
    DnsQuery(DnsQueryParams),
    DnsQueryDoh(DnsQueryDohParams),
    Bandwidth(BandwidthParams),
    #[cfg(feature = "sql-tasks")]
    SqlQuery(SqlQueryParams),
}

/// Parameters for TLS handshake tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TlsHandshakeParams {
    /// Target host:port to connect (e.g., "example.com:443")
    pub host: String,
    /// Whether to verify SSL certificates (default: false)
    #[serde(default)]
    pub verify_ssl: bool,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for ICMP ping tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PingParams {
    /// Target host to ping
    pub host: String,
    /// Optional timeout in seconds (default: 5)
    #[serde(default = "default_ping_timeout")]
    pub timeout_seconds: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for TCP connection tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TcpParams {
    /// Target host:port to connect (e.g., "example.com:80" or "192.168.1.1:22")
    pub host: String,
    /// Optional timeout in seconds (default: 5)
    #[serde(default = "default_ping_timeout")]
    pub timeout_seconds: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for HTTP GET tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpGetParams {
    /// Target URL to request
    pub url: String,
    /// Optional timeout in seconds (default: 30)
    #[serde(default = "default_http_timeout")]
    pub timeout_seconds: u32,
    /// Optional custom headers
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Whether to verify SSL certificates (default: false)
    #[serde(default)]
    pub verify_ssl: bool,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for HTTP content check tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpContentParams {
    /// Target URL to request
    pub url: String,
    /// Regular expression pattern to match in response body
    pub regexp: String,
    /// Optional timeout in seconds (default: 30)
    #[serde(default = "default_http_timeout")]
    pub timeout_seconds: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for DNS query tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DnsQueryParams {
    /// DNS server to query
    pub server: String,
    /// Domain name to resolve
    pub domain: String,
    /// DNS record type to query
    pub record_type: DnsRecordType,
    /// Optional timeout in seconds (default: 10)
    #[serde(default = "default_dns_timeout")]
    pub timeout_seconds: u32,
    /// Optional expected IP address to validate resolution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_ip: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for DNS over HTTPS query tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DnsQueryDohParams {
    /// DNS over HTTPS server URL (e.g., "https://cloudflare-dns.com/dns-query")
    pub server_url: String,
    /// Domain name to resolve
    pub domain: String,
    /// DNS record type to query
    pub record_type: DnsRecordType,
    /// Optional timeout in seconds (default: 10)
    #[serde(default = "default_dns_timeout")]
    pub timeout_seconds: u32,
    /// Optional expected IP address to validate resolution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_ip: Option<String>,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for bandwidth test tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BandwidthParams {
    /// Optional timeout in seconds (default: 60)
    #[serde(default = "default_bandwidth_timeout")]
    pub timeout_seconds: u32,
    /// Maximum retry attempts when server requests delay (default: 10)
    #[serde(default = "default_bandwidth_max_retries")]
    pub max_retries: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// Parameters for SQL query tasks
#[cfg(feature = "sql-tasks")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqlQueryParams {
    /// SQL query to execute
    pub query: String,
    /// Database connection URL (e.g., "postgresql://user:pass@localhost/db")
    pub database_url: String,
    /// Database type (e.g., "postgres", "mysql", "sqlite")
    pub database_type: String,
    /// Optional username for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Optional password for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Optional timeout in seconds (default: 30)
    #[serde(default = "default_sql_timeout")]
    pub timeout_seconds: u32,
    /// Optional target identifier for grouping/filtering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// DNS record types supported for queries
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum DnsRecordType {
    A,
    AAAA,
    MX,
    CNAME,
    TXT,
    NS,
}

/// Server configuration loaded from server.toml
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerConfig {
    /// Address and port to bind the API server to
    pub listen_address: String,
    /// Pre-shared secret key for agent authentication
    pub api_key: String,
    /// Number of days to retain metric data before purging
    pub data_retention_days: u32,
    /// Optional configuration directory path
    #[serde(default = "default_config_dir")]
    pub agent_configs_dir: String,
    /// Optional bandwidth test file size in MB
    #[serde(default = "default_bandwidth_size")]
    pub bandwidth_test_size_mb: u32,
    /// Interval in seconds to check for reconfiguration requests (default: 10, min: 1, max: 300)
    #[serde(default = "default_reconfigure_interval")]
    pub reconfigure_check_interval_seconds: u32,
    /// Optional whitelist of allowed agent IDs (empty = all agents allowed)
    #[serde(default)]
    pub agent_id_whitelist: Vec<String>,
    /// Interval in hours between database cleanup runs (default: 24)
    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval_hours: u32,
    /// Enable rate limiting (default: true)
    #[serde(default = "default_rate_limit_enabled")]
    pub rate_limit_enabled: bool,
    /// Rate limit time window in seconds (default: 60)
    #[serde(default = "default_rate_limit_window")]
    pub rate_limit_window_seconds: u32,
    /// Maximum requests per agent within rate limit window (default: 100)
    #[serde(default = "default_rate_limit_max_requests")]
    pub rate_limit_max_requests: usize,

    // Bandwidth test management
    /// Bandwidth test timeout in seconds (default: 120)
    #[serde(default = "default_bandwidth_test_timeout")]
    pub bandwidth_test_timeout_seconds: u64,
    /// Base delay for bandwidth queue when no test running in seconds (default: 30)
    #[serde(default = "default_bandwidth_queue_base_delay")]
    pub bandwidth_queue_base_delay_seconds: u64,
    /// Delay for current bandwidth test in seconds (default: 60)
    #[serde(default = "default_bandwidth_queue_current_test_delay")]
    pub bandwidth_queue_current_test_delay_seconds: u64,
    /// Delay multiplier per queue position in seconds (default: 30)
    #[serde(default = "default_bandwidth_queue_position_multiplier")]
    pub bandwidth_queue_position_multiplier_seconds: u64,
    /// Maximum delay suggestion for bandwidth queue in seconds (default: 300)
    #[serde(default = "default_bandwidth_max_delay")]
    pub bandwidth_max_delay_seconds: u64,

    // Cleanup and maintenance
    /// Initial delay before first cleanup in seconds (default: 3600)
    #[serde(default = "default_initial_cleanup_delay")]
    pub initial_cleanup_delay_seconds: u64,
    /// Graceful shutdown timeout in seconds (default: 30)
    #[serde(default = "default_server_graceful_shutdown_timeout")]
    pub graceful_shutdown_timeout_seconds: u64,
    /// WAL checkpoint interval in seconds (default: 60)
    #[serde(default = "default_wal_checkpoint_interval")]
    pub wal_checkpoint_interval_seconds: u64,
}

impl AgentConfig {
    /// Validate the agent configuration
    pub fn validate(&self) -> crate::Result<()> {
        if self.agent_id.is_empty() {
            return Err(
                crate::MonitoringError::Validation("agent_id cannot be empty".to_string()).into(),
            );
        }

        if !self
            .agent_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(crate::MonitoringError::Validation(
                "agent_id can only contain alphanumeric characters, hyphens, and underscores"
                    .to_string(),
            )
            .into());
        }

        // Skip server-related validation when running in local-only mode
        if !self.local_only {
            if self.central_server_url.is_empty() {
                return Err(crate::MonitoringError::Validation(
                    "central_server_url cannot be empty (or set local_only=true)".to_string(),
                )
                .into());
            }

            if !self.central_server_url.starts_with("http://")
                && !self.central_server_url.starts_with("https://")
            {
                return Err(crate::MonitoringError::Validation(
                    "central_server_url must start with http:// or https://".to_string(),
                )
                .into());
            }

            if self.api_key.is_empty() {
                return Err(crate::MonitoringError::Validation(
                    "api_key cannot be empty (or set local_only=true)".to_string(),
                )
                .into());
            }
        }

        if self.local_data_retention_days == 0 {
            return Err(crate::MonitoringError::Validation(
                "local_data_retention_days must be greater than 0".to_string(),
            )
            .into());
        }

        if self.metrics_flush_interval_seconds == 0 {
            return Err(crate::MonitoringError::Validation(
                "metrics_flush_interval_seconds must be at least 1".to_string(),
            )
            .into());
        }

        if self.metrics_flush_interval_seconds > 60 {
            return Err(crate::MonitoringError::Validation(
                "metrics_flush_interval_seconds must not exceed 60 (use smaller intervals for better latency)"
                    .to_string(),
            )
            .into());
        }

        // Validate new configuration fields
        if self.metrics_send_interval_seconds == 0 {
            return Err(crate::MonitoringError::Validation(
                "metrics_send_interval_seconds must be at least 1".to_string(),
            )
            .into());
        }

        if self.metrics_batch_size == 0 {
            return Err(crate::MonitoringError::Validation(
                "metrics_batch_size must be at least 1".to_string(),
            )
            .into());
        }

        if self.max_concurrent_tasks == 0 {
            return Err(crate::MonitoringError::Validation(
                "max_concurrent_tasks must be at least 1".to_string(),
            )
            .into());
        }

        if self.http_response_max_size_mb == 0 {
            return Err(crate::MonitoringError::Validation(
                "http_response_max_size_mb must be at least 1".to_string(),
            )
            .into());
        }

        if self.channel_buffer_size == 0 {
            return Err(crate::MonitoringError::Validation(
                "channel_buffer_size must be at least 1".to_string(),
            )
            .into());
        }

        Ok(())
    }
}

impl TaskConfig {
    /// Validate the task configuration
    pub fn validate(&self) -> crate::Result<()> {
        if self.name.is_empty() {
            return Err(crate::MonitoringError::Validation(
                "Task name cannot be empty. Please provide a descriptive name for this task."
                    .to_string(),
            )
            .into());
        }

        if self.schedule_seconds == 0 {
            return Err(crate::MonitoringError::Validation(format!(
                "Invalid schedule_seconds: {}. Value must be greater than 0.",
                self.schedule_seconds
            ))
            .into());
        }

        // Bandwidth tasks have a minimum schedule of 60 seconds
        if self.task_type == TaskType::Bandwidth && self.schedule_seconds < 60 {
            return Err(crate::MonitoringError::Validation(
                format!("Invalid schedule_seconds for bandwidth task: {}. Bandwidth tasks must have schedule_seconds >= 60 to prevent server overload.", self.schedule_seconds)
            )
            .into());
        }

        // SQL tasks have a minimum schedule of 60 seconds
        #[cfg(feature = "sql-tasks")]
        if self.task_type == TaskType::SqlQuery && self.schedule_seconds < 60 {
            return Err(crate::MonitoringError::Validation(
                format!("Invalid schedule_seconds for SQL query task: {}. SQL tasks must have schedule_seconds >= 60.", self.schedule_seconds)
            )
            .into());
        }

        // Validate task-specific parameters
        match (&self.task_type, &self.params) {
            (TaskType::Ping, TaskParams::Ping(params)) => {
                if params.host.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "Ping task is missing required parameter 'host'. Please specify a hostname or IP address to ping.".to_string(),
                    )
                    .into());
                }
            }
            (TaskType::Tcp, TaskParams::Tcp(params)) => {
                if params.host.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "TCP task is missing required parameter 'host'. Please specify a host:port to connect (e.g., 'example.com:80' or '192.168.1.1:22').".to_string(),
                    )
                    .into());
                }
            }
            (TaskType::TlsHandshake, TaskParams::TlsHandshake(params)) => {
                if params.host.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "TLS Handshake task is missing required parameter 'host'. Please specify a host:port to connect (e.g., 'example.com:443').".to_string(),
                    )
                    .into());
                }
            }
            (TaskType::HttpGet, TaskParams::HttpGet(params)) => {
                if params.url.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "HTTP GET task is missing required parameter 'url'. Please specify the URL to request.".to_string(),
                    )
                    .into());
                }
                if !params.url.starts_with("http://") && !params.url.starts_with("https://") {
                    return Err(crate::MonitoringError::Validation(
                        format!("HTTP GET task has invalid URL '{}'. URL must start with http:// or https://", params.url)
                    )
                    .into());
                }
            }
            (TaskType::HttpContent, TaskParams::HttpContent(params)) => {
                if params.url.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "HTTP Content task is missing required parameter 'url'. Please specify the URL to request.".to_string(),
                    )
                    .into());
                }
                if !params.url.starts_with("http://") && !params.url.starts_with("https://") {
                    return Err(crate::MonitoringError::Validation(
                        format!("HTTP Content task has invalid URL '{}'. URL must start with http:// or https://", params.url)
                    )
                    .into());
                }
                if params.regexp.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "HTTP Content task is missing required parameter 'regexp'. Please specify a regular expression pattern to match in the response.".to_string(),
                    )
                    .into());
                }
                // Validate that the regex pattern is valid
                if let Err(e) = regex::Regex::new(&params.regexp) {
                    return Err(crate::MonitoringError::Validation(format!(
                        "HTTP Content task has invalid regexp pattern '{}'. Regex compilation error: {}",
                        params.regexp,
                        e
                    ))
                    .into());
                }
            }
            (TaskType::DnsQuery, TaskParams::DnsQuery(params)) => {
                if params.server.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "DNS Query task is missing required parameter 'server'. Please specify the DNS server address (e.g., '8.8.8.8:53' or '8.8.8.8').".to_string(),
                    )
                    .into());
                }
                if params.domain.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "DNS Query task is missing required parameter 'domain'. Please specify the domain name to resolve.".to_string(),
                    )
                    .into());
                }
            }
            (TaskType::DnsQueryDoh, TaskParams::DnsQueryDoh(params)) => {
                if params.server_url.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "DNS-over-HTTPS task is missing required parameter 'server_url'. Please specify the DoH server URL (e.g., 'https://cloudflare-dns.com/dns-query').".to_string(),
                    )
                    .into());
                }
                if !params.server_url.starts_with("https://") {
                    return Err(crate::MonitoringError::Validation(
                        format!("DNS-over-HTTPS task has invalid server_url '{}'. URL must start with https:// (DoH requires HTTPS)", params.server_url)
                    )
                    .into());
                }
                if params.domain.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "DNS-over-HTTPS task is missing required parameter 'domain'. Please specify the domain name to resolve.".to_string(),
                    )
                    .into());
                }
            }
            (TaskType::Bandwidth, TaskParams::Bandwidth(_)) => {
                // Bandwidth tasks don't have required parameters
            }
            #[cfg(feature = "sql-tasks")]
            (TaskType::SqlQuery, TaskParams::SqlQuery(params)) => {
                if params.query.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "SQL Query task is missing required parameter 'query'. Please specify the SQL query to execute.".to_string(),
                    )
                    .into());
                }
                if params.database_url.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "SQL Query task is missing required parameter 'database_url'. Please specify the database connection URL (e.g., 'postgresql://user:pass@localhost/db').".to_string(),
                    )
                    .into());
                }
                if params.database_type.is_empty() {
                    return Err(crate::MonitoringError::Validation(
                        "SQL Query task is missing required parameter 'database_type'. Please specify the database type (e.g., 'postgres', 'mysql', 'sqlite').".to_string(),
                    )
                    .into());
                }
            }
            _ => {
                return Err(crate::MonitoringError::Validation(
                    format!("Task type mismatch: The task is defined as type '{:?}' but the parameters provided do not match this type. Please ensure the task type and parameters are consistent.", self.task_type)
                )
                .into());
            }
        }

        Ok(())
    }

    /// Get the schedule duration for this task
    pub fn schedule_duration(&self) -> Duration {
        Duration::from_secs(self.schedule_seconds as u64)
    }

    /// Get the effective timeout for this task (uses task-level timeout if set, otherwise task-specific default)
    pub fn get_effective_timeout(&self) -> u32 {
        if let Some(timeout) = self.timeout {
            return timeout;
        }

        match &self.params {
            TaskParams::Ping(params) => params.timeout_seconds,
            TaskParams::Tcp(params) => params.timeout_seconds,
            TaskParams::HttpGet(params) => params.timeout_seconds,
            TaskParams::HttpContent(params) => params.timeout_seconds,
            TaskParams::TlsHandshake(_) => 10, // Default timeout for TLS handshake
            TaskParams::DnsQuery(params) => params.timeout_seconds,
            TaskParams::DnsQueryDoh(params) => params.timeout_seconds,
            TaskParams::Bandwidth(params) => params.timeout_seconds,
            #[cfg(feature = "sql-tasks")]
            TaskParams::SqlQuery(params) => params.timeout_seconds,
        }
    }
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl TasksConfig {
    /// Creates a new, empty `TasksConfig`.
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Validate all tasks in the configuration
    pub fn validate(&self) -> crate::Result<()> {
        if self.tasks.is_empty() {
            // It's valid to have no tasks. The agent can decide if this is an issue.
            return Ok(());
        }

        // Check for duplicate task names
        let mut names = std::collections::HashSet::new();
        for (index, task) in self.tasks.iter().enumerate() {
            if !names.insert(&task.name) {
                return Err(crate::MonitoringError::Validation(format!(
                    "Task #{} (name: '{}'): Duplicate task name found. Each task must have a unique name.",
                    index + 1,
                    task.name
                ))
                .into());
            }
        }

        // Validate each task with detailed error context
        for (index, task) in self.tasks.iter().enumerate() {
            if let Err(e) = task.validate() {
                // Extract the inner error message, stripping the "Validation error: " prefix if present
                let error_msg = e.to_string();
                let clean_msg = error_msg
                    .strip_prefix("Validation error: ")
                    .unwrap_or(&error_msg);

                return Err(crate::MonitoringError::Validation(format!(
                    "Task #{} (name: '{}'): {}",
                    index + 1,
                    task.name,
                    clean_msg
                ))
                .into());
            }
        }

        Ok(())
    }

    /// Validate tasks configuration from TOML string content
    /// This is used for validating configuration files before applying them
    pub fn validate_from_toml(toml_content: &str) -> crate::Result<TasksConfig> {
        // Parse the TOML content
        let tasks_config: TasksConfig = toml::from_str(toml_content).map_err(|e| {
            crate::MonitoringError::Validation(format!("Invalid TOML format: {}", e))
        })?;

        // Validate the parsed configuration
        tasks_config.validate()?;

        Ok(tasks_config)
    }
}

impl ServerConfig {
    /// Validate the server configuration
    pub fn validate(&self) -> crate::Result<()> {
        if self.listen_address.is_empty() {
            return Err(crate::MonitoringError::Validation(
                "listen_address cannot be empty".to_string(),
            )
            .into());
        }

        // Try to parse as socket address
        if self.listen_address.parse::<SocketAddr>().is_err() {
            return Err(crate::MonitoringError::Validation(format!(
                "invalid listen_address: {}",
                self.listen_address
            ))
            .into());
        }

        if self.api_key.is_empty() {
            return Err(
                crate::MonitoringError::Validation("api_key cannot be empty".to_string()).into(),
            );
        }

        if self.data_retention_days == 0 {
            return Err(crate::MonitoringError::Validation(
                "data_retention_days must be greater than 0".to_string(),
            )
            .into());
        }

        if self.reconfigure_check_interval_seconds == 0 {
            return Err(crate::MonitoringError::Validation(
                "reconfigure_check_interval_seconds must be at least 1".to_string(),
            )
            .into());
        }

        if self.reconfigure_check_interval_seconds > 300 {
            return Err(crate::MonitoringError::Validation(
                "reconfigure_check_interval_seconds must not exceed 300 (5 minutes)".to_string(),
            )
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_config_validation() {
        let mut config = AgentConfig {
            agent_id: "test-agent-01".to_string(),
            central_server_url: "https://monitoring.example.com".to_string(),
            api_key: "secret-key".to_string(),
            local_data_retention_days: 7,
            auto_update_tasks: true,
            local_only: false,
            metrics_flush_interval_seconds: 5,
            metrics_send_interval_seconds: 30,
            metrics_batch_size: 50,
            metrics_max_retries: 10,
            queue_cleanup_interval_seconds: 3600,
            data_cleanup_interval_seconds: 86400,
            max_concurrent_tasks: 50,
            http_response_max_size_mb: 100,
            http_client_timeout_seconds: 30,
            database_busy_timeout_seconds: 5,
            graceful_shutdown_timeout_seconds: 30,
            channel_buffer_size: 1000,
        };

        assert!(config.validate().is_ok());

        // Test empty agent_id
        config.agent_id = "".to_string();
        assert!(config.validate().is_err());

        config.agent_id = "test-agent-01".to_string();

        // Test invalid URL
        config.central_server_url = "not-a-url".to_string();
        assert!(config.validate().is_err());

        config.central_server_url = "https://monitoring.example.com".to_string();

        // Test zero retention days
        config.local_data_retention_days = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_task_config_validation() {
        let ping_task = TaskConfig {
            task_type: TaskType::Ping,
            schedule_seconds: 10,
            name: "Test Ping".to_string(),
            timeout: None,
            params: TaskParams::Ping(PingParams {
                host: "8.8.8.8".to_string(),
                timeout_seconds: 1,
                target_id: None,
            }),
        };

        assert!(ping_task.validate().is_ok());

        let http_task = TaskConfig {
            task_type: TaskType::HttpGet,
            schedule_seconds: 30,
            name: "Test HTTP".to_string(),
            timeout: Some(15),
            params: TaskParams::HttpGet(HttpGetParams {
                url: "https://example.com".to_string(),
                timeout_seconds: 10,
                headers: HashMap::new(),
                verify_ssl: false,
                target_id: None,
            }),
        };

        assert!(http_task.validate().is_ok());
    }

    #[test]
    fn test_get_effective_timeout() {
        let ping_task = TaskConfig {
            task_type: TaskType::Ping,
            schedule_seconds: 10,
            name: "Test Ping".to_string(),
            timeout: Some(3),
            params: TaskParams::Ping(PingParams {
                host: "8.8.8.8".to_string(),
                timeout_seconds: 1,
                target_id: None,
            }),
        };

        // Should use task-level timeout when specified
        assert_eq!(ping_task.get_effective_timeout(), 3);

        let ping_task_no_override = TaskConfig {
            task_type: TaskType::Ping,
            schedule_seconds: 10,
            name: "Test Ping No Override".to_string(),
            timeout: None,
            params: TaskParams::Ping(PingParams {
                host: "8.8.8.8".to_string(),
                timeout_seconds: 1,
                target_id: None,
            }),
        };

        // Should use default timeout when task-level timeout is None
        assert_eq!(ping_task_no_override.get_effective_timeout(), 1);

        let http_task = TaskConfig {
            task_type: TaskType::HttpGet,
            schedule_seconds: 30,
            name: "Test HTTP".to_string(),
            timeout: None,
            params: TaskParams::HttpGet(HttpGetParams {
                url: "https://example.com".to_string(),
                timeout_seconds: 10,
                headers: HashMap::new(),
                verify_ssl: false,
                target_id: None,
            }),
        };

        // Should use HTTP default timeout
        assert_eq!(http_task.get_effective_timeout(), 10);
    }

    #[test]
    fn test_bandwidth_task_minimum_schedule() {
        let valid_bandwidth_task = TaskConfig {
            task_type: TaskType::Bandwidth,
            schedule_seconds: 60,
            name: "Valid Bandwidth Test".to_string(),
            timeout: None,
            params: TaskParams::Bandwidth(BandwidthParams {
                timeout_seconds: 60,
                max_retries: 10,
                target_id: None,
            }),
        };

        // Should pass validation with 60 second schedule
        assert!(valid_bandwidth_task.validate().is_ok());

        let invalid_bandwidth_task = TaskConfig {
            task_type: TaskType::Bandwidth,
            schedule_seconds: 30, // Less than 60 seconds
            name: "Invalid Bandwidth Test".to_string(),
            timeout: None,
            params: TaskParams::Bandwidth(BandwidthParams {
                timeout_seconds: 60,
                max_retries: 10,
                target_id: None,
            }),
        };

        // Should fail validation with less than 60 second schedule
        assert!(invalid_bandwidth_task.validate().is_err());

        let error_msg = invalid_bandwidth_task.validate().unwrap_err().to_string();
        assert!(error_msg.contains("Bandwidth tasks must have schedule_seconds >= 60"));
    }

    #[test]
    fn test_task_params_ordering() {
        // Test that HttpContent (more specific) is correctly deserialized
        // even when it comes before HttpGet in the untagged enum
        let toml_str = r#"
[[tasks]]
type = "http_content"
name = "Content Check"
schedule_seconds = 30
url = "https://example.com"
regexp = "test"
timeout_seconds = 30

[[tasks]]
type = "http_get"
name = "GET Request"
schedule_seconds = 30
url = "https://example.com"
timeout_seconds = 30
"#;

        let config: TasksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tasks.len(), 2);

        // Verify first task is HttpContent
        assert_eq!(config.tasks[0].task_type, TaskType::HttpContent);
        match &config.tasks[0].params {
            TaskParams::HttpContent(p) => {
                assert_eq!(p.url, "https://example.com");
                assert_eq!(p.regexp, "test");
            }
            _ => panic!("Expected HttpContent params"),
        }

        // Verify second task is HttpGet
        assert_eq!(config.tasks[1].task_type, TaskType::HttpGet);
        match &config.tasks[1].params {
            TaskParams::HttpGet(p) => {
                assert_eq!(p.url, "https://example.com");
            }
            _ => panic!("Expected HttpGet params"),
        }

        // Validate both tasks
        assert!(config.tasks[0].validate().is_ok());
        assert!(config.tasks[1].validate().is_ok());
    }

    #[test]
    fn test_type_field_is_respected() {
        // With the custom deserializer, the type field determines which params
        // struct to use, regardless of which fields are present

        // Test 1: type=http_get with extra regexp field
        // The regexp field should be ignored since HttpGetParams doesn't have it
        let toml_str = r#"
[[tasks]]
type = "http_get"
name = "HttpGet with extra field"
schedule_seconds = 30
url = "https://example.com"
regexp = "ignored"
"#;

        let config: TasksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tasks.len(), 1);
        assert_eq!(config.tasks[0].task_type, TaskType::HttpGet);

        // Should deserialize as HttpGet (extra field ignored)
        match &config.tasks[0].params {
            TaskParams::HttpGet(_) => {} // Expected
            _ => panic!("Expected HttpGet params"),
        }

        // Validation should pass (type and params match)
        assert!(config.tasks[0].validate().is_ok());

        // Test 2: type=http_content without required regexp field
        // Should fail at deserialization
        let toml_str2 = r#"
[[tasks]]
type = "http_content"
name = "HttpContent missing regexp"
schedule_seconds = 30
url = "https://example.com"
"#;

        let result = toml::from_str::<TasksConfig>(toml_str2);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("regexp") || err_msg.contains("missing field"));
    }

    #[test]
    fn test_tcp_task_validation() {
        // Test valid TCP task
        let tcp_task = TaskConfig {
            task_type: TaskType::Tcp,
            schedule_seconds: 60,
            name: "SSH Port Check".to_string(),
            timeout: None,
            params: TaskParams::Tcp(TcpParams {
                host: "example.com:22".to_string(),
                timeout_seconds: 5,
                target_id: Some("ssh-server".to_string()),
            }),
        };

        assert!(tcp_task.validate().is_ok());

        // Test TCP task with missing host
        let invalid_tcp_task = TaskConfig {
            task_type: TaskType::Tcp,
            schedule_seconds: 60,
            name: "Invalid TCP".to_string(),
            timeout: None,
            params: TaskParams::Tcp(TcpParams {
                host: "".to_string(),
                timeout_seconds: 5,
                target_id: None,
            }),
        };

        assert!(invalid_tcp_task.validate().is_err());

        // Test TCP task from TOML
        let toml_str = r#"
[[tasks]]
type = "tcp"
name = "wilam SSH Port Check"
schedule_seconds = 1
host = "wilam.ovh:22"
timeout_seconds = 5
"#;

        let config: TasksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tasks.len(), 1);
        assert_eq!(config.tasks[0].task_type, TaskType::Tcp);
        assert!(config.tasks[0].validate().is_ok());

        match &config.tasks[0].params {
            TaskParams::Tcp(params) => {
                assert_eq!(params.host, "wilam.ovh:22");
                assert_eq!(params.timeout_seconds, 5);
            }
            _ => panic!("Expected Tcp params"),
        }
    }

    #[test]
    fn test_tls_handshake_task_validation() {
        // Test valid TLS handshake task
        let tls_task = TaskConfig {
            task_type: TaskType::TlsHandshake,
            schedule_seconds: 60,
            name: "TLS Check".to_string(),
            timeout: None,
            params: TaskParams::TlsHandshake(TlsHandshakeParams {
                host: "example.com:443".to_string(),
                verify_ssl: true,
                target_id: None,
            }),
        };

        assert!(tls_task.validate().is_ok());

        // Test TLS task with missing host
        let invalid_tls_task = TaskConfig {
            task_type: TaskType::TlsHandshake,
            schedule_seconds: 60,
            name: "Invalid TLS".to_string(),
            timeout: None,
            params: TaskParams::TlsHandshake(TlsHandshakeParams {
                host: "".to_string(),
                verify_ssl: true,
                target_id: None,
            }),
        };

        assert!(invalid_tls_task.validate().is_err());
    }

    #[test]
    fn test_toml_serialization() {
        let config = AgentConfig {
            agent_id: "test-agent".to_string(),
            central_server_url: "https://example.com".to_string(),
            api_key: "secret".to_string(),
            local_data_retention_days: 30,
            auto_update_tasks: true,
            local_only: false,
            metrics_flush_interval_seconds: 5,
            metrics_send_interval_seconds: 30,
            metrics_batch_size: 50,
            metrics_max_retries: 10,
            queue_cleanup_interval_seconds: 3600,
            data_cleanup_interval_seconds: 86400,
            max_concurrent_tasks: 50,
            http_response_max_size_mb: 100,
            http_client_timeout_seconds: 30,
            database_busy_timeout_seconds: 5,
            graceful_shutdown_timeout_seconds: 30,
            channel_buffer_size: 1000,
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: AgentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config, parsed);
    }
}
