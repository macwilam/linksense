//! Utility functions for the network monitoring system
//!
//! This module provides common utility functions used across the agent and server
//! components, including hashing, validation, and encoding utilities.

use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use blake3::Hasher;

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

/// Encode string to base64
pub fn encode_base64(content: &str) -> String {
    B64_STANDARD.encode(content)
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
