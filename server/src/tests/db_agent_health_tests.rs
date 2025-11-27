//! Tests for the agent health check database operations

use crate::database::db_agent_health::{
    cleanup_old_data, create_table, get_agent_health_history, get_problematic_agents,
    get_recent_health_checks, store_health_check, AgentHealthCheck,
};
use rusqlite::{params, Connection};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();

    // Create agents table (required for foreign key)
    conn.execute(
        r#"
        CREATE TABLE agents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT UNIQUE NOT NULL,
            first_seen INTEGER NOT NULL,
            last_seen INTEGER NOT NULL,
            last_config_checksum TEXT,
            total_metrics_received INTEGER DEFAULT 0,
            created_at INTEGER DEFAULT (strftime('%s', 'now'))
        )
        "#,
        [],
    )
    .unwrap();

    conn.execute("PRAGMA foreign_keys=ON", []).unwrap();

    create_table(&conn).unwrap();
    conn
}

fn insert_test_agent(conn: &Connection, agent_id: &str) {
    conn.execute(
        "INSERT INTO agents (agent_id, first_seen, last_seen) VALUES (?1, ?2, ?3)",
        params![agent_id, 1000, 2000],
    )
    .unwrap();
}

#[test]
fn test_create_table() {
    let conn = setup_test_db();

    // Verify table exists
    let table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='agent_health_checks')",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert!(table_exists);
}

#[test]
fn test_store_and_retrieve_health_check() {
    let mut conn = setup_test_db();
    insert_test_agent(&conn, "test-agent");

    let health_check = AgentHealthCheck {
        agent_id: "test-agent".to_string(),
        check_timestamp: 3000,
        period_start: 2700,
        period_end: 3000,
        seconds_since_last_push: 1000,
        expected_entries: 100,
        received_entries: 85,
        success_ratio: 0.85,
        is_problematic: true,
    };

    let tx = conn.transaction().unwrap();
    store_health_check(&tx, &health_check).unwrap();
    tx.commit().unwrap();

    let checks = get_recent_health_checks(&conn).unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].agent_id, "test-agent");
    assert_eq!(checks[0].success_ratio, 0.85);
    assert!(checks[0].is_problematic);
}

#[test]
fn test_get_problematic_agents() {
    let mut conn = setup_test_db();
    insert_test_agent(&conn, "healthy-agent");
    insert_test_agent(&conn, "sick-agent");

    let healthy = AgentHealthCheck {
        agent_id: "healthy-agent".to_string(),
        check_timestamp: 3000,
        period_start: 2700,
        period_end: 3000,
        seconds_since_last_push: 100,
        expected_entries: 100,
        received_entries: 95,
        success_ratio: 0.95,
        is_problematic: false,
    };

    let sick = AgentHealthCheck {
        agent_id: "sick-agent".to_string(),
        check_timestamp: 3000,
        period_start: 2700,
        period_end: 3000,
        seconds_since_last_push: 500,
        expected_entries: 100,
        received_entries: 50,
        success_ratio: 0.50,
        is_problematic: true,
    };

    let tx = conn.transaction().unwrap();
    store_health_check(&tx, &healthy).unwrap();
    store_health_check(&tx, &sick).unwrap();
    tx.commit().unwrap();

    let problematic = get_problematic_agents(&conn).unwrap();
    assert_eq!(problematic.len(), 1);
    assert_eq!(problematic[0].agent_id, "sick-agent");
}

#[test]
fn test_get_agent_health_history() {
    let mut conn = setup_test_db();
    insert_test_agent(&conn, "test-agent");

    // Insert multiple health checks at different times
    for i in 0..5 {
        let health_check = AgentHealthCheck {
            agent_id: "test-agent".to_string(),
            check_timestamp: 1000 + (i * 100),
            period_start: 900 + (i * 100),
            period_end: 1000 + (i * 100),
            seconds_since_last_push: 50,
            expected_entries: 100,
            received_entries: 90 + i,
            success_ratio: (90 + i) as f64 / 100.0,
            is_problematic: false,
        };

        let tx = conn.transaction().unwrap();
        store_health_check(&tx, &health_check).unwrap();
        tx.commit().unwrap();
    }

    let history = get_agent_health_history(&conn, "test-agent", 3).unwrap();
    assert_eq!(history.len(), 3);
    // Should be ordered by timestamp DESC
    assert_eq!(history[0].check_timestamp, 1400);
    assert_eq!(history[1].check_timestamp, 1300);
    assert_eq!(history[2].check_timestamp, 1200);
}

#[test]
fn test_cleanup_old_data() {
    let mut conn = setup_test_db();
    insert_test_agent(&conn, "test-agent");

    // Insert old and recent health checks
    let old_check = AgentHealthCheck {
        agent_id: "test-agent".to_string(),
        check_timestamp: 1000,
        period_start: 700,
        period_end: 1000,
        seconds_since_last_push: 50,
        expected_entries: 100,
        received_entries: 90,
        success_ratio: 0.90,
        is_problematic: false,
    };

    let recent_check = AgentHealthCheck {
        agent_id: "test-agent".to_string(),
        check_timestamp: 5000,
        period_start: 4700,
        period_end: 5000,
        seconds_since_last_push: 50,
        expected_entries: 100,
        received_entries: 95,
        success_ratio: 0.95,
        is_problematic: false,
    };

    let tx = conn.transaction().unwrap();
    store_health_check(&tx, &old_check).unwrap();
    store_health_check(&tx, &recent_check).unwrap();
    tx.commit().unwrap();

    // Delete records older than timestamp 3000
    let deleted = cleanup_old_data(&conn, 3000).unwrap();
    assert_eq!(deleted, 1);

    let history = get_agent_health_history(&conn, "test-agent", 10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].check_timestamp, 5000);
}

#[test]
fn test_get_recent_health_checks_multiple_agents() {
    let mut conn = setup_test_db();
    insert_test_agent(&conn, "agent-1");
    insert_test_agent(&conn, "agent-2");

    // Insert old checks
    for agent in &["agent-1", "agent-2"] {
        let check = AgentHealthCheck {
            agent_id: agent.to_string(),
            check_timestamp: 1000,
            period_start: 700,
            period_end: 1000,
            seconds_since_last_push: 50,
            expected_entries: 100,
            received_entries: 90,
            success_ratio: 0.90,
            is_problematic: false,
        };

        let tx = conn.transaction().unwrap();
        store_health_check(&tx, &check).unwrap();
        tx.commit().unwrap();
    }

    // Insert recent checks
    for agent in &["agent-1", "agent-2"] {
        let check = AgentHealthCheck {
            agent_id: agent.to_string(),
            check_timestamp: 5000,
            period_start: 4700,
            period_end: 5000,
            seconds_since_last_push: 50,
            expected_entries: 100,
            received_entries: 95,
            success_ratio: 0.95,
            is_problematic: false,
        };

        let tx = conn.transaction().unwrap();
        store_health_check(&tx, &check).unwrap();
        tx.commit().unwrap();
    }

    let recent = get_recent_health_checks(&conn).unwrap();
    assert_eq!(recent.len(), 2);
    // All should have the most recent timestamp
    assert!(recent.iter().all(|c| c.check_timestamp == 5000));
}
