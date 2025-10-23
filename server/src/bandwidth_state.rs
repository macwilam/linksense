//! Bandwidth test state management for preventing concurrent tests
//!
//! This module manages the state of bandwidth tests to ensure only one test
//! runs at a time per server, coordinating test execution between agents.

use shared::api::{BandwidthTestAction, BandwidthTestResponse};

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Manages bandwidth test state to prevent concurrent tests
#[derive(Clone)]
pub struct BandwidthTestManager {
    state: Arc<RwLock<BandwidthTestState>>,
}

/// Internal state for bandwidth test management
struct BandwidthTestState {
    /// Current test in progress (agent_id and start time)
    current_test: Option<(String, Instant)>,
    /// Queue of agents waiting to run tests (agent_id and request time)
    waiting_queue: Vec<(String, Instant)>,
    /// Test timeout duration
    test_timeout: Duration,
    /// Maximum delay to suggest to agents
    max_delay: Duration,
    /// Base delay for bandwidth queue when no test running
    base_delay_seconds: u64,
    /// Delay for current bandwidth test
    current_test_delay_seconds: u64,
    /// Delay multiplier per queue position
    position_multiplier_seconds: u64,
}

impl BandwidthTestManager {
    /// Create a new bandwidth test manager with configurable timeouts
    pub fn new(
        test_timeout_seconds: u64,
        max_delay_seconds: u64,
        base_delay_seconds: u64,
        current_test_delay_seconds: u64,
        position_multiplier_seconds: u64,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(BandwidthTestState {
                current_test: None,
                waiting_queue: Vec::new(),
                test_timeout: Duration::from_secs(test_timeout_seconds),
                max_delay: Duration::from_secs(max_delay_seconds),
                base_delay_seconds,
                current_test_delay_seconds,
                position_multiplier_seconds,
            })),
        }
    }

    /// Request to start a bandwidth test for the given agent
    pub async fn request_test(
        &self,
        agent_id: String,
        data_size_bytes: u64,
    ) -> BandwidthTestResponse {
        let mut state = self.state.write().await;

        // Clean up any expired tests first
        self.cleanup_expired_tests(&mut state);

        // Check if this agent is already in queue or running a test
        if let Some((current_agent, _)) = &state.current_test {
            if current_agent == &agent_id {
                debug!("Agent {} already running bandwidth test", agent_id);
                return BandwidthTestResponse {
                    status: "success".to_string(),
                    action: BandwidthTestAction::Proceed,
                    delay_seconds: None,
                    data_size_bytes: Some(data_size_bytes),
                };
            }
        }

        // Remove agent from waiting queue if already present
        state.waiting_queue.retain(|(id, _)| id != &agent_id);

        // If no test is currently running, start this one
        if state.current_test.is_none() {
            state.current_test = Some((agent_id.clone(), Instant::now()));
            info!("Starting bandwidth test for agent {}", agent_id);

            BandwidthTestResponse {
                status: "success".to_string(),
                action: BandwidthTestAction::Proceed,
                delay_seconds: None,
                data_size_bytes: Some(data_size_bytes),
            }
        } else {
            // Test is running, add to queue and suggest delay
            let delay_seconds = self.calculate_delay(&state);
            state.waiting_queue.push((agent_id.clone(), Instant::now()));

            info!(
                "Bandwidth test already running, queuing agent {} with {}s delay",
                agent_id, delay_seconds
            );

            BandwidthTestResponse {
                status: "success".to_string(),
                action: BandwidthTestAction::Delay,
                delay_seconds: Some(delay_seconds),
                data_size_bytes: None,
            }
        }
    }

    /// Mark a test as completed for the given agent
    pub async fn complete_test(&self, agent_id: &str) {
        let mut state = self.state.write().await;

        if let Some((current_agent, _)) = &state.current_test {
            if current_agent == agent_id {
                state.current_test = None;
                info!("Completed bandwidth test for agent {}", agent_id);

                // Start next test from queue if available
                if let Some((next_agent, _)) = state.waiting_queue.first().cloned() {
                    state.waiting_queue.remove(0);
                    state.current_test = Some((next_agent.clone(), Instant::now()));
                    info!(
                        "Auto-starting queued bandwidth test for agent {}",
                        next_agent
                    );
                }
            }
        }
    }

    /// Get current test status for monitoring
    pub async fn get_status(&self) -> BandwidthTestStatus {
        let state = self.state.read().await;
        BandwidthTestStatus {
            current_test: state
                .current_test
                .as_ref()
                .map(|(agent, start)| (agent.clone(), start.elapsed().as_secs())),
            queue_length: state.waiting_queue.len(),
        }
    }

    /// Remove timed-out current test and expired queue entries
    ///
    /// Removes current test if it exceeds timeout, and removes waiting queue
    /// entries older than max_delay.
    fn cleanup_expired_tests(&self, state: &mut BandwidthTestState) {
        if let Some((agent, start_time)) = &state.current_test {
            if start_time.elapsed() > state.test_timeout {
                warn!(
                    "Bandwidth test for agent {} timed out after {:?}, removing",
                    agent, state.test_timeout
                );
                state.current_test = None;
            }
        }

        // Remove old entries from waiting queue (older than max_delay)
        let cutoff = Instant::now() - state.max_delay;
        let initial_len = state.waiting_queue.len();
        state.waiting_queue.retain(|(_, time)| *time > cutoff);

        if state.waiting_queue.len() < initial_len {
            debug!(
                "Removed {} expired entries from bandwidth test queue",
                initial_len - state.waiting_queue.len()
            );
        }
    }

    /// Calculate suggested delay for agent based on queue state
    ///
    /// Returns delay in seconds based on whether a test is running and the
    /// agent's position in queue, capped at max_delay.
    fn calculate_delay(&self, state: &BandwidthTestState) -> u32 {
        let base_delay = if state.current_test.is_some() {
            // If test is running, suggest waiting for it to complete plus buffer
            state.current_test_delay_seconds as u32
        } else {
            state.base_delay_seconds as u32
        };

        // Add delay based on queue position
        let queue_delay =
            state.waiting_queue.len() as u32 * state.position_multiplier_seconds as u32;

        let total_delay = base_delay + queue_delay;

        // Cap at max_delay
        std::cmp::min(total_delay, state.max_delay.as_secs() as u32)
    }
}

