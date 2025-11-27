//! Tests for API types and structures

use crate::api::{ApiResponse, ConfigStatus, MetricsResponse};

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
