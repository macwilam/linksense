//! API types and structures for client-server communication
//!
//! This module defines the request and response types used by the REST API
//! endpoints between the agent and central server.

use serde::{Deserialize, Serialize};

/// Status of agent configuration on the server
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigStatus {
    /// Agent configuration is up to date
    UpToDate,
    /// Agent configuration is stale and needs updating
    Stale,
}

/// Generic API request wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest<T> {
    pub data: T,
    pub timestamp_utc: String,
}

/// Generic API response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub status: String,
    pub data: Option<T>,
    pub error: Option<String>,
}

/// Request body for POST /api/v1/metrics endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsRequest {
    pub agent_id: String,
    pub timestamp_utc: String,
    pub config_checksum: String,
    pub metrics: Vec<crate::metrics::AggregatedMetrics>,
    #[serde(default)]
    pub agent_version: Option<String>,
}

/// Response body for POST /api/v1/metrics endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsResponse {
    pub status: String,
    pub config_status: ConfigStatus,
}

/// Response body for GET /api/v1/configs endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigsResponse {
    pub agent_toml: String, // Base64 encoded
    pub tasks_toml: String, // Base64 encoded
}

/// Request body for POST /api/v1/config/error endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigErrorRequest {
    pub agent_id: String,
    pub timestamp_utc: String,
    pub error_message: String,
}

/// Request body for POST /api/v1/config/verify endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigVerifyRequest {
    pub agent_id: String,
    pub tasks_config_hash: String,
}

/// Response body for POST /api/v1/config/verify endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigVerifyResponse {
    pub status: String,
    pub config_status: ConfigStatus,
    pub tasks_toml: Option<String>, // Base64 encoded gzipped content if config_status is Stale
}

/// Request body for POST /api/v1/config/upload endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigUploadRequest {
    pub agent_id: String,
    pub timestamp_utc: String,
    pub tasks_toml: String, // Base64 encoded gzipped content
}

/// Response body for POST /api/v1/config/upload endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigUploadResponse {
    pub status: String,
    pub message: String,
    pub accepted: bool,
}

/// Request body for POST /api/v1/bandwidth_test endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandwidthTestRequest {
    pub agent_id: String,
    pub timestamp_utc: String,
}

/// Response body for POST /api/v1/bandwidth_test endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandwidthTestResponse {
    pub status: String,
    pub action: BandwidthTestAction,
    pub delay_seconds: Option<u32>,
    pub data_size_bytes: Option<u64>,
}

/// Bandwidth test action from server
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BandwidthTestAction {
    /// Proceed with the test immediately
    Proceed,
    /// Delay the test by the specified number of seconds
    Delay,
}

/// HTTP headers used for authentication and metadata
pub mod headers {
    pub const API_KEY: &str = "X-API-Key";
    pub const AGENT_ID: &str = "X-Agent-Id";
    pub const CONTENT_TYPE: &str = "Content-Type";
}

/// API endpoint paths
pub mod endpoints {
    pub const METRICS: &str = "/api/v1/metrics";
    pub const CONFIGS: &str = "/api/v1/configs";
    pub const CONFIG_ERROR: &str = "/api/v1/config/error";
    pub const CONFIG_VERIFY: &str = "/api/v1/config/verify";
    pub const CONFIG_UPLOAD: &str = "/api/v1/config/upload";
    pub const BANDWIDTH_TEST: &str = "/api/v1/bandwidth_test";
    pub const BANDWIDTH_DOWNLOAD: &str = "/api/v1/bandwidth_download";
}

impl<T> ApiResponse<T> {
    /// Create a successful API response
    pub fn success(data: T) -> Self {
        Self {
            status: "success".to_string(),
            data: Some(data),
            error: None,
        }
    }

    /// Create an error API response
    pub fn error(error_message: String) -> Self {
        Self {
            status: "error".to_string(),
            data: None,
            error: Some(error_message),
        }
    }
}

impl MetricsResponse {
    /// Create a metrics response indicating config is up to date
    pub fn up_to_date() -> Self {
        Self {
            status: "success".to_string(),
            config_status: ConfigStatus::UpToDate,
        }
    }

    /// Create a metrics response indicating config is stale
    pub fn stale() -> Self {
        Self {
            status: "success".to_string(),
            config_status: ConfigStatus::Stale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_status_serialization() {
        let up_to_date = ConfigStatus::UpToDate;
        let json = serde_json::to_string(&up_to_date).unwrap();
        assert_eq!(json, "\"up_to_date\"");

        let stale = ConfigStatus::Stale;
        let json = serde_json::to_string(&stale).unwrap();
        assert_eq!(json, "\"stale\"");
    }

    #[test]
    fn test_metrics_response_creation() {
        let response = MetricsResponse::up_to_date();
        assert_eq!(response.status, "success");
        assert_eq!(response.config_status, ConfigStatus::UpToDate);

        let response = MetricsResponse::stale();
        assert_eq!(response.config_status, ConfigStatus::Stale);
    }

    #[test]
    fn test_api_response_helpers() {
        let success_response = ApiResponse::success("test data");
        assert_eq!(success_response.status, "success");
        assert_eq!(success_response.data, Some("test data"));
        assert_eq!(success_response.error, None);

        let error_response: ApiResponse<()> = ApiResponse::error("test error".to_string());
        assert_eq!(error_response.status, "error");
        assert_eq!(error_response.data, None);
        assert_eq!(error_response.error, Some("test error".to_string()));
    }
}
