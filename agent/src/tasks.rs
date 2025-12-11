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

    /// Refreshes HTTP clients and TLS connectors by dropping old instances and creating new ones
    ///
    /// This method explicitly drops the existing reqwest client and TLS connectors,
    /// then creates fresh instances. This helps prevent memory leaks and ensures
    /// clean state for HTTP/TLS operations.
    ///
    /// # Returns
    /// `Ok(())` on successful refresh, error if client/connector creation fails
    pub fn refresh_clients(&mut self) -> Result<()> {
        debug!("Refreshing HTTP clients and TLS connectors");

        // Create new instances
        self.http_content_client = reqwest::Client::builder()
            .build()
            .context("Failed to create new HTTP client for content tasks")?;

        self.tls_connector_verify = crate::task_tls::create_tls_connector_with_verification()
            .context("Failed to create new TLS connector with verification")?;

        self.tls_connector_no_verify = crate::task_tls::create_tls_connector_without_verification()
            .context("Failed to create new TLS connector without verification")?;

        debug!("Successfully refreshed HTTP clients and TLS connectors");
        Ok(())
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
                    #[cfg(feature = "snmp-tasks")]
                    TaskType::Snmp => self.execute_snmp_task(task_config).await,
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
                        mode: params.mode.clone(),
                        value: None,
                        json_result: None,
                        json_truncated: false,
                        column_count: None,
                    }),
                ),
            };

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for SQL query task"))
        }
    }

    /// Executes an SNMP query task
    #[cfg(feature = "snmp-tasks")]
    async fn execute_snmp_task(&self, task_config: &TaskConfig) -> Result<MetricData> {
        debug!("Executing SNMP task: {}", task_config.name);

        if let TaskParams::Snmp(params) = &task_config.params {
            let metric_result = crate::task_snmp::execute_snmp_task(params).await;

            let metric_data = match metric_result {
                Ok(metric) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::Snmp,
                    RawMetricData::Snmp(metric),
                ),
                Err(e) => MetricData::new(
                    task_config.name.clone(),
                    TaskType::Snmp,
                    RawMetricData::Snmp(shared::metrics::RawSnmpMetric {
                        response_time_ms: None,
                        success: false,
                        value: None,
                        value_type: None,
                        oid_queried: params.oid.clone(),
                        error: Some(e.to_string()),
                        target_id: params.target_id.clone(),
                    }),
                ),
            };

            Ok(metric_data)
        } else {
            Err(anyhow::anyhow!("Invalid parameters for SNMP task"))
        }
    }
}
