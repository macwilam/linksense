//! Utility functions for the network monitoring system
//!
//! This module provides common utility functions used across the agent and server
//! components, including hashing, validation, and data manipulation utilities.

use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use blake3::Hasher;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Calculate BLAKE3 checksum of concatenated configuration files
///
/// Takes agent.toml and tasks.toml contents, concatenates them,
/// and returns a BLAKE3 hash as a hex-encoded string.
pub fn calculate_checksum(agent_toml: &str, tasks_toml: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(agent_toml.as_bytes());
    hasher.update(tasks_toml.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Calculate BLAKE3 checksum of a string
///
/// Returns the hash as a hex-encoded string (64 characters).
pub fn calculate_string_checksum(content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(content.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Calculate BLAKE3 checksum of file contents
///
/// Reads the file and returns its BLAKE3 hash as a hex-encoded string.
pub fn calculate_file_checksum<P: AsRef<Path>>(file_path: P) -> crate::Result<String> {
    let content = fs::read_to_string(file_path)?;
    Ok(calculate_string_checksum(&content))
}

/// Validate agent ID format
///
/// Agent IDs must contain only alphanumeric characters, hyphens, and underscores.
/// They must not be empty and should be reasonable in length.
pub fn validate_agent_id(agent_id: &str) -> crate::Result<()> {
    if agent_id.is_empty() {
        return Err(
            crate::MonitoringError::Validation("Agent ID cannot be empty".to_string()).into(),
        );
    }

    if agent_id.len() > 64 {
        return Err(crate::MonitoringError::Validation(
            "Agent ID cannot be longer than 64 characters".to_string(),
        )
        .into());
    }

    if !agent_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(crate::MonitoringError::Validation(
            "Agent ID can only contain alphanumeric characters, hyphens, and underscores"
                .to_string(),
        )
        .into());
    }

    Ok(())
}

/// Get current Unix timestamp in seconds
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Get current Unix timestamp in milliseconds
pub fn current_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Convert timestamp to ISO 8601 string format
pub fn timestamp_to_iso8601(timestamp: u64) -> String {
    let datetime = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(timestamp);
    // For simplicity, we'll use a basic format. In a real implementation,
    // you might want to use the `chrono` crate for more sophisticated date handling.
    format!("{:?}", datetime)
}

/// Encode string to base64
pub fn encode_base64(content: &str) -> String {
    B64_STANDARD.encode(content)
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

/// Validate URL format and structure
///
/// Performs proper URL parsing to ensure:
/// - URL is syntactically valid
/// - Uses http or https scheme (or just https if `https_only` is true)
/// - Has a valid host
/// - Does not contain embedded credentials (security risk)
///
/// # Arguments
/// * `url_str` - The URL string to validate
/// * `https_only` - If true, only https:// URLs are allowed
///
/// # Returns
/// * `Ok(())` if the URL is valid
/// * `Err` with a descriptive error message if validation fails
pub fn validate_url(url_str: &str, https_only: bool) -> crate::Result<()> {
    use url::Url;

    let parsed = Url::parse(url_str).map_err(|e| {
        crate::MonitoringError::Validation(format!("Invalid URL '{}': {}", url_str, e))
    })?;

    // Check scheme
    let scheme = parsed.scheme();
    if https_only {
        if scheme != "https" {
            return Err(crate::MonitoringError::Validation(format!(
                "URL '{}' must use https:// scheme",
                url_str
            ))
            .into());
        }
    } else if scheme != "http" && scheme != "https" {
        return Err(crate::MonitoringError::Validation(format!(
            "URL '{}' must use http:// or https:// scheme",
            url_str
        ))
        .into());
    }

    // Check for valid host
    if parsed.host().is_none() {
        return Err(crate::MonitoringError::Validation(format!(
            "URL '{}' must have a valid host",
            url_str
        ))
        .into());
    }

    // Security: reject URLs with embedded credentials
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(crate::MonitoringError::Validation(format!(
            "URL '{}' must not contain embedded credentials (use separate authentication)",
            url_str
        ))
        .into());
    }

    Ok(())
}
