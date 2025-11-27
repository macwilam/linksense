//! Tests for the bandwidth test state management module

use crate::bandwidth_state::BandwidthTestManager;
use shared::api::BandwidthTestAction;

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

    // With active test
    manager
        .request_test("agent1".to_string(), 1024 * 1024)
        .await;
    let status = manager.get_status().await;
    assert!(status.current_test.is_some());
    assert_eq!(status.current_test.as_ref().unwrap().0, "agent1");
    // Check elapsed time is very small (just started)
    assert!(status.current_test.as_ref().unwrap().1 < 2);

    // With queued tests
    manager
        .request_test("agent2".to_string(), 2 * 1024 * 1024)
        .await;
    manager
        .request_test("agent3".to_string(), 3 * 1024 * 1024)
        .await;
    let status = manager.get_status().await;
    assert_eq!(status.current_test.as_ref().unwrap().0, "agent1");
}

#[tokio::test]
async fn test_bandwidth_complete_test_nonexistent_agent() {
    let manager = BandwidthTestManager::new(120, 300, 30, 60, 30);

    // Complete test for agent not in queue - should not panic
    manager.complete_test("nonexistent").await;

    // Verify status unchanged
    let status = manager.get_status().await;
    assert!(status.current_test.is_none());
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
}
