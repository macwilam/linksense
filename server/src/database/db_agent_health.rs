//! Database operations for agent health check data
//!
//! This module handles storage and retrieval of agent health monitoring results.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};
use tracing::debug;

/// Represents health check data for a single agent at a point in time
#[derive(Debug, Clone)]
pub struct AgentHealthCheck {
    pub agent_id: String,
    pub check_timestamp: i64,
    pub period_start: i64,
    pub period_end: i64,
    pub seconds_since_last_push: i64,
    pub expected_entries: i64,
    pub received_entries: i64,
    pub success_ratio: f64,
    pub is_problematic: bool,
}

/// Creates the agent_health_checks table and related indexes
pub fn create_table(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agent_health_checks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            check_timestamp INTEGER NOT NULL,
            period_start INTEGER NOT NULL,
            period_end INTEGER NOT NULL,
            seconds_since_last_push INTEGER NOT NULL,
            expected_entries INTEGER NOT NULL,
            received_entries INTEGER NOT NULL,
            success_ratio REAL NOT NULL,
            is_problematic INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents (agent_id)
        )
        "#,
        [],
    )
    .context("Failed to create agent_health_checks table")?;

    // Create indexes for efficient querying
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agent_health_agent_id ON agent_health_checks(agent_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agent_health_check_timestamp ON agent_health_checks(check_timestamp)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agent_health_is_problematic ON agent_health_checks(is_problematic)",
        [],
    )?;

    debug!("Agent health checks table and indexes created");
    Ok(())
}

/// Stores a health check result in the database
pub fn store_health_check(tx: &Transaction, health_check: &AgentHealthCheck) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO agent_health_checks (
            agent_id,
            check_timestamp,
            period_start,
            period_end,
            seconds_since_last_push,
            expected_entries,
            received_entries,
            success_ratio,
            is_problematic
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            health_check.agent_id,
            health_check.check_timestamp,
            health_check.period_start,
            health_check.period_end,
            health_check.seconds_since_last_push,
            health_check.expected_entries,
            health_check.received_entries,
            health_check.success_ratio,
            health_check.is_problematic as i32,
        ],
    )
    .with_context(|| {
        format!(
            "Failed to store health check for agent: {}",
            health_check.agent_id
        )
    })?;

    Ok(())
}

/// Retrieves the most recent health check results for all agents
#[cfg(test)]
pub fn get_recent_health_checks(conn: &Connection) -> Result<Vec<AgentHealthCheck>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            agent_id,
            check_timestamp,
            period_start,
            period_end,
            seconds_since_last_push,
            expected_entries,
            received_entries,
            success_ratio,
            is_problematic
        FROM agent_health_checks
        WHERE check_timestamp = (
            SELECT MAX(check_timestamp) FROM agent_health_checks
        )
        ORDER BY agent_id
        "#,
    )?;

    let checks = stmt
        .query_map([], |row| {
            Ok(AgentHealthCheck {
                agent_id: row.get(0)?,
                check_timestamp: row.get(1)?,
                period_start: row.get(2)?,
                period_end: row.get(3)?,
                seconds_since_last_push: row.get(4)?,
                expected_entries: row.get(5)?,
                received_entries: row.get(6)?,
                success_ratio: row.get(7)?,
                is_problematic: row.get::<_, i32>(8)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(checks)
}

/// Retrieves problematic agents from the most recent health check
pub fn get_problematic_agents(conn: &Connection) -> Result<Vec<AgentHealthCheck>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            agent_id,
            check_timestamp,
            period_start,
            period_end,
            seconds_since_last_push,
            expected_entries,
            received_entries,
            success_ratio,
            is_problematic
        FROM agent_health_checks
        WHERE check_timestamp = (
            SELECT MAX(check_timestamp) FROM agent_health_checks
        )
        AND is_problematic = 1
        ORDER BY success_ratio ASC, agent_id
        "#,
    )?;

    let checks = stmt
        .query_map([], |row| {
            Ok(AgentHealthCheck {
                agent_id: row.get(0)?,
                check_timestamp: row.get(1)?,
                period_start: row.get(2)?,
                period_end: row.get(3)?,
                seconds_since_last_push: row.get(4)?,
                expected_entries: row.get(5)?,
                received_entries: row.get(6)?,
                success_ratio: row.get(7)?,
                is_problematic: row.get::<_, i32>(8)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(checks)
}

/// Retrieves health check history for a specific agent
#[cfg(test)]
pub fn get_agent_health_history(
    conn: &Connection,
    agent_id: &str,
    limit: usize,
) -> Result<Vec<AgentHealthCheck>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            agent_id,
            check_timestamp,
            period_start,
            period_end,
            seconds_since_last_push,
            expected_entries,
            received_entries,
            success_ratio,
            is_problematic
        FROM agent_health_checks
        WHERE agent_id = ?1
        ORDER BY check_timestamp DESC
        LIMIT ?2
        "#,
    )?;

    let checks = stmt
        .query_map(params![agent_id, limit], |row| {
            Ok(AgentHealthCheck {
                agent_id: row.get(0)?,
                check_timestamp: row.get(1)?,
                period_start: row.get(2)?,
                period_end: row.get(3)?,
                seconds_since_last_push: row.get(4)?,
                expected_entries: row.get(5)?,
                received_entries: row.get(6)?,
                success_ratio: row.get(7)?,
                is_problematic: row.get::<_, i32>(8)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(checks)
}

/// Deletes health check records older than the specified cutoff timestamp
pub fn cleanup_old_data(conn: &Connection, cutoff_time: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM agent_health_checks WHERE check_timestamp < ?1",
        params![cutoff_time],
    )?;

    debug!(
        "Deleted {} old health check records (before timestamp: {})",
        deleted, cutoff_time
    );

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

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
}
