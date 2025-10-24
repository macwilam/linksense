//! Task execution implementations for the network monitoring agent
//!
//! This module contains the implementation of all monitoring tasks:
//! - ICMP Ping tasks (delegated to task_ping module)
//! - HTTP GET tasks (delegated to task_http module)
//! - HTTP Content tasks (delegated to task_http_content module)
//! - DNS Query tasks (delegated to task_dns module)
//! - Bandwidth measurement tasks
//! - SQL Query tasks (delegated to task_sql module)
//!
//! This module is the "worker" part of the agent. While the scheduler decides
//! *when* to run tasks, this module defines *what* each task actually does.
//! Each task is implemented as a function that performs a network measurement
//! and returns the result in a structured format.

use anyhow::{Context, Result};
use shared::config::{TaskConfig, TaskParams, TaskType};
use shared::metrics::{MetricData, RawMetricData};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tracing::{debug, error};

/// Represents the result of a single task execution.
/// This struct is sent from the `TaskExecutor` back to the `TaskScheduler`
/// to report the outcome of a task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// The name of the task that was executed.
    pub task_name: String,
    /// A simple boolean indicating success or failure.
    pub success: bool,
    /// If the task completed (even if it failed in a way that still yields data),
    /// this field will contain the collected metric data.
    pub metric_data: Option<MetricData>,
    /// If the task failed, this contains a string explaining the error.
    pub error: Option<String>,
    /// The time it took to execute the task, in milliseconds.
    pub execution_time_ms: f64,
}

/// The `TaskExecutor` is responsible for running individual monitoring tasks.
/// It acts as a dispatcher, calling the appropriate implementation based on the
// task's type.
#[derive(Clone)]
pub struct TaskExecutor {
    /// The sender part of a channel used to send `TaskResult`s back to the scheduler.
    result_sender: Arc<mpsc::Sender<TaskResult>>,
    /// Server URL for bandwidth tests (optional, only needed for server-connected agents)
    server_url: Option<String>,
    /// API key for server authentication (optional, only needed for server-connected agents)
    api_key: Option<String>,
    /// Agent ID for identification (optional, only needed for server-connected agents)
    agent_id: Option<String>,
    /// Shared HTTP client for HTTP content tasks (reused across all requests)
    http_content_client: reqwest::Client,
    /// Shared TLS connector with verification enabled (reused across all TLS connections)
    tls_connector_verify: TlsConnector,
    /// Shared TLS connector with verification disabled (reused across all TLS connections)
    tls_connector_no_verify: TlsConnector,
}

impl TaskExecutor {
    /// Creates a new `TaskExecutor` with optional agent configuration
    ///
    /// The agent configuration (server_url, api_key, agent_id) is required for
    /// bandwidth tests but optional for standalone agents.
    pub fn new(
        result_sender: mpsc::Sender<TaskResult>,
        server_url: Option<String>,
        api_key: Option<String>,
        agent_id: Option<String>,
    ) -> Result<Self> {
        // Create a shared HTTP client for HTTP content tasks
        // This client is reused across all HTTP content requests to avoid
        // the overhead of creating a new client for each request (~50µs per request)
        let http_content_client = reqwest::Client::builder()
            .build()
            .context("Failed to create shared HTTP client for content tasks")?;

        // Create shared TLS connectors for TLS connections
        // These are reused across all TLS handshakes to avoid repeated initialization overhead
        // We create two: one with TLS verification enabled, one with it disabled
        let tls_connector_verify = crate::task_tls::create_tls_connector_with_verification()
            .context("Failed to create TLS connector with verification")?;

        let tls_connector_no_verify = crate::task_tls::create_tls_connector_without_verification()
            .context("Failed to create TLS connector without verification")?;

        Ok(Self {
            result_sender: Arc::new(result_sender),
            server_url,
            api_key,
            agent_id,
            http_content_client,
            tls_connector_verify,
            tls_connector_no_verify,
        })
    }

