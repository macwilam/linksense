//! Test utility functions
//!
//! These functions are only used in tests and are not part of the public API.

use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use std::time::{SystemTime, UNIX_EPOCH};

/// Get current Unix timestamp in seconds
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Decode base64 string
pub fn decode_base64(encoded: &str) -> crate::Result<String> {
    let decoded_bytes = B64_STANDARD.decode(encoded).map_err(|e| {
        crate::MonitoringError::Validation(format!("Invalid base64 sequence: {}", e))
    })?;
    String::from_utf8(decoded_bytes).map_err(|e| {
        crate::MonitoringError::Validation(format!("Invalid UTF-8 in base64 decoded data: {}", e))
            .into()
    })
}

/// Sanitize file path to prevent directory traversal attacks
pub fn sanitize_file_path(path: &str) -> crate::Result<String> {
    if path.contains("..") || path.contains("/..") || path.starts_with('/') {
        return Err(crate::MonitoringError::Validation(
            "Invalid file path: directory traversal not allowed".to_string(),
        )
        .into());
    }

    let sanitized = path.replace('\\', "/");
    Ok(sanitized)
}

/// Format duration in human-readable format
pub fn format_duration(duration_ms: f64) -> String {
    if duration_ms < 1000.0 {
        format!("{:.1}ms", duration_ms)
    } else if duration_ms < 60_000.0 {
        format!("{:.1}s", duration_ms / 1000.0)
    } else {
        format!("{:.1}m", duration_ms / 60_000.0)
    }
}

/// Calculate exponential backoff delay for retries
pub fn calculate_backoff_delay(attempt: u32, base_delay_ms: u64, max_delay_ms: u64) -> u64 {
    let delay = base_delay_ms * 2_u64.pow(attempt.min(10)); // Cap at 2^10 to prevent overflow
    delay.min(max_delay_ms)
}

/// Truncate string to maximum length with ellipsis
pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
