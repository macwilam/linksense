# SQL Query Task

The **SQL Query** task monitors database health and query performance by executing SQL statements and measuring their response time. It supports two execution modes: **Value mode** for extracting numeric metrics and **JSON mode** for capturing full query results.

**Note**: This task requires the `sql-tasks` feature flag to be enabled during compilation.

## Implementation Details

### Crate: `rsql_drivers`
**Link**: [rsql_drivers on crates.io](https://crates.io/crates/rsql_drivers)

This task uses **`rsql_drivers`**, a Rust SQL driver abstraction layer.

**Key Characteristics**:
- **Async/Non-blocking**: Built on Tokio, no thread blocking during queries
- **Multi-Database Support**: PostgreSQL, MySQL, SQLite, and more via unified interface
- **Runtime Query Execution**: Queries are strings executed at runtime
- **Result Set Access**: Full access to query results (columns, rows, values)
- **Type-Safe Values**: `Value` enum provides safe access to database values

**Consequences**:
- ✅ **Database Flexibility**: Single codebase supports multiple database engines
- ✅ **Async Performance**: Non-blocking queries allow high concurrency
- ✅ **Full Result Access**: Can extract values and serialize to JSON
- ✅ **Pure Rust**: No C dependencies, easier cross-compilation
- ⚠️ **Feature Compilation**: Must compile with `sql-tasks` feature
- ⚠️ **No Query Validation**: SQL syntax errors only detected at runtime

**Default Database Drivers**:

By default, only the following database drivers are compiled:
```toml
# In workspace Cargo.toml
rsql_drivers = { version = "0.19.2", default-features = false, features = ["driver-postgresql", "driver-mysql", "driver-mariadb"] }
```

To add support for additional databases (e.g., SQLite), modify the `rsql_drivers` line in the workspace `Cargo.toml`:
```toml
# Example: Adding SQLite support
rsql_drivers = { version = "0.19.2", default-features = false, features = [
    "driver-postgresql",
    "driver-mysql", 
    "driver-mariadb",
    "driver-sqlite"
] }
```

**Available driver features**: `driver-postgresql`, `driver-mysql`, `driver-mariadb`, `driver-sqlite`, `driver-duckdb`, `driver-snowflake`, and others. See [rsql_drivers documentation](https://docs.rs/rsql_drivers) for the full list.

**Note**: Adding `driver-sqlite` may cause compilation conflicts with the project's `rusqlite` dependency due to different `libsqlite3-sys` versions. If you encounter this, ensure compatible versions are used.

### Supported Databases

**PostgreSQL** (`database_type = "postgresql"`):
- **Protocol**: PostgreSQL wire protocol (native)
- **Versions**: PostgreSQL 9.5+
- **SSL/TLS**: Supported
- **Note**: Use `"postgresql"` (not `"postgres"`) as the database_type

**MySQL** (`database_type = "mysql"`):
- **Protocol**: MySQL wire protocol (native)
- **Versions**: MySQL 5.6+
- **SSL/TLS**: Supported

**MariaDB** (`database_type = "mariadb"` or `"mysql"`):
- **Protocol**: MySQL-compatible wire protocol
- **Versions**: MariaDB 10.2+
- **SSL/TLS**: Supported
- **Note**: Can use either `"mariadb"` or `"mysql"` as database_type

**SQLite** (`database_type = "sqlite"`):
- **File Access**: Direct file-based database access
- **Versions**: SQLite 3.x
- **Note**: Local file only, no client-server
- ⚠️ **Not enabled by default**: Requires adding `driver-sqlite` feature (see "Default Database Drivers" above)

**Databases NOT Enabled by Default**:
- **SQLite**: Add `driver-sqlite` feature to enable
- **DuckDB**: Add `driver-duckdb` feature to enable
- **Snowflake**: Add `driver-snowflake` feature to enable

**Databases NOT Supported**:
- ❌ **Oracle**: Not supported
- ❌ **SQL Server (MSSQL)**: Not supported
- ❌ **MongoDB**: Not a SQL database

### Query Execution Modes

The SQL task supports two execution modes:

#### Value Mode (Default)
Extracts a single numeric value from the first row, first column of the query result. Ideal for monitoring metrics like counts, gauges, and numeric indicators.

```
Query: SELECT COUNT(*) FROM users
Result: row_count=1, value=42.0
```

#### JSON Mode
Returns the full query result as a JSON array of objects. Ideal for capturing structured data, recent records, or diagnostic information.

```
Query: SELECT id, name FROM users LIMIT 3
Result: [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}, {"id": 3, "name": "Charlie"}]
```

### Query Execution Flow

```
1. Parse database_url and create connection
2. Authenticate with database server
3. Send SQL query string to database
4. Wait for response (async, with timeout)
5. Based on mode:
   - Value mode: Extract first cell as numeric, count all rows
   - JSON mode: Collect rows up to max_rows, convert to JSON
6. Apply size limits (JSON mode) and truncate if needed
7. Close connection
```

**Timing Measurement**:
- `total_time_ms`: Complete round-trip time including:
  - Network latency to database
  - Query parsing and planning
  - Query execution on database
  - Result set transfer
  - Value extraction or JSON serialization

### Connection Handling

Each task execution:
1. Creates new database connection
2. Executes single query
3. Closes connection

**Why Not Connection Pooling?**
- Tasks run infrequently (≥60s intervals)
- Connection overhead is part of health check (tests full connection path)
- Simpler implementation, no pool management
- Avoids stale connection issues

### Security Considerations

**Credential Storage**:
```toml
# Credentials in plaintext in config file
database_url = "postgresql://user:password@host/db"
```

**Recommendations**:
1. Use read-only database accounts
2. Restrict config file permissions (`chmod 600`)
3. Use network firewalls to limit database access
4. Consider secrets management for production

**SQL Injection Protection**:
- Static queries only (no user input)
- No string interpolation in queries

## Configuration

### Basic Configuration (Value Mode)

```toml
[[tasks]]
type = "sql_query"
name = "Database Health Check"
schedule_seconds = 60
query = "SELECT 1"
database_url = "localhost:5432/mydb"
database_type = "postgresql"
username = "monitor"
password = "secret"
timeout_seconds = 30
# mode = "value"  # Default, can be omitted
```

### JSON Mode Configuration

```toml
[[tasks]]
type = "sql_query"
name = "Recent Errors Log"
schedule_seconds = 300
query = "SELECT id, message, created_at FROM errors ORDER BY created_at DESC LIMIT 50"
database_url = "localhost:5432/mydb"
database_type = "postgresql"
username = "monitor"
password = "secret"
mode = "json"
max_json_size_bytes = 131072    # 128 KB limit
max_rows = 100                   # Maximum rows to return
timeout_seconds = 60
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"sql_query"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between queries (≥60 seconds, enforced) |
| `query` | string | ✅ | - | SQL query to execute |
| `database_url` | string | ✅ | - | Database host/path (without protocol prefix) |
| `database_type` | string | ✅ | - | Database type: `postgresql`, `mysql`, `mariadb`, `sqlite` |
| `username` | string | ❌ | - | Database username |
| `password` | string | ❌ | - | Database password |
| `timeout_seconds` | integer | ❌ | 30 | Query timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering |
| `mode` | string | ❌ | `"value"` | Execution mode: `"value"` or `"json"` |
| `max_json_size_bytes` | integer | ❌ | 65536 | Maximum JSON result size (JSON mode, max: 1MB) |
| `max_rows` | integer | ❌ | 1000 | Maximum rows to return (JSON mode, max: 10000) |

### Database Connection URLs

#### PostgreSQL
```toml
database_url = "localhost:5432/database_name"
database_type = "postgresql"
username = "user"
password = "password"
```

#### MySQL
```toml
database_url = "localhost:3306/database_name"
database_type = "mysql"
username = "user"
password = "password"
```

#### MariaDB
```toml
database_url = "localhost:3306/database_name"
database_type = "mariadb"
username = "user"
password = "password"
```

#### SQLite
```toml
database_url = "/path/to/database.db"
database_type = "sqlite"
```

### Configuration Examples

#### Simple Health Check (Value Mode)
```toml
[[tasks]]
type = "sql_query"
name = "Postgres Health"
schedule_seconds = 60
query = "SELECT 1"
database_url = "localhost:5432/postgres"
database_type = "postgresql"
username = "monitor"
password = "secret"
```

#### Monitor Active Connections (Value Mode)
```toml
[[tasks]]
type = "sql_query"
name = "Active Database Connections"
schedule_seconds = 60
query = "SELECT COUNT(*) FROM pg_stat_activity WHERE state = 'active'"
database_url = "localhost:5432/postgres"
database_type = "postgresql"
username = "admin"
password = "secret"
mode = "value"
```

#### Monitor Table Row Count (Value Mode)
```toml
[[tasks]]
type = "sql_query"
name = "Orders Table Size"
schedule_seconds = 300
query = "SELECT COUNT(*) FROM orders"
database_url = "db.prod.com:5432/sales"
database_type = "postgresql"
username = "readonly"
password = "pass"
timeout_seconds = 60
```

#### Recent Error Logs (JSON Mode)
```toml
[[tasks]]
type = "sql_query"
name = "Recent Application Errors"
schedule_seconds = 300
query = "SELECT id, level, message, created_at FROM logs WHERE level = 'ERROR' ORDER BY created_at DESC LIMIT 20"
database_url = "localhost:5432/app"
database_type = "postgresql"
username = "monitor"
password = "secret"
mode = "json"
max_json_size_bytes = 65536
max_rows = 20
```

#### Current User Sessions (JSON Mode)
```toml
[[tasks]]
type = "sql_query"
name = "Active User Sessions"
schedule_seconds = 120
query = "SELECT user_id, ip_address, started_at FROM sessions WHERE expires_at > NOW() ORDER BY started_at DESC"
database_url = "localhost:3306/webapp"
database_type = "mysql"
username = "monitor"
password = "secret"
mode = "json"
max_rows = 100
```

#### Database Lock Monitoring (JSON Mode)
```toml
[[tasks]]
type = "sql_query"
name = "Database Locks"
schedule_seconds = 60
query = """
SELECT
  pid,
  usename,
  query,
  state,
  wait_event_type,
  wait_event
FROM pg_stat_activity
WHERE wait_event IS NOT NULL
"""
database_url = "localhost:5432/postgres"
database_type = "postgresql"
username = "admin"
password = "secret"
mode = "json"
max_rows = 50
timeout_seconds = 10
```

#### Replica Lag Detection (Value Mode)
```toml
# Primary database
[[tasks]]
type = "sql_query"
name = "Primary - Latest Order ID"
schedule_seconds = 300
query = "SELECT MAX(id) FROM orders"
database_url = "primary.db.com:5432/sales"
database_type = "postgresql"
username = "user"
password = "pass"
target_id = "db-primary"

# Read replica
[[tasks]]
type = "sql_query"
name = "Replica - Latest Order ID"
schedule_seconds = 300
query = "SELECT MAX(id) FROM orders"
database_url = "replica.db.com:5432/sales"
database_type = "postgresql"
username = "user"
password = "pass"
target_id = "db-replica"
```

## Metrics

### Raw Metrics (`raw_metric_sql_query`)

Captured for each individual SQL query execution:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when query was executed |
| `total_time_ms` | REAL | Query execution time (ms) - NULL if failed |
| `row_count` | INTEGER | Number of rows returned - NULL if failed |
| `success` | BOOLEAN | Whether query succeeded (1) or failed (0) |
| `error` | TEXT | Error message if query failed (NULL on success) |
| `target_id` | TEXT | Optional target identifier from task configuration |
| `mode` | TEXT | Execution mode: `"value"` or `"json"` |
| `value` | REAL | Extracted numeric value (Value mode only) |
| `json_result` | TEXT | JSON result string (JSON mode only) |
| `json_truncated` | BOOLEAN | Whether JSON was truncated due to size limit |
| `column_count` | INTEGER | Number of columns in the result |

### Aggregated Metrics (`agg_metric_sql_query`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of queries in period |
| `success_rate_percent` | REAL | Percentage of successful queries (0-100) |
| `avg_total_time_ms` | REAL | Mean query execution time |
| `max_total_time_ms` | REAL | Maximum execution time observed |
| `avg_row_count` | REAL | Mean number of rows returned |
| `max_row_count` | INTEGER | Maximum rows returned in any query |
| `successful_queries` | INTEGER | Count of successful queries |
| `failed_queries` | INTEGER | Count of failed queries |
| `target_id` | TEXT | Optional target identifier |
| `avg_value` | REAL | Average extracted value (Value mode only) |
| `min_value` | REAL | Minimum extracted value (Value mode only) |
| `max_value` | REAL | Maximum extracted value (Value mode only) |
| `json_truncated_count` | INTEGER | Count of truncated JSON responses |

### Metrics Interpretation

#### Query Performance
- **< 50ms**: Excellent (indexed lookup, simple query)
- **50-200ms**: Good (moderate complexity or small joins)
- **200-1000ms**: Acceptable (complex reports or aggregations)
- **1-5s**: Slow (needs optimization)
- **> 5s**: Very slow (critical performance issue)

#### Value Mode Metrics
- **avg_value**: Track trends in numeric metrics over time
- **min_value/max_value**: Identify outliers and anomalies
- **value = NULL**: Query returned non-numeric or empty result

#### JSON Mode Metrics
- **json_truncated = true**: Result exceeded size limit, some rows omitted
- **json_truncated_count**: Number of times truncation occurred in period

#### Success Rate
- **100%**: Healthy database connectivity
- **95-99%**: Occasional timeouts or connection issues
- **< 90%**: Database availability problems
- **0%**: Database unreachable or credentials invalid

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_total_time_ms > 1000
  - max_total_time_ms > 5000
  - success_rate_percent < 95
  - json_truncated_count > 0 (may indicate need for larger limit)

CRITICAL:
  - avg_total_time_ms > 5000
  - successful_queries == 0 (for 10 minutes)
  - success_rate_percent < 80
```

## Design Philosophy

### What It Does

Executes SQL queries against databases and measures:
- **Query Execution Success**: Can the database execute queries?
- **Response Time**: How long do queries take to complete?
- **Result Values**: What values are returned? (Value mode)
- **Result Data**: What records exist? (JSON mode)
- **Database Availability**: Is the database reachable and responsive?

### Value Mode Use Cases

1. **Metric Collection**: Extract numeric KPIs from databases
   - Active user count
   - Queue depth
   - Pending orders
   - Error counts

2. **Threshold Monitoring**: Track values for alerting
   - Disk usage percentage
   - Replication lag seconds
   - Connection count

3. **Performance Gauges**: Monitor database internals
   - Cache hit ratio
   - Query execution time
   - Lock wait time

### JSON Mode Use Cases

1. **Diagnostic Data**: Capture detailed information for troubleshooting
   - Recent error messages
   - Slow query log entries
   - Active sessions

2. **Audit Trail**: Record state snapshots
   - Current user sessions
   - Pending transactions
   - Configuration values

3. **Health Details**: Capture structured health data
   - Database lock information
   - Replication status
   - Table statistics

### Limitations

- **Feature Flag Required**: Must compile with `--features sql-tasks`
- **Read-Only Recommended**: Only SELECT queries should be used
- **Minimum Schedule**: Must run ≥60 seconds apart (enforced)
- **Credentials in Config**: Database passwords stored in configuration files
- **Single Query per Task**: Each task executes one query
- **JSON Size Limit**: Maximum 1MB for JSON results
- **No Transaction Control**: Queries auto-commit

## Performance Characteristics

### Resource Usage

**Agent**:
- **CPU**: Low (1-3% during query execution)
- **Memory**: 
  - Value mode: Minimal (~1-5 MB)
  - JSON mode: ~(row_count * row_size) + serialization overhead
- **Network**: Minimal (query + result set)
- **Disk**: Batch writes to SQLite

**Database Server**:
- **CPU**: Depends on query complexity
- **Memory**: Query execution buffer
- **I/O**: Depends on query (indexes vs. full table scans)

### Execution Time
- **Simple queries** (`SELECT 1`): 5-50ms
- **Indexed lookups**: 10-100ms
- **Aggregations/joins**: 100-1000ms
- **Complex reports**: 1-10s
- **Timeout**: Configurable (default 30s, properly enforced)

### JSON Mode Considerations
- Large result sets consume more memory
- `max_rows` prevents memory exhaustion
- `max_json_size_bytes` limits storage/transmission size
- Truncation uses binary search for efficiency

## Security Considerations

### Credential Management

**⚠️ IMPORTANT**: Database credentials are stored in plaintext in configuration files!

**Best Practices**:

1. **Use Read-Only Accounts**:
   ```sql
   -- PostgreSQL: Create monitoring user with SELECT-only access
   CREATE USER monitoring WITH PASSWORD 'secure-password';
   GRANT CONNECT ON DATABASE mydb TO monitoring;
   GRANT SELECT ON ALL TABLES IN SCHEMA public TO monitoring;

   -- MySQL: Create monitoring user
   CREATE USER 'monitoring'@'%' IDENTIFIED BY 'secure-password';
   GRANT SELECT ON mydb.* TO 'monitoring'@'%';
   ```

2. **Restrict Configuration File Permissions**:
   ```bash
   chmod 600 /path/to/tasks.toml
   chown monitoring-agent:monitoring-agent /path/to/tasks.toml
   ```

3. **Network Isolation**:
   - Use firewall rules to restrict agent → database connections
   - Use VPN/private networks for production databases

### Query Safety

**Always use SELECT queries**:
```toml
# ✅ GOOD: Read-only query
query = "SELECT COUNT(*) FROM users"

# ❌ BAD: Modifying query (avoid!)
query = "DELETE FROM old_logs WHERE created_at < NOW() - INTERVAL '90 days'"
```

### JSON Mode Data Sensitivity

Be cautious about what data is captured in JSON mode:
- Avoid capturing passwords or tokens
- Consider data retention policies
- JSON results are stored in agent's local database

## Troubleshooting

### Common Issues

#### "Connection timeout" Errors
**Symptom**: Queries consistently timeout
**Causes**:
- Database server unreachable
- Firewall blocking connection
- Database under heavy load
**Solutions**:
```bash
# Test connection manually
psql -h hostname -U username -d database -c "SELECT 1"

# Check firewall
telnet hostname 5432

# Increase timeout
timeout_seconds = 60
```

#### "Authentication failed" Errors
**Symptom**: Permission denied or auth failures
**Causes**:
- Incorrect username/password
- User doesn't have SELECT permission
**Solutions**:
```sql
-- PostgreSQL: Check user permissions
SELECT * FROM pg_roles WHERE rolname = 'monitoring';

-- MySQL: Check user grants
SHOW GRANTS FOR 'monitoring'@'%';
```

#### Value Mode Returns NULL
**Symptom**: `value` field is NULL despite successful query
**Causes**:
- Query returns non-numeric data in first cell
- Query returns empty result set
- First column contains NULL
- Unsupported data type (e.g., DATE, TIME, UUID)
**Solutions**:
```toml
# Ensure query returns numeric in first column
query = "SELECT COUNT(*)::float FROM users"

# Or cast to numeric
query = "SELECT CAST(value AS DECIMAL) FROM metrics LIMIT 1"
```

**Note on MySQL/MariaDB**: Aggregate functions like `SUM()` and `AVG()` return DECIMAL type values, which are properly converted to numeric values. Integer columns work correctly with these aggregates.

#### JSON Truncation
**Symptom**: `json_truncated = true` frequently
**Causes**:
- Result set too large for configured limit
**Solutions**:
```toml
# Increase size limit (max 1MB)
max_json_size_bytes = 524288  # 512 KB

# Or reduce rows in query
query = "SELECT ... LIMIT 50"  # Instead of LIMIT 1000

# Or increase max_rows and size together
max_rows = 500
max_json_size_bytes = 262144  # 256 KB
```

### Debugging Tips

**Test query manually:**
```bash
# PostgreSQL
psql -h hostname -U username -d database -c "SELECT COUNT(*) FROM users" -c "\timing"

# MySQL
mysql -h hostname -u username -p -D database -e "SELECT COUNT(*) FROM users"
```

**Check Value extraction:**
```sql
-- Ensure first column is numeric
SELECT pg_typeof(column_name) FROM table LIMIT 1;
```

**Analyze JSON size:**
```sql
-- Estimate JSON size before configuring
SELECT LENGTH(json_agg(t)::text)
FROM (SELECT * FROM table LIMIT 100) t;
```

## Best Practices

1. **Use Value Mode for Metrics**:
   ```toml
   [[tasks]]
   name = "Active Connections"
   mode = "value"
   query = "SELECT COUNT(*) FROM pg_stat_activity WHERE state = 'active'"
   schedule_seconds = 60
   ```

2. **Use JSON Mode for Diagnostics**:
   ```toml
   [[tasks]]
   name = "Current Locks"
   mode = "json"
   query = "SELECT * FROM pg_locks WHERE granted = false"
   schedule_seconds = 300
   max_rows = 50
   ```

3. **Set Appropriate Size Limits**:
   ```toml
   # Small diagnostic data
   max_json_size_bytes = 32768   # 32 KB
   max_rows = 50

   # Larger data exports
   max_json_size_bytes = 262144  # 256 KB
   max_rows = 500
   ```

4. **Use Indexed Columns in WHERE Clauses**:
   ```sql
   -- ✅ GOOD: Uses index on created_at
   SELECT COUNT(*) FROM orders WHERE created_at > NOW() - INTERVAL '1 day'

   -- ❌ BAD: Full table scan
   SELECT COUNT(*) FROM orders WHERE YEAR(created_at) = 2024
   ```

5. **Set Timeouts Based on Expected Query Time**:
   ```toml
   # Fast query
   timeout_seconds = 10

   # Complex aggregation
   timeout_seconds = 60
   ```

6. **Use target_id for Multi-Database Monitoring**:
   ```toml
   [[tasks]]
   name = "Primary DB Health"
   target_id = "db-primary"
   ...

   [[tasks]]
   name = "Replica DB Health"
   target_id = "db-replica"
   ...
   ```

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_PING.md](TASK_PING.md) - Network connectivity monitoring
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP endpoint monitoring