    /// Executes a given task based on its configuration.
    /// This is the main entry point for the executor. It measures the execution
    /// time, calls the appropriate task implementation, and then sends the
    /// result back to the scheduler.
    pub async fn execute_task(&self, task_config: &TaskConfig) -> Result<()> {
        let start_time = Instant::now();
        debug!(
            "Executing task: {} ({:?})",
            task_config.name, task_config.task_type
        );

        let timeout_duration = Duration::from_secs(task_config.get_effective_timeout() as u64);

        // Use tokio::select to handle timeout
        let result = tokio::select! {
            task_result = async {
                // A `match` statement is used to dispatch to the correct task function
                // based on the `task_type` enum. This is a clean and type-safe way to
                // handle different kinds of tasks.
                match &task_config.task_type {
                    TaskType::Ping => crate::task_ping::execute_ping_task(task_config).await,
                    TaskType::Tcp => self.execute_tcp_task(task_config).await,
                    TaskType::HttpGet => self.execute_http_task(task_config).await,
                    TaskType::HttpContent => self.execute_http_content_task(task_config).await,
                    TaskType::TlsHandshake => self.execute_tls_task(task_config).await,
                    TaskType::DnsQuery => self.execute_dns_task(task_config).await,
                    TaskType::DnsQueryDoh => self.execute_dns_doh_task(task_config).await,
                    TaskType::Bandwidth => self.execute_bandwidth_task(task_config).await,
                    #[cfg(feature = "sql-tasks")]
                    TaskType::SqlQuery => self.execute_sql_query_task(task_config).await,
                }
            } => task_result,
            _ = tokio::time::sleep(timeout_duration) => {
                Err(anyhow::anyhow!("Task timed out after {} seconds", task_config.get_effective_timeout()))
            }
        };

        let execution_time = start_time.elapsed().as_secs_f64() * 1000.0;

        // The result of the task execution (either `Ok(MetricData)` or `Err(e)`)
        // is transformed into a `TaskResult` struct.
        let task_result = match result {
            Ok(metric_data) => TaskResult {
                task_name: task_config.name.clone(),
                success: true,
                metric_data: Some(metric_data),
                error: None,
                execution_time_ms: execution_time,
            },
            Err(e) => {
                error!("Task '{}' failed: {}", task_config.name, e);
                TaskResult {
                    task_name: task_config.name.clone(),
                    success: false,
                    metric_data: None,
                    error: Some(e.to_string()),
                    execution_time_ms: execution_time,
                }
            }
        };

        // The final result is sent back to the scheduler via the channel.
        if let Err(e) = self.result_sender.send(task_result).await {
            error!(
                "Failed to send task result for '{}': {}",
                task_config.name, e
            );
        }

        Ok(())
    }

    /// Executes an HTTP GET task.
    async fn execute_http_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing HTTP task: {}", task_config.name);
        // tokio::time::sleep(Duration::from_millis(2500)).await;
        // let metric_data = MetricData::new(
        //     task_config.name.clone(),
        //     TaskType::HttpGet,
        //     RawMetricData::HttpGet(shared::metrics::RawHttpMetric {
        //         status_code: None,
        //         dns_timing_ms: None,
        //         tcp_timing_ms: None,
        //         tls_timing_ms: None,
        //         ttfb_timing_ms: None,
        //         content_download_timing_ms: None,
        //         total_time_ms: None,
        //         success: false,
        //         error: Some("dsfsfd".to_string()),
        //     }),
        // );
        // return Ok(metric_data);

