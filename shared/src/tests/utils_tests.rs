//! Tests for utility functions

use crate::utils::{
    calculate_backoff_delay, calculate_checksum, decode_base64, encode_base64, format_duration,
    sanitize_file_path, truncate_string, validate_agent_id, validate_url,
};

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
    use crate::utils::current_timestamp;

    let ts1 = current_timestamp();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let ts2 = current_timestamp();

    // Should be at least 1 second apart
    assert!(ts2 > ts1);
    assert!(ts2 - ts1 >= 1);
}