/// Status information for bandwidth tests
#[derive(Debug)]
pub struct BandwidthTestStatus {
    /// Current running test (agent_id, elapsed_seconds)
    pub current_test: Option<(String, u64)>,
    /// Number of agents in waiting queue
    pub queue_length: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bandwidth_manager_single_test() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        let response = manager
            .request_test("agent1".to_string(), 1024 * 1024)
            .await;
        assert_eq!(response.action, BandwidthTestAction::Proceed);
        assert!(response.data_size_bytes.is_some());

        let status = manager.get_status().await;
        assert!(status.current_test.is_some());
        assert_eq!(status.queue_length, 0);
    }

    #[tokio::test]
    async fn test_bandwidth_manager_concurrent_tests() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        // Start first test
        let response1 = manager
            .request_test("agent1".to_string(), 1024 * 1024)
            .await;
        assert_eq!(response1.action, BandwidthTestAction::Proceed);

        // Try to start second test - should be delayed
        let response2 = manager
            .request_test("agent2".to_string(), 1024 * 1024)
            .await;
        assert_eq!(response2.action, BandwidthTestAction::Delay);
        assert!(response2.delay_seconds.is_some());

        let status = manager.get_status().await;
        assert!(status.current_test.is_some());
        assert_eq!(status.queue_length, 1);
    }

    #[tokio::test]
    async fn test_bandwidth_manager_test_completion() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        // Start first test
        manager
            .request_test("agent1".to_string(), 1024 * 1024)
            .await;

        // Queue second test
        manager
            .request_test("agent2".to_string(), 1024 * 1024)
            .await;

        // Complete first test
        manager.complete_test("agent1").await;

        let status = manager.get_status().await;
        assert!(status.current_test.is_some());
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent2");
        assert_eq!(status.queue_length, 0);
    }

    #[tokio::test]
    async fn test_bandwidth_manager_queue_ordering() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        // Start first test
        manager
            .request_test("agent1".to_string(), 1024 * 1024)
            .await;

        // Queue multiple tests
        manager
            .request_test("agent2".to_string(), 1024 * 1024)
            .await;
        manager
            .request_test("agent3".to_string(), 1024 * 1024)
            .await;
        manager
            .request_test("agent4".to_string(), 1024 * 1024)
            .await;

        // Verify queue length
        let status = manager.get_status().await;
        assert_eq!(status.queue_length, 3);

        // Complete tests and verify FIFO ordering
        manager.complete_test("agent1").await;
        let status = manager.get_status().await;
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent2");

        manager.complete_test("agent2").await;
        let status = manager.get_status().await;
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent3");

        manager.complete_test("agent3").await;
        let status = manager.get_status().await;
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent4");
    }

    #[tokio::test]
    async fn test_bandwidth_manager_get_status_accuracy() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        // Initially empty
        let status = manager.get_status().await;
        assert!(status.current_test.is_none());
        assert_eq!(status.queue_length, 0);

        // With active test
        manager
            .request_test("agent1".to_string(), 1024 * 1024)
            .await;
        let status = manager.get_status().await;
        assert!(status.current_test.is_some());
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent1");
        // Check elapsed time is very small (just started)
        assert!(status.current_test.as_ref().unwrap().1 < 2);
        assert_eq!(status.queue_length, 0);

        // With queued tests
        manager
            .request_test("agent2".to_string(), 2 * 1024 * 1024)
            .await;
        manager
            .request_test("agent3".to_string(), 3 * 1024 * 1024)
            .await;
        let status = manager.get_status().await;
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent1");
        assert_eq!(status.queue_length, 2);
    }

    #[tokio::test]
    async fn test_bandwidth_complete_test_nonexistent_agent() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        // Complete test for agent not in queue - should not panic
        manager.complete_test("nonexistent").await;

        // Verify status unchanged
        let status = manager.get_status().await;
        assert!(status.current_test.is_none());
        assert_eq!(status.queue_length, 0);
    }

    #[tokio::test]
    async fn test_bandwidth_manager_complete_wrong_agent() {
        let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

        // Start test for agent1
        manager
            .request_test("agent1".to_string(), 1024 * 1024)
            .await;

        // Queue agent2
        manager
            .request_test("agent2".to_string(), 1024 * 1024)
            .await;

        // Try to complete agent2 (not current test) - should have no effect
        manager.complete_test("agent2").await;

        // agent1 should still be the current test
        let status = manager.get_status().await;
        assert_eq!(status.current_test.as_ref().unwrap().0, "agent1");
        assert_eq!(status.queue_length, 1);
    }
}