        if let TaskParams::HttpGet(params) = &task_config.params {
            // Use our async HTTP timing implementation
            let timeout = Some(std::time::Duration::from_secs(
                params.timeout_seconds as u64,
            ));

            // Select the appropriate shared TLS connector based on verify_ssl setting
            let connector = if params.verify_ssl {
                &self.tls_connector_verify
            } else {
                &self.tls_connector_no_verify
            };

            let timing_result =
                crate::task_http::from_string(&params.url, timeout, connector).await;
            let metric_data = match timing_result {
                Ok(response) => {
                    let total_time_ms = (response.timings.tcp
                        + response.timings.tls.unwrap_or_default()
                        + response.timings.ttfb
                        + response.timings.content_download)
                        .as_millis() as u64;

                    // Calculate SSL validity and days until expiry
                    let (ssl_valid, ssl_cert_days_until_expiry) =
                        if let Some(ref cert_info) = response.certificate_information {
                            let is_valid = cert_info.is_active;
                            let days_until_expiry = match cert_info
                                .expires_at
                                .duration_since(std::time::SystemTime::now())
                            {
                                Ok(duration) => (duration.as_secs() / 86400) as i64,
                                Err(_) => {
                                    // Certificate is expired
                                    match std::time::SystemTime::now()
                                        .duration_since(cert_info.expires_at)
                                    {
                                        Ok(duration) => -((duration.as_secs() / 86400) as i64),
                                        Err(_) => 0,
                                    }
                                }
                            };
                            (Some(is_valid), Some(days_until_expiry))
                        } else {
                            (None, None)
                        };

                    let metric = MetricData::new(
                        task_config.name.clone(),
                        TaskType::HttpGet,
                        RawMetricData::HttpGet(shared::metrics::RawHttpMetric {
                            status_code: Some(response.status),
                            tcp_timing_ms: Some(response.timings.tcp.as_millis() as f64),
                            tls_timing_ms: response.timings.tls.map(|t| t.as_millis() as f64),
                            ttfb_timing_ms: Some(response.timings.ttfb.as_millis() as f64),
                            content_download_timing_ms: Some(
                                response.timings.content_download.as_millis() as f64,
                            ),
                            total_time_ms: Some(total_time_ms as f64),
                            success: response.status >= 200 && response.status < 400,
                            error: None,
                            ssl_valid,
                            ssl_cert_days_until_expiry,
                            target_id: params.target_id.clone(),
                        }),
                    );

                    // Explicitly drop SSL certificate and certificate info to free memory
                    // This is important because OpenSSL FFI objects may hold additional references
                    // The drop happens when response goes out of scope here, but we're explicit about it
                    drop(response);

                    metric
                }
                Err(err) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::HttpGet,
                    RawMetricData::HttpGet(shared::metrics::RawHttpMetric {
                        status_code: None,
                        tcp_timing_ms: None,
                        tls_timing_ms: None,
                        ttfb_timing_ms: None,
                        content_download_timing_ms: None,
                        total_time_ms: None,
                        success: false,
                        error: Some(err.to_string()),
                        ssl_valid: None,
                        ssl_cert_days_until_expiry: None,
                        target_id: params.target_id.clone(),
                    }),
                ),
            };

