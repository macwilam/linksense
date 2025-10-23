# SQL Query Task

The **SQL Query** task monitors database health and query performance by executing SQL statements and measuring their response time. It's designed to ensure database availability and detect performance degradation before it impacts applications.

**Note**: This task requires the `sql-tasks` feature flag to be enabled during compilation.

## Implementation Details

### Crate: `sqlx`
**Link**: [sqlx on crates.io](https://crates.io/crates/sqlx)

This task uses **`sqlx`**, a pure Rust async SQL toolkit and database driver.

**Key Characteristics**:
- **Async/Non-blocking**: Built on Tokio, no thread blocking during queries
- **Multi-Database Support**: PostgreSQL, MySQL, SQLite via feature flags
- **Runtime Query Execution**: Queries are strings, not compile-time checked
- **Connection Pooling**: Automatic connection management (not used, single connection per task)
- **Type Mapping**: Automatic conversion between SQL and Rust types
- **No ORM**: Direct SQL execution, not an Object-Relational Mapper

**Consequences**:
- ✅ **Database Flexibility**: Single codebase supports multiple database engines
- ✅ **Async Performance**: Non-blocking queries allow high concurrency
- ✅ **Pure Rust**: No C dependencies, easier cross-compilation
- ✅ **Production Ready**: Used by many production Rust applications
- ⚠️ **Feature Compilation**: Must compile with specific database drivers
- ⚠️ **No Query Validation**: SQL syntax errors only detected at runtime
- ⚠️ **Driver Limitations**: Supported databases determined by `sqlx` capabilities

### Supported Databases

**PostgreSQL** (`database_type = "postgres"`):
- **Driver**: `sqlx` with `runtime-tokio-rustls` feature
- **Protocol**: PostgreSQL wire protocol (native, not libpq)
- **Versions**: PostgreSQL 9.5+, supports modern features
- **SSL/TLS**: Built-in via `rustls` (no OpenSSL dependency)

**MySQL** (`database_type = "mysql"`):
- **Driver**: `sqlx` with `runtime-tokio-rustls` feature
- **Protocol**: MySQL wire protocol (native, not libmysqlclient)
- **Versions**: MySQL 5.6+, MariaDB 10.2+
- **SSL/TLS**: Built-in via `rustls`

**SQLite** (`database_type = "sqlite"`):
- **Driver**: `sqlx` with bundled SQLite (via `rusqlite`)
- **File Access**: Direct file-based database access
- **Versions**: SQLite 3.x
- **Note**: Local file only, no client-server

**Databases NOT Supported**:
- ❌ **Oracle**: Not supported by `sqlx`
- ❌ **SQL Server (MSSQL)**: Experimental support only, not production-ready
- ❌ **MongoDB**: Not a SQL database (different driver needed)
- ❌ **Cassandra**: Not a SQL database

### Query Execution Flow

```rust
1. Parse database_url and create connection
2. Authenticate with database server
3. Send SQL query string to database
4. Wait for response (async, with timeout)
5. Fetch all rows from result set
6. Count rows, measure total execution time
7. Close connection (or return to pool)
```

**Timing Measurement**:
- `total_time_ms`: Complete round-trip time including:
  - Network latency to database
  - Query parsing and planning
  - Query execution on database
  - Result set transfer
  - Row counting on agent side

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

**For Production Monitoring**, this means:
- Each query tests full connection establishment (good for health checks)
- Connection overhead included in timing (may add 10-50ms)
- Database connection limits should account for agent connections

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
4. Consider secrets management for production (Vault, AWS Secrets Manager)

**SQL Injection Protection**:
- Static queries only (no user input)
- No string interpolation in queries
- Parameterization not available (queries are fixed strings)

### Why `sqlx` Instead of Alternatives?

**vs. `diesel` (ORM)**:
- Diesel requires compile-time schema, too heavy for monitoring
- Diesel is sync-only, would block Tokio runtime
- sqlx is lighter-weight for simple query execution

**vs. Native drivers** (`rust-postgres`, `mysql_async`):
- sqlx provides unified interface across databases
- Easier to support multiple database types
- Built-in async support

**vs. `tokio-postgres`, `tokio-mysql`**:
- sqlx includes these internally
- Provides higher-level abstractions
- Better error handling

## Configuration

### Basic Configuration

```toml
[[tasks]]
type = "sql_query"
name = "Database Health Check"
schedule_seconds = 60
query = "SELECT 1"
database_url = "postgresql://user:password@localhost:5432/mydb"
database_type = "postgres"
timeout_seconds = 30
```

### Advanced Configuration

```toml
[[tasks]]
type = "sql_query"
name = "Active Users Count"
schedule_seconds = 120
query = "SELECT COUNT(*) FROM users WHERE last_active > NOW() - INTERVAL '1 hour'"
database_url = "postgresql://monitoring:secret@db.example.com:5432/production"
database_type = "postgres"
username = "monitoring"          # Optional: override URL username
password = "secret"              # Optional: override URL password
timeout_seconds = 45
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"sql_query"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between queries (≥60 seconds, enforced) |
| `query` | string | ✅ | - | SQL query to execute (SELECT recommended) |
| `database_url` | string | ✅ | - | Database connection URL |
| `database_type` | string | ✅ | - | Database type: `postgres`, `mysql`, `sqlite` |
| `username` | string | ❌ | - | Optional username (overrides URL) |
| `password` | string | ❌ | - | Optional password (overrides URL) |
| `timeout_seconds` | integer | ❌ | 30 | Query timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "db-primary", "db-replica") |

### Database Connection URLs

#### PostgreSQL
```toml
database_url = "postgresql://username:password@hostname:5432/database_name"
database_type = "postgres"

# With SSL
database_url = "postgresql://user:pass@host:5432/db?sslmode=require"
```

#### MySQL
```toml
database_url = "mysql://username:password@hostname:3306/database_name"
database_type = "mysql"

# With SSL
database_url = "mysql://user:pass@host:3306/db?ssl-mode=required"
```

#### SQLite
```toml
database_url = "sqlite:///path/to/database.db"
database_type = "sqlite"

# In-memory database
database_url = "sqlite::memory:"
```

### Configuration Examples

#### Simple Health Check
```toml
[[tasks]]
type = "sql_query"
name = "Postgres Health"
schedule_seconds = 60
query = "SELECT 1"
database_url = "postgresql://monitor:secret@localhost/postgres"
database_type = "postgres"
```

#### Monitor Table Row Count
```toml
[[tasks]]
type = "sql_query"
name = "Orders Table Size"
schedule_seconds = 300
query = "SELECT COUNT(*) FROM orders"
database_url = "postgresql://readonly:pass@db.prod.com:5432/sales"
database_type = "postgres"
timeout_seconds = 60
```

#### Track Active Sessions
```toml
[[tasks]]
type = "sql_query"
name = "Active Database Connections"
schedule_seconds = 120
query = "SELECT COUNT(*) FROM pg_stat_activity WHERE state = 'active'"
database_url = "postgresql://admin:secret@localhost/postgres"
database_type = "postgres"
```

#### Replica Lag Detection
```toml
# Primary database
[[tasks]]
type = "sql_query"
name = "Primary - Latest Order ID"
schedule_seconds = 300
query = "SELECT MAX(id) FROM orders"
database_url = "postgresql://user:pass@primary.db.com/sales"
database_type = "postgres"

# Read replica
[[tasks]]
type = "sql_query"
name = "Replica - Latest Order ID"
schedule_seconds = 300
query = "SELECT MAX(id) FROM orders"
database_url = "postgresql://user:pass@replica.db.com/sales"
database_type = "postgres"

# Compare results to detect lag
```

#### Monitor Critical Business Metrics
```toml
[[tasks]]
type = "sql_query"
name = "Recent Transactions"
schedule_seconds = 180
query = """
SELECT COUNT(*)
FROM transactions
WHERE created_at > NOW() - INTERVAL '5 minutes'
  AND status = 'completed'
"""
database_url = "mysql://analytics:pass@analytics.db.com:3306/payments"
database_type = "mysql"
```

#### Slow Query Detection (Synthetic)
```toml
[[tasks]]
type = "sql_query"
name = "Complex Report Query"
schedule_seconds = 600
query = """
SELECT
  DATE(created_at) as date,
  COUNT(*) as orders,
  SUM(total) as revenue
FROM orders
WHERE created_at > NOW() - INTERVAL '30 days'
GROUP BY DATE(created_at)
ORDER BY date DESC
"""
database_url = "postgresql://reporting:pass@warehouse.db.com/analytics"
database_type = "postgres"
timeout_seconds = 120
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
| `target_id` | TEXT | Optional target identifier from task configuration |


### Metrics Interpretation

#### Query Performance
- **< 50ms**: Excellent (indexed lookup, simple query)
- **50-200ms**: Good (moderate complexity or small joins)
- **200-1000ms**: Acceptable (complex reports or aggregations)
- **1-5s**: Slow (needs optimization)
- **> 5s**: Very slow (critical performance issue)

#### Row Count Patterns
- **Consistent row_count**: Stable data, expected results
- **Growing row_count**: Data growth (normal for COUNT queries on active tables)
- **Sudden row_count change**: Investigate (data deleted, query changed, issue)

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
- **Result Set Size**: How many rows are returned?
- **Database Availability**: Is the database reachable and responsive?

### Strong Sides

1. **Application-Level Monitoring**: Tests actual database operations, not just network connectivity
2. **Performance Tracking**: Measures real query execution time including database processing
3. **Multi-Database Support**: Works with PostgreSQL, MySQL, SQLite, and others
4. **Proactive Detection**: Catches slow queries before they cause application timeouts
5. **Connection Validation**: Ensures connection pool and authentication work correctly
6. **Data Consistency Checks**: Can validate expected data exists

### Typical Use Cases

- **Database Health Monitoring**: Verify primary/replica databases are responding
- **Query Performance Tracking**: Monitor critical query execution times
- **Connection Pool Health**: Ensure database connections are available
- **Read Replica Lag Detection**: Compare query results across replicas
- **Data Validation**: Confirm expected records exist (user count, inventory levels)
- **Slow Query Detection**: Alert when queries exceed performance thresholds
- **Failover Verification**: Test database failover/high availability works

### Limitations

- **Feature Flag Required**: Must compile with `--features sql-tasks`
- **Read-Only Recommended**: Only SELECT queries should be used (no INSERT/UPDATE/DELETE)
- **Minimum Schedule**: Must run ≥60 seconds apart (enforced by validation)
- **Credentials in Config**: Database passwords stored in configuration files
- **Single Query per Task**: Each task executes one query (create multiple tasks for multiple queries)
- **No Transaction Control**: Queries auto-commit, no BEGIN/COMMIT/ROLLBACK

## Performance Characteristics

### Resource Usage

**Agent**:
- **CPU**: Low (1-3% during query execution)
- **Memory**: ~(row_count * row_size) + 10MB
  - Small result sets (<100 rows): ~1-5 MB
  - Large result sets (>1000 rows): ~10-50 MB
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
- **Timeout**: Configurable (default 30s) - **Now properly enforced** with `tokio::time::timeout()` to prevent indefinite hangs

### Scalability
- Can monitor **10-20 databases** simultaneously
- Queries execute sequentially (one at a time per task)
- Recommended schedule: 60-300 seconds
- For high-frequency monitoring, use connection pooling at database level

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

3. **Use Secrets Management** (Advanced):
   - Store credentials in HashiCorp Vault, AWS Secrets Manager, etc.
   - Inject at runtime via environment variables (requires code modification)

4. **Network Isolation**:
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

**Avoid dynamic SQL**:
- Don't include user input in queries
- No string concatenation from external sources
- Parameterized queries not supported (static queries only)

## Troubleshooting

### Common Issues

#### "Connection timeout" Errors
**Symptom**: Queries consistently timeout
**Causes**:
- Database server unreachable
- Firewall blocking connection
- Database under heavy load
- Connection pool exhausted
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
- pg_hba.conf restricting connections (PostgreSQL)
**Solutions**:
```sql
-- PostgreSQL: Check user permissions
SELECT * FROM pg_roles WHERE rolname = 'monitoring';
SELECT grantee, privilege_type
FROM information_schema.role_table_grants
WHERE grantee = 'monitoring';

-- MySQL: Check user grants
SHOW GRANTS FOR 'monitoring'@'%';
```

#### Slow Query Performance
**Symptom**: `total_time_ms` consistently high (> 1000ms)
**Causes**:
- Missing indexes
- Full table scan
- Database under load
- Query complexity
**Solutions**:
```sql
-- PostgreSQL: Analyze query plan
EXPLAIN ANALYZE SELECT COUNT(*) FROM users;

-- Add indexes for common filters
CREATE INDEX idx_users_last_active ON users(last_active);

-- MySQL: Check slow query log
SHOW VARIABLES LIKE 'slow_query_log';
```

#### Connection Pool Exhaustion
**Symptom**: Intermittent "too many connections" errors
**Cause**: Agent not closing connections properly or database max_connections too low
**Solutions**:
```sql
-- PostgreSQL: Check connection limits
SELECT * FROM pg_stat_activity;
SHOW max_connections;

-- Increase max connections (requires restart)
ALTER SYSTEM SET max_connections = 200;

-- MySQL: Check connections
SHOW PROCESSLIST;
SHOW VARIABLES LIKE 'max_connections';
```

### Debugging Tips

**Test query manually:**
```bash
# PostgreSQL
psql -h hostname -U username -d database -c "SELECT COUNT(*) FROM users" -c "\timing"

# MySQL
mysql -h hostname -u username -p -D database -e "SELECT COUNT(*) FROM users"

# Measure execution time
time psql -h hostname -U username -d database -c "SELECT 1"
```

**Analyze performance patterns:**
```sql
SELECT
  timestamp,
  total_time_ms,
  row_count,
  success,
  error
FROM raw_metric_sql_query
WHERE task_name = 'Orders Count'
ORDER BY timestamp DESC
LIMIT 50;
```

**Enable database query logging:**
```sql
-- PostgreSQL: Enable slow query log
ALTER SYSTEM SET log_min_duration_statement = 1000;  -- Log queries > 1s
SELECT pg_reload_conf();

-- MySQL: Enable slow query log
SET GLOBAL slow_query_log = 'ON';
SET GLOBAL long_query_time = 1;  -- Log queries > 1s
```

## Best Practices

1. **Use Simple Health Check Queries**:
   ```sql
   -- Best for availability monitoring
   SELECT 1

   -- Or check database version
   SELECT version()
   ```

2. **Create Dedicated Monitoring Schema** (Optional):
   ```sql
   CREATE SCHEMA monitoring;

   CREATE TABLE monitoring.health_check (
     id SERIAL PRIMARY KEY,
     check_time TIMESTAMP DEFAULT NOW()
   );

   -- Query this table instead of production tables
   SELECT COUNT(*) FROM monitoring.health_check;
   ```

3. **Set Appropriate Schedules**:
   - Critical databases: 60-120 seconds
   - Less critical: 300-600 seconds
   - Avoid < 60 seconds (enforced minimum)

4. **Monitor Both Speed and Result**:
   ```toml
   # Task 1: Fast health check
   [[tasks]]
   name = "DB Health"
   query = "SELECT 1"
   schedule_seconds = 60

   # Task 2: Business metric
   [[tasks]]
   name = "Active Users"
   query = "SELECT COUNT(*) FROM users WHERE last_active > NOW() - INTERVAL '1 hour'"
   schedule_seconds = 300
   ```

5. **Use Indexed Columns in WHERE Clauses**:
   ```sql
   -- ✅ GOOD: Uses index on created_at
   SELECT COUNT(*) FROM orders WHERE created_at > NOW() - INTERVAL '1 day'

   -- ❌ BAD: Full table scan
   SELECT COUNT(*) FROM orders WHERE YEAR(created_at) = 2024
   ```

6. **Set Timeouts Based on Expected Query Time**:
   ```toml
   # Fast query
   timeout_seconds = 10

   # Complex aggregation
   timeout_seconds = 60
   ```

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_PING.md](TASK_PING.md) - Network connectivity monitoring
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP endpoint monitoring
