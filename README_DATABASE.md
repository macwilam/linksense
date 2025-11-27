# Database Schema Documentation

## Overview

LinkSense uses SQLite for local persistence on agents and centralized storage on server. Each task type has dedicated tables optimized for its specific metrics.

## Design: Separate Tables Per Task Type

**Rationale:**
- **Type safety** - Each metric has exact required columns, no sparse nullable fields
- **Query performance** - Narrow tables with targeted indexes, no unused column overhead
- **Storage efficiency** - No wasted space on NULL values
- **Direct SQL queries** - Native aggregation without JSON parsing
- **BI tool integration** - Standard column metadata for visualization tools

**Trade-off:** More tables for better performance and developer experience.

## Architecture Pattern

### Agent Database (`{config_dir}/agent_metrics.db`)

**Two-tier storage:**
1. **Raw metrics** - Individual measurements (`raw_metric_*` tables)
2. **Aggregated metrics** - 60-second summaries (`agg_metric_*` tables)

Benefits:
- High-resolution debugging data locally
- Bandwidth reduction (send aggregated only)
- Resilient to server downtime

### Server Database (`{data_dir}/server_metrics.db`)

**Receives aggregated metrics only:**
- Same table structure as agent aggregated tables
- Additional columns: `agent_id`, `received_at`
- `agents` table tracks registration and config versions
- `config_errors` table logs agent-reported issues

## Data Flow

```
Agent: Task → raw_metric_* → [60s aggregation] → agg_metric_*
                                                        ↓
                                              POST /api/v1/metrics
                                                        ↓
Server:                                       agg_metric_* (with agent_id)
```

## Table Structure Example (Ping)

**Agent:**
```sql
raw_metric_ping:       id, task_name, timestamp, rtt_ms, success, error
agg_metric_ping:       id, task_name, period_start, period_end, sample_count,
                       avg_latency_ms, max_latency_ms, min_latency_ms,
                       packet_loss_percent, successful_pings, failed_pings
```

**Server:**
```sql
agg_metric_ping:       [same as agent] + agent_id, received_at
agents:                id, agent_id, first_seen, last_seen, last_config_checksum
config_errors:         id, agent_id, timestamp_utc, error_message, received_at
```

Pattern applies to all task types: `ping`, `tcp`, `tls`, `http`, `http_content`, `dns`, `bandwidth`, `sql_query`.

## Performance

**Indexes:**
- Agent: `(task_name, timestamp)` for time-range queries
- Server: `(agent_id)`, `(period_start)`, `(task_name)` for multi-agent analytics

**Storage savings:**
- Agent monitoring 10 targets every 10 seconds for 7 days
- Raw: 86,400 rows/day → Aggregated: 1,440 rows/day = **98.3% reduction**
- Server receives only aggregated data = **60x bandwidth reduction**

## Configuration

```sql
PRAGMA journal_mode=WAL;        -- Crash recovery and better concurrency
PRAGMA foreign_keys=ON;         -- Referential integrity (server only)
PRAGMA busy_timeout=5000;       -- Agent: 5s, Server: 30s
```

## Data Retention

**Agent:** `local_data_retention_days` in `agent.toml`
- Deletes old raw and aggregated metrics
- Runs daily cleanup + VACUUM

**Server:** `data_retention_days` in `server.toml`
- Deletes old aggregated metrics, config errors, inactive agents
- Runs daily cleanup + VACUUM

## Implementation Files

- `agent/src/database.rs` - Agent schema and operations
- `server/src/database.rs` - Server schema and operations
- `shared/src/metrics.rs` - Metric data structures

## Reference

For complete table schemas, see source files. Tables are created dynamically based on configured task types.