            // Check if the HTTP request failed
            if let RawMetricData::HttpGet(ref http_metric) = metric_data.data {
                if !http_metric.success {
                    return Err(anyhow::anyhow!(
                        "HTTP request failed: {}",
                        http_metric.error.as_deref().unwrap_or("Unknown error")
                    ));
                }
                // Check if SSL verification was requested and failed
                if params.verify_ssl {
                    if let Some(false) = http_metric.ssl_valid {
                        return Err(anyhow::anyhow!("SSL certificate validation failed"));
                    }
                }
            }

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for HTTP task"))
        }
    }

    /// Executes a TLS handshake task.
    async fn execute_tls_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing TLS handshake task: {}", task_config.name);

        if let TaskParams::TlsHandshake(params) = &task_config.params {
            // Get effective timeout from config (default or overridden)
            let timeout = Some(Duration::from_secs(
                task_config.get_effective_timeout() as u64
            ));

            // Select the appropriate shared TLS connector based on verify_ssl setting
            let connector = if params.verify_ssl {
                &self.tls_connector_verify
            } else {
                &self.tls_connector_no_verify
            };

            let result =
                crate::task_tls::check_tls_handshake_with_timeout(&params.host, timeout, connector)
                    .await;

            let metric_data = match result {
                Ok(check) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::TlsHandshake,
                    RawMetricData::TlsHandshake(shared::metrics::RawTlsMetric {
                        tcp_timing_ms: Some(check.tcp_timing.as_millis() as f64),
                        tls_timing_ms: check.tls_timing.map(|t| t.as_millis() as f64),
                        ssl_valid: check.ssl_valid,
                        ssl_cert_days_until_expiry: check.ssl_cert_days_until_expiry,
                        success: check.success,
                        error: check.error,
                        target_id: params.target_id.clone(),
                    }),
                ),
                Err(err) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::TlsHandshake,
                    RawMetricData::TlsHandshake(shared::metrics::RawTlsMetric {
                        tcp_timing_ms: None,
                        tls_timing_ms: None,
                        ssl_valid: None,
                        ssl_cert_days_until_expiry: None,
                        success: false,
                        error: Some(err.to_string()),
                        target_id: params.target_id.clone(),
                    }),
                ),
            };

            // Check if the TLS handshake failed
            if let RawMetricData::TlsHandshake(ref tls_metric) = metric_data.data {
                if !tls_metric.success {
                    return Err(anyhow::anyhow!(
                        "TLS handshake failed: {}",
                        tls_metric.error.as_deref().unwrap_or("Unknown error")
                    ));
                }
                // If SSL verification was requested, check if certificate is valid
                if params.verify_ssl {
                    if let Some(false) = tls_metric.ssl_valid {
                        return Err(anyhow::anyhow!("SSL certificate validation failed"));
                    }
                }
            }

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for TLS task"))
        }
    }

    /// Executes a TCP connection test task
    async fn execute_tcp_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing TCP connection task: {}", task_config.name);

        if let TaskParams::Tcp(params) = &task_config.params {
            let result = crate::task_tcp::execute_tcp_task(params).await;

            let metric_data = MetricData::new(
                task_config.name.clone(),
                TaskType::Tcp,
                RawMetricData::Tcp(result),
            );

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for TCP task"))
        }
    }

    /// Executes an HTTP content check task
    async fn execute_http_content_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing HTTP content task: {}", task_config.name);

        if let TaskParams::HttpContent(params) = &task_config.params {
            let metric_result = crate::task_http_content::execute_http_content_check(
                &self.http_content_client,
                params,
            )
            .await;

            let metric_data = match metric_result {
                Ok(metric) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::HttpContent,
                    RawMetricData::HttpContent(metric),
                ),
                Err(e) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::HttpContent,
                    RawMetricData::HttpContent(shared::metrics::RawHttpContentMetric {
                        status_code: None,
                        total_time_ms: None,
                        total_size: None,
                        regexp_match: None,
                        success: false,
                        error: Some(e.to_string()),
                        target_id: params.target_id.clone(),
                    }),
                ),
            };

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for HTTP content task"))
        }
    }

    /// Executes a DNS query task using regular DNS
    async fn execute_dns_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing DNS task: {}", task_config.name);

        if let TaskParams::DnsQuery(params) = &task_config.params {
            let dns_metric = crate::task_dns::execute_dns_query(params).await?;

            // Check if resolution failed or didn't match expected IP
            if !dns_metric.success {
                return Err(anyhow::anyhow!(
                    "DNS query failed: {}",
                    dns_metric.error.as_deref().unwrap_or("Unknown error")
                ));
            }

            if !dns_metric.correct_resolution {
                let resolved = dns_metric.resolved_ip.as_deref().unwrap_or("none");
                let expected = dns_metric.expected_ip.as_deref().unwrap_or("none");
                return Err(anyhow::anyhow!(
                    "Resolved IP '{}' does not match expected IP '{}'",
                    resolved,
                    expected
                ));
            }

            let metric_data = MetricData::new(
                task_config.name.clone(),
                TaskType::DnsQuery,
                RawMetricData::DnsQuery(dns_metric),
            );

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for DNS task"))
        }
    }

    /// Executes a DNS over HTTPS query task
    async fn execute_dns_doh_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing DNS over HTTPS task: {}", task_config.name);

        if let TaskParams::DnsQueryDoh(params) = &task_config.params {
            let dns_metric = crate::task_dns::execute_dns_over_https_query(params).await?;

            // Check if resolution failed or didn't match expected IP
            if !dns_metric.success {
                return Err(anyhow::anyhow!(
                    "DNS over HTTPS query failed: {}",
                    dns_metric.error.as_deref().unwrap_or("Unknown error")
                ));
            }

            if !dns_metric.correct_resolution {
                let resolved = dns_metric.resolved_ip.as_deref().unwrap_or("none");
                let expected = dns_metric.expected_ip.as_deref().unwrap_or("none");
                return Err(anyhow::anyhow!(
                    "Resolved IP '{}' does not match expected IP '{}'",
                    resolved,
                    expected
                ));
            }

            let metric_data = MetricData::new(
                task_config.name.clone(),
                TaskType::DnsQueryDoh,
                RawMetricData::DnsQuery(dns_metric),
            );

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!(
                "Invalid parameters for DNS over HTTPS task"
            ))
        }
    }

    /// Executes a bandwidth test task
    ///
    /// Coordinates with the server to ensure only one test runs at a time,
    /// then downloads test data and measures the bandwidth.
    async fn execute_bandwidth_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing bandwidth task: {}", task_config.name);

        if let TaskParams::Bandwidth(params) = &task_config.params {
            // Check if we have the required configuration for bandwidth tests
            let (server_url, api_key, agent_id) = match (
                self.server_url.as_ref(),
                self.api_key.as_ref(),
                self.agent_id.as_ref(),
            ) {
                (Some(url), Some(key), Some(id)) => (url.as_str(), key.as_str(), id.as_str()),
                _ => {
                    return Ok(MetricData::new(
                        task_config.name.clone(),
                        TaskType::Bandwidth,
                        RawMetricData::Bandwidth(shared::metrics::RawBandwidthMetric {
                            bandwidth_mbps: None,
                            duration_ms: None,
                            bytes_downloaded: None,
                            success: false,
                            error: Some(
                                "Bandwidth test requires server configuration (not available in local-only mode)"
                                    .to_string(),
                            ),
                            target_id: params.target_id.clone(),
                        }),
                    ));
                }
            };

            let metric_result = crate::task_bandwidth::execute_bandwidth_task(
                params, server_url, api_key, agent_id,
            )
            .await;

            let metric_data = match metric_result {
                Ok(metric) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::Bandwidth,
                    RawMetricData::Bandwidth(metric),
                ),
                Err(e) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::Bandwidth,
                    RawMetricData::Bandwidth(shared::metrics::RawBandwidthMetric {
                        bandwidth_mbps: None,
                        duration_ms: None,
                        bytes_downloaded: None,
                        success: false,
                        error: Some(e.to_string()),
                        target_id: params.target_id.clone(),
                    }),
                ),
            };

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for bandwidth task"))
        }
    }

    /// Executes a SQL query task
    #[cfg(feature = "sql-tasks")]
    async fn execute_sql_query_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing SQL query task: {}", task_config.name);

        if let TaskParams::SqlQuery(params) = &task_config.params {
            let metric_result = crate::task_sql::execute_sql_query(params).await;

            let metric_data = match metric_result {
                Ok(metric) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::SqlQuery,
                    RawMetricData::SqlQuery(metric),
                ),
                Err(e) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::SqlQuery,
                    RawMetricData::SqlQuery(shared::metrics::RawSqlQueryMetric {
                        total_time_ms: None,
                        row_count: None,
                        success: false,
                        error: Some(e.to_string()),
                        target_id: params.target_id.clone(),
                    }),
                ),
            };

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for SQL query task"))
        }
    }
}

