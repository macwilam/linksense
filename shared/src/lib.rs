//! Shared data structures and utilities for the network monitoring system
//!
//! This crate contains common types, configuration structures, and utilities
//! used by both the agent and server components.

pub mod api;
pub mod config;
pub mod defaults;
pub mod metrics;
pub mod utils;

// Re-export commonly used types for convenience
pub use api::{ApiRequest, ApiResponse, ConfigStatus};
pub use config::{AgentConfig, TaskConfig, TaskType};
pub use metrics::{AggregatedMetrics, MetricData};
pub use utils::{calculate_checksum, validate_agent_id};

/// Result type alias used throughout the shared crate
pub type Result<T> = anyhow::Result<T>;

/// Common error types for the monitoring system
#[derive(Debug, thiserror::Error)]
pub enum MonitoringError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Task execution error: {0}")]
    TaskExecution(String),

    #[error("Validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_basic_imports() {
        // Basic smoke test to ensure all modules can be imported
        // More comprehensive tests will be added in Phase 1.2
    }
}
