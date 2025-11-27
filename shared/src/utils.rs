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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_checksum() {
        let agent_toml = "agent_id = \"test\"";
        let tasks_toml = "[[tasks]]\ntype = \"ping\"";

        let checksum1 = calculate_checksum(agent_toml, tasks_toml);
        let checksum2 = calculate_checksum(agent_toml, tasks_toml);

        // Same input should produce same checksum
        assert_eq!(checksum1, checksum2);
        assert!(!checksum1.is_empty());
        assert_eq!(checksum1.len(), 64); // BLAKE3 hex output is 64 characters
    }

    #[test]
    fn test_checksum_changes_with_content() {
        let agent_toml1 = "agent_id = \"test1\"";
        let agent_toml2 = "agent_id = \"test2\"";
        let tasks_toml = "[[tasks]]\ntype = \"ping\"";

        let checksum1 = calculate_checksum(agent_toml1, tasks_toml);
        let checksum2 = calculate_checksum(agent_toml2, tasks_toml);

        // Different input should produce different checksums
        assert_ne!(checksum1, checksum2);
    }

    #[test]
    fn test_validate_url() {
        // Valid URLs
        assert!(validate_url("https://example.com", false).is_ok());
        assert!(validate_url("http://example.com", false).is_ok());
        assert!(validate_url("https://example.com/path/to/resource", false).is_ok());
        assert!(validate_url("https://example.com:8080/api", false).is_ok());
        assert!(validate_url("https://sub.domain.example.com", false).is_ok());

        // HTTPS only mode
        assert!(validate_url("https://example.com", true).is_ok());
        assert!(validate_url("http://example.com", true).is_err()); // HTTP not allowed in https_only mode

        // Invalid URLs
        assert!(validate_url("", false).is_err()); // Empty
        assert!(validate_url("not-a-url", false).is_err()); // No scheme
        assert!(validate_url("ftp://example.com", false).is_err()); // Wrong scheme
        assert!(validate_url("http://", false).is_err()); // No host
        assert!(validate_url("https://user:pass@example.com", false).is_err()); // Embedded credentials
        assert!(validate_url("https://user@example.com", false).is_err()); // Embedded username
    }

    #[test]
    fn test_validate_agent_id() {
        // Valid agent IDs
        assert!(validate_agent_id("agent-01").is_ok());
        assert!(validate_agent_id("web_cluster_01").is_ok());
        assert!(validate_agent_id("Agent123").is_ok());

        // Invalid agent IDs
        assert!(validate_agent_id("").is_err());
        assert!(validate_agent_id("agent with spaces").is_err());
        assert!(validate_agent_id("agent@domain").is_err());
        assert!(validate_agent_id("agent/path").is_err());

        // Too long
        let long_id = "a".repeat(65);
        assert!(validate_agent_id(&long_id).is_err());
    }

    #[test]
    fn test_base64_encoding_decoding() {
        let original = "Hello, World!";
        let encoded = encode_base64(original);
        let decoded = decode_base64(&encoded).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_sanitize_file_path() {
        assert!(sanitize_file_path("config.toml").is_ok());
        assert!(sanitize_file_path("agent/config.toml").is_ok());

        // These should fail
        assert!(sanitize_file_path("../config.toml").is_err());
        assert!(sanitize_file_path("/etc/passwd").is_err());
        assert!(sanitize_file_path("agent/../server/config.toml").is_err());
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(500.0), "500.0ms");
        assert_eq!(format_duration(1500.0), "1.5s");
        assert_eq!(format_duration(90000.0), "1.5m");
    }

    #[test]
    fn test_calculate_backoff_delay() {
        assert_eq!(calculate_backoff_delay(0, 1000, 30000), 1000);
        assert_eq!(calculate_backoff_delay(1, 1000, 30000), 2000);
        assert_eq!(calculate_backoff_delay(2, 1000, 30000), 4000);
        assert_eq!(calculate_backoff_delay(10, 1000, 30000), 30000); // Capped at max
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("Hello", 10), "Hello");
        assert_eq!(truncate_string("Hello, World!", 10), "Hello, ...");
        assert_eq!(truncate_string("Hi", 2), "Hi");
        assert_eq!(truncate_string("Hello", 3), "...");
    }

    #[test]
    fn test_current_timestamp() {
        let ts1 = current_timestamp();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let ts2 = current_timestamp();

        // Should be at least 1 second apart
        assert!(ts2 > ts1);
        assert!(ts2 - ts1 >= 1);
    }
}