// Unit tests for the task executor.
#[cfg(test)]
mod tests {
    use super::*;
    use shared::config::{
        BandwidthParams, DnsQueryDohParams, DnsQueryParams, DnsRecordType, HttpContentParams,
        HttpGetParams, PingParams, TaskParams, TcpParams, TlsHandshakeParams,
    };
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_task_executor_creation() {
        let (sender, _receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None);
        assert!(executor.is_ok());
    }

    #[tokio::test]
    async fn test_ping_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
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

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        // Verify that a result was sent through the channel and it has the correct data.
        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test Ping");
        assert!(task_result.success);
        assert!(task_result.metric_data.is_some());
    }

    #[tokio::test]
    async fn test_http_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
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

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test HTTP");
        assert!(task_result.success);
    }

    #[tokio::test]
    async fn test_dns_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::DnsQuery,
            schedule_seconds: 60,
            name: "Test DNS".to_string(),
            timeout: None,
            params: TaskParams::DnsQuery(DnsQueryParams {
                server: "1.1.1.1:53".to_string(),
                domain: "google.com".to_string(),
                record_type: DnsRecordType::A,
                timeout_seconds: 5,
                expected_ip: None,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test DNS");
        if !task_result.success {
            eprintln!("DNS task failed with error: {:?}", task_result.error);
        }
        assert!(
            task_result.success,
            "DNS task failed: {:?}",
            task_result.error
        );
    }

    #[tokio::test]
    async fn test_bandwidth_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::Bandwidth,
            schedule_seconds: 60, // Use minimum valid value
            name: "Test Bandwidth".to_string(),
            timeout: None,
            params: TaskParams::Bandwidth(BandwidthParams {
                timeout_seconds: 60,
                max_retries: 10,
                target_id: None,
            }),
        };

        // Validate the task config first
        assert!(task_config.validate().is_ok());

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test Bandwidth");
        // Note: The task may fail due to no server running, but it should at least try
        // and return a result with proper error handling
    }

    #[tokio::test]
    async fn test_bandwidth_task_validation() {
        // Test that bandwidth tasks with schedule < 60 seconds fail validation
        let invalid_task_config = TaskConfig {
            task_type: TaskType::Bandwidth,
            schedule_seconds: 30, // Less than minimum 60 seconds
            name: "Invalid Bandwidth Test".to_string(),
            timeout: None,
            params: TaskParams::Bandwidth(BandwidthParams {
                timeout_seconds: 60,
                max_retries: 10,
                target_id: None,
            }),
        };

        assert!(invalid_task_config.validate().is_err());

        // Test that bandwidth tasks with schedule >= 60 seconds pass validation
        let valid_task_config = TaskConfig {
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

        assert!(valid_task_config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_ping_timeout() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        // Use a non-routable IP address that should timeout
        let task_config = TaskConfig {
            task_type: TaskType::Ping,
            schedule_seconds: 10,
            name: "Test Ping Timeout".to_string(),
            timeout: None,
            params: TaskParams::Ping(PingParams {
                host: "192.0.2.1".to_string(), // TEST-NET-1, reserved for documentation
                timeout_seconds: 1,
                target_id: None,
            }),
        };

        let start = std::time::Instant::now();
        let result = executor.execute_task(&task_config).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test Ping Timeout");

        // Verify timeout was respected (should be around 1 second, not hang indefinitely)
        assert!(
            elapsed.as_secs() <= 3,
            "Timeout took too long: {:?}",
            elapsed
        );

        // The key assertion: timeout was respected and task completed within reasonable time
        // This proves the timeout mechanism is working (task didn't hang indefinitely)
    }

    #[tokio::test]
    async fn test_http_timeout() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        // Use a non-routable IP that will timeout
        let task_config = TaskConfig {
            task_type: TaskType::HttpGet,
            schedule_seconds: 30,
            name: "Test HTTP Timeout".to_string(),
            timeout: None,
            params: TaskParams::HttpGet(HttpGetParams {
                url: "http://192.0.2.1/test".to_string(), // TEST-NET-1
                timeout_seconds: 2,
                headers: HashMap::new(),
                verify_ssl: false,
                target_id: None,
            }),
        };

        let start = std::time::Instant::now();
        let result = executor.execute_task(&task_config).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test HTTP Timeout");

        // Verify timeout was respected (should be around 2 seconds)
        assert!(
            elapsed.as_secs() <= 5,
            "HTTP timeout took too long: {:?}",
            elapsed
        );

        // The key assertion: timeout was respected and task completed within reasonable time
        // This proves the timeout mechanism is working (task didn't hang indefinitely)
    }

    #[tokio::test]
    async fn test_dns_timeout() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        // Use an IP that doesn't respond to DNS queries
        let task_config = TaskConfig {
            task_type: TaskType::DnsQuery,
            schedule_seconds: 60,
            name: "Test DNS Timeout".to_string(),
            timeout: None,
            params: TaskParams::DnsQuery(DnsQueryParams {
                server: "192.0.2.1:53".to_string(), // TEST-NET-1
                domain: "example.com".to_string(),
                record_type: DnsRecordType::A,
                timeout_seconds: 2,
                expected_ip: None,
                target_id: None,
            }),
        };

        let start = std::time::Instant::now();
        let result = executor.execute_task(&task_config).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test DNS Timeout");

        // Verify timeout was respected (should be around 2 seconds, not hang indefinitely)
        assert!(
            elapsed.as_secs() <= 5,
            "DNS timeout took too long: {:?}",
            elapsed
        );

        // The key assertion: timeout was respected and task completed within reasonable time
        // This proves the timeout mechanism is working (task didn't hang indefinitely)
    }

    #[tokio::test]
    async fn test_http_task_failed_domain() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::HttpGet,
            schedule_seconds: 30,
            name: "Test HTTP Failure".to_string(),
            timeout: None,
            params: TaskParams::HttpGet(HttpGetParams {
                url: "https://non-existent-domain-12345.com".to_string(),
                timeout_seconds: 5,
                headers: HashMap::new(),
                verify_ssl: false,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test HTTP Failure");
        assert!(!task_result.success);
        assert!(task_result.error.is_some());
    }

    #[tokio::test]
    async fn test_dns_task_expected_ip_match() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::DnsQuery,
            schedule_seconds: 60,
            name: "Test DNS Expected IP Match".to_string(),
            timeout: None,
            params: TaskParams::DnsQuery(DnsQueryParams {
                server: "1.1.1.1:53".to_string(),
                domain: "one.one.one.one".to_string(),
                record_type: DnsRecordType::A,
                timeout_seconds: 5,
                expected_ip: Some("1.1.1.1".to_string()),
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test DNS Expected IP Match");
        assert!(
            task_result.success,
            "Task failed when it should have succeeded: {:?}",
            task_result.error
        );
    }

    #[tokio::test]
    async fn test_dns_task_expected_ip_mismatch() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::DnsQuery,
            schedule_seconds: 60,
            name: "Test DNS Expected IP Mismatch".to_string(),
            timeout: None,
            params: TaskParams::DnsQuery(DnsQueryParams {
                server: "1.1.1.1:53".to_string(),
                domain: "one.one.one.one".to_string(),
                record_type: DnsRecordType::A,
                timeout_seconds: 5,
                expected_ip: Some("1.2.3.4".to_string()),
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test DNS Expected IP Mismatch");
        assert!(!task_result.success);
        assert!(task_result.error.is_some());
        assert!(task_result
            .error
            .unwrap()
            .contains("does not match expected IP"));
    }

    #[tokio::test]
    async fn test_tcp_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::Tcp,
            schedule_seconds: 10,
            name: "Test TCP".to_string(),
            timeout: None,
            params: TaskParams::Tcp(TcpParams {
                host: "google.com:80".to_string(),
                timeout_seconds: 5,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test TCP");
        assert!(task_result.success);
    }

    #[tokio::test]
    async fn test_tls_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        // Test with SSL verification enabled (should fail)
        let task_config_verify = TaskConfig {
            task_type: TaskType::TlsHandshake,
            schedule_seconds: 30,
            name: "Test TLS Verify".to_string(),
            timeout: None,
            params: TaskParams::TlsHandshake(TlsHandshakeParams {
                host: "expired.badssl.com:443".to_string(),
                verify_ssl: true,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config_verify).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test TLS Verify");
        assert!(!task_result.success);

        // Test with SSL verification disabled (should succeed)
        let task_config_no_verify = TaskConfig {
            task_type: TaskType::TlsHandshake,
            schedule_seconds: 30,
            name: "Test TLS No Verify".to_string(),
            timeout: None,
            params: TaskParams::TlsHandshake(TlsHandshakeParams {
                host: "expired.badssl.com:443".to_string(),
                verify_ssl: false,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config_no_verify).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test TLS No Verify");
        assert!(task_result.success);
    }

    #[tokio::test]
    async fn test_http_content_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::HttpContent,
            schedule_seconds: 30,
            name: "Test HTTP Content".to_string(),
            timeout: None,
            params: TaskParams::HttpContent(HttpContentParams {
                url: "https://example.com".to_string(),
                regexp: "Example Domain".to_string(),
                timeout_seconds: 10,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test HTTP Content");
        assert!(task_result.success);
    }

    #[tokio::test]
    async fn test_dns_doh_task_execution() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        let task_config = TaskConfig {
            task_type: TaskType::DnsQueryDoh,
            schedule_seconds: 60,
            name: "Test DoH".to_string(),
            timeout: None,
            params: TaskParams::DnsQueryDoh(DnsQueryDohParams {
                server_url: "https://chrome.cloudflare-dns.com/dns-query".to_string(),
                domain: "google.com".to_string(),
                record_type: DnsRecordType::A,
                timeout_seconds: 5,
                expected_ip: None,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test DoH");
        assert!(task_result.success);
    }

    #[tokio::test]
    async fn test_http_get_ssl_verification() {
        let (sender, mut receiver) = mpsc::channel(100);
        let executor = TaskExecutor::new(sender, None, None, None).unwrap();

        // Test with SSL verification enabled (should fail)
        let task_config_verify = TaskConfig {
            task_type: TaskType::HttpGet,
            schedule_seconds: 30,
            name: "Test HTTP Verify SSL".to_string(),
            timeout: None,
            params: TaskParams::HttpGet(HttpGetParams {
                url: "https://expired.badssl.com/".to_string(),
                timeout_seconds: 10,
                headers: HashMap::new(),
                verify_ssl: true,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config_verify).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test HTTP Verify SSL");
        assert!(!task_result.success);

        // Test with SSL verification disabled (should succeed)
        let task_config_no_verify = TaskConfig {
            task_type: TaskType::HttpGet,
            schedule_seconds: 30,
            name: "Test HTTP No Verify SSL".to_string(),
            timeout: None,
            params: TaskParams::HttpGet(HttpGetParams {
                url: "https://expired.badssl.com/".to_string(),
                timeout_seconds: 10,
                headers: HashMap::new(),
                verify_ssl: false,
                target_id: None,
            }),
        };

        let result = executor.execute_task(&task_config_no_verify).await;
        assert!(result.is_ok());

        let task_result = receiver.recv().await.unwrap();
        assert_eq!(task_result.task_name, "Test HTTP No Verify SSL");
        assert!(task_result.success);
    }
}
