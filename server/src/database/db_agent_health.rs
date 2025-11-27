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
