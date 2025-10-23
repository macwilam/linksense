# LinkSense Agent - Installation and Configuration Guide

The LinkSense agent is a lightweight monitoring service that executes network tests and reports metrics. This guide covers agent installation, configuration, and operation.

## 🎯 Operating Modes Overview

LinkSense agent supports two operating modes to fit different deployment scenarios:

### Centralized Mode (Default)
Agent connects to a central server to report metrics and receive configuration updates. Best for production deployments with multiple monitoring locations.

**Key Features:**
- Metrics sent to central server
- Automatic configuration synchronization
- Centralized management and alerting
- Requires: `central_server_url`, `api_key`

### Standalone Mode (Local-Only)
Agent operates independently without server connection. All metrics stored locally in SQLite. Best for development, testing, or air-gapped environments.

**Key Features:**
- No network communication required
- All data stored locally
- Manual configuration updates
- Requires: `local_only = true` in config

Choose your mode before proceeding with installation. Configuration examples for both modes are provided in the [Configuration](#️-configuration) section below. For detailed mode information, see [Operating Modes - Detailed Configuration](#-operating-modes---detailed-configuration).

## 📦 Installation

### Build from Source

```bash
# Build agent only
cargo build --release --bin agent

# Build with SQL monitoring support
cargo build --release --bin agent --features sql-tasks

# Binary location
./target/release/agent
```

### Prerequisites

**Operating System Requirements**:
- Linux: Recommended (best ICMP ping support)
- macOS: Supported
- Windows: Supported with limitations

**For ICMP Ping Tasks on Linux**:
```bash
# Option 1: Add user to ping group (recommended) Code below will add all users to the group
# You can check specfic id of your user and narrow down the permission.
echo 'net.ipv4.ping_group_range = 0 2147483647' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p

# Option 2: Add specific user to ping group
sudo usermod -a -G ping $USER
# Log out and back in for group change to take effect

# Option 3: Grant CAP_NET_RAW capability (binary-specific)
sudo setcap cap_net_raw+ep ./target/release/agent
```

## ⚙️ Configuration

The agent requires a configuration directory containing two TOML files:
- `agent.toml` - Agent settings and server connection
- `tasks.toml` - Monitoring task definitions

### Directory Structure

```
agent-config/
├── agent.toml          # Agent configuration
├── tasks.toml          # Task definitions
├── agent.db            # Local SQLite database (auto-created)
└── previous_configs/   # Automatic backups (auto-created)
    └── tasks_TIMESTAMP.toml
```

### agent.toml - Agent Configuration

**Centralized Mode** (with server):
```toml
agent_id = "production-web-01"
central_server_url = "http://monitoring.example.com:8787"
api_key = "your-secure-api-key"
local_data_retention_days = 7
auto_update_tasks = true
metrics_flush_interval_seconds = 5
```

**Standalone Mode** (without server):
```toml
agent_id = "standalone-01"
local_data_retention_days = 30
local_only = true
# central_server_url and api_key not required in local_only mode
```

#### Configuration Options

| Option | Required | Default | Description |
|--------|----------|---------|-------------|
| `agent_id` | Yes | - | Unique identifier for this agent (alphanumeric, hyphens, underscores) |
| `central_server_url` | Conditional* | - | URL of the monitoring server |
| `api_key` | Conditional* | - | Authentication key for server (must match server's api_key) |
| `local_data_retention_days` | Yes | - | Days to retain metrics locally before cleanup |
| `auto_update_tasks` | No | `false` | Enable automatic config sync from server every 5 minutes |
| `local_only` | No | `false` | Run in standalone mode without server communication |
| `metrics_flush_interval_seconds` | No | `5` | Interval between database flushes for buffered metrics (min: 1, max: 60) |

*Required unless `local_only = true`

**Note:** When not in `local_only` mode, the agent automatically registers with the server at startup by uploading its local configuration. The server will store this configuration only if it doesn't already have one for this agent ID.

### tasks.toml - Task Configuration

Define monitoring tasks as an array of TOML tables:

```toml
[[tasks]]
type = "ping"
name = "gateway"
schedule_seconds = 30
host = "192.168.1.1"

[[tasks]]
type = "http_get"
name = "api-health"
schedule_seconds = 60
url = "https://api.example.com/health"

[[tasks]]
type = "dns_query"
name = "dns-test"
schedule_seconds = 120
server = "8.8.8.8"
domain = "example.com"
record_type = "A"
```

#### Common Task Parameters

All tasks support these parameters:

| Parameter | Required | Description |
|-----------|----------|-------------|
| `type` | Yes | Task type: `ping`, `http_get`, `http_content`, `dns_query`, `dns_query_doh`, `bandwidth`, `sql_query` |
| `name` | Yes | Unique identifier for this task (used in metrics and logs) |
| `schedule_seconds` | Yes | Interval between executions (minimum varies by task type) |

For task-specific parameters, see the detailed task documentation:
- [TASK_PING.md](TASK_PING.md) - ICMP ping
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP/HTTPS requests
- [TASK_HTTP_CONTENT.md](TASK_HTTP_CONTENT.md) - HTTP content validation
- [TASK_DNS.md](TASK_DNS.md) - DNS queries
- [TASK_BANDWIDTH.md](TASK_BANDWIDTH.md) - Bandwidth testing
- [TASK_SQL.md](TASK_SQL.md) - Database queries

## 🚀 Running the Agent

### Basic Usage

```bash
# Start agent with config directory
./target/release/agent /path/to/config-dir

# With logging
RUST_LOG=info ./target/release/agent ./agent-config

# Debug mode
RUST_LOG=debug ./target/release/agent ./agent-config
```

### Command-Line Overrides

Override configuration values via command line (changes are persisted to `agent.toml`):

```bash
# Override server connection
./target/release/agent ./agent-config \
  --server-url http://localhost:8787 \
  --api-key my-secure-key

# Override all settings
./target/release/agent ./agent-config \
  --agent-id my-agent \
  --server-url http://localhost:8787 \
  --api-key my-key \
  --retention-days 30 \
  --auto-update-tasks true

# Switch to local-only mode
./target/release/agent ./agent-config --local-only true
```

#### Command-Line Options

| Option | Description | Example |
|--------|-------------|---------|
| `--agent-id <ID>` | Override agent_id | `--agent-id prod-web-01` |
| `--server-url <URL>` | Override central_server_url | `--server-url http://localhost:8787` |
| `--api-key <KEY>` | Override api_key | `--api-key secret-key` |
| `--retention-days <DAYS>` | Override local_data_retention_days | `--retention-days 14` |
| `--auto-update-tasks <BOOL>` | Override auto_update_tasks | `--auto-update-tasks true` |
| `--local-only <BOOL>` | Override local_only mode | `--local-only false` |

**Override Behavior**:
- Command-line arguments take precedence over config file
- All overrides are written back to `agent.toml` for persistence
- Partial overrides supported (override only what you need)
- No file write occurs if value matches existing config
- Configuration validated before applying and persisting

## 🏗️ Agent Architecture

### Data Storage

The agent uses SQLite for local metric storage with two-tier architecture:

**Raw Metrics Tables** (individual measurements):
- `raw_metric_ping` - Individual ping measurements
- `raw_metric_http_get` - Individual HTTP request timings
- `raw_metric_http_content` - Individual content check results
- `raw_metric_dns_query` - Individual DNS query results
- `raw_metric_bandwidth` - Individual bandwidth tests
- `raw_metric_sql_query` - Individual SQL query results

**Aggregated Metrics Tables** (60-second summaries):
- `agg_metric_ping` - Aggregated ping statistics
- `agg_metric_http_get` - Aggregated HTTP timings
- `agg_metric_http_content` - Aggregated content checks
- `agg_metric_dns_query` - Aggregated DNS queries
- `agg_metric_bandwidth` - Aggregated bandwidth tests
- `agg_metric_sql_query` - Aggregated SQL queries

**Aggregation Process**:
1. Tasks execute and store raw measurements
2. Every 60 seconds: SQL GROUP BY aggregates raw data
3. Aggregations include: min, max, avg, stddev, count, success_rate
4. Aggregated data sent to server (if configured)
5. Raw and aggregated data subject to retention policy

See [README_DATABASE.md](README_DATABASE.md) for complete schema.

### Task Scheduling

Tasks are scheduled using Tokio intervals:

```rust
// Conceptual scheduling model
for each task in tasks.toml:
    spawn_task_loop:
        every task.schedule_seconds:
            execute_task()
            store_raw_metric()
```

**Scheduling Features**:
- Staggered startup prevents thundering herd
- Timeout management prevents hung tasks
- Concurrent execution via async runtime
- Automatic retry on transient failures

### Configuration Sync (Auto-Update)

When `auto_update_tasks = true`, agent checks for config updates every 5 minutes:

```
┌──────────────────────────────────────────────────────────┐
│ Agent sends metrics with config hash                     │
│   POST /api/v1/metrics                                   │
│   X-Config-Hash: blake3_hash_of_tasks_toml               │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ Server compares hash with master config                  │
│   ├─→ Match: HTTP 200 (no action)                       │
│   └─→ Mismatch: HTTP 409 (config update available)      │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓ (if mismatch)
┌──────────────────────────────────────────────────────────┐
│ Agent downloads new config                               │
│   GET /api/v1/config/verify                             │
│   Response: base64(gzip(tasks.toml))                     │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ Agent applies update                                     │
│   1. Validate new config                                 │
│   2. Backup current config to previous_configs/          │
│   3. Write new config to tasks.toml                      │
│   4. Reload task scheduler                               │
│   5. Report errors if validation fails                   │
└──────────────────────────────────────────────────────────┘
```

**Configuration Backup**: Old configs automatically saved to `previous_configs/tasks_TIMESTAMP.toml`.

## 🎯 Operating Modes - Detailed Configuration

### Centralized Mode (Default)

Agent reports to central server for aggregated monitoring:

**Configuration**:
```toml
agent_id = "prod-agent-01"
central_server_url = "http://server:8787"
api_key = "secure-key"
local_data_retention_days = 7
auto_update_tasks = true          # Recommended: enable automatic config sync
metrics_flush_interval_seconds = 5 # Optional: customize database flush interval
```

**Features**:
- Metrics sent to server every 60 seconds
- Configuration managed centrally
- Automatic config synchronization
- Server coordination for bandwidth tests

**Use Cases**:
- Production monitoring across multiple locations
- Centralized alerting and visualization
- Unified configuration management
- Historical metric analysis

### Standalone Mode (Local-Only)

Agent operates independently without server connection:

**Configuration**:
```toml
agent_id = "standalone-01"
local_data_retention_days = 30
local_only = true
```

**Features**:
- All metrics stored locally in SQLite
- No network communication required
- Manual configuration updates
- Independent operation

**Use Cases**:
- Development and testing
- Air-gapped environments
- Single-location monitoring
- Proof-of-concept deployments

### Switching Between Modes

```bash
# Switch to centralized mode
./target/release/agent ./config \
  --local-only false \
  --server-url http://server:8787 \
  --api-key your-key

# Switch to standalone mode
./target/release/agent ./config --local-only true
```

## 🐛 Troubleshooting

### Connection Issues

**Symptom**: Agent cannot connect to server

**Diagnosis**:
```bash
# Check network connectivity
curl -v http://server:8787/api/v1/health

# Verify agent configuration
cat agent-config/agent.toml

# Check agent logs
RUST_LOG=debug ./target/release/agent ./agent-config
```

**Solutions**:
- Verify `central_server_url` is correct
- Ensure `api_key` matches server configuration
- Check firewall rules allow outbound connections
- Verify server is running and accessible

### ICMP Ping Failures

**Symptom**: Ping tasks fail with permission errors

**Solutions**:
```bash
# Option 1: Configure system-wide ping group
echo 'net.ipv4.ping_group_range = 0 2147483647' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p

# Option 2: Add user to ping group
sudo usermod -a -G ping $USER
# Log out and back in

# Option 3: Grant capability to binary
sudo setcap cap_net_raw+ep ./target/release/agent
```

### Tasks Not Executing

**Symptom**: Tasks defined but no metrics generated

**Diagnosis**:
```bash
# Check task configuration syntax
cargo run --bin agent -- ./agent-config --validate

# Enable debug logging
RUST_LOG=agent=debug,scheduler=trace ./target/release/agent ./agent-config

# Query local database
sqlite3 agent-config/agent.db "SELECT * FROM raw_metric_ping LIMIT 10;"
```

**Solutions**:
- Verify TOML syntax in `tasks.toml`
- Check task parameters match task type requirements
- Ensure `schedule_seconds` meets minimum requirements
- Review logs for validation errors

### Configuration Updates Not Applied

**Symptom**: Server has new config but agent continues using old config

**Diagnosis**:
```bash
# Check auto-update setting
grep auto_update_tasks agent-config/agent.toml

# Verify agent can reach server
curl -H "X-API-Key: your-key" -H "X-Agent-ID: agent1" \
  http://server:8787/api/v1/config/verify

# Check agent logs for update errors
grep "config" logs/agent.log
```

**Solutions**:
- Ensure `auto_update_tasks = true` in agent.toml
- Verify agent ID matches server-side config filename
- Check `reconfigure/error.txt` on server for validation errors
- Manually restart agent to force config reload

### High Memory/CPU Usage

**Symptom**: Agent consuming excessive resources

**Diagnosis**:
```bash
# Check task count and intervals
wc -l agent-config/tasks.toml
grep schedule_seconds agent-config/tasks.toml

# Monitor with system tools
top -p $(pgrep agent)
```

**Solutions**:
- Reduce number of concurrent tasks
- Increase `schedule_seconds` for less frequent execution
- Check for tasks with very short intervals (<10s)
- Review tasks for timeout issues (hung connections)

### Database Errors

**Symptom**: SQLite errors in logs

**Diagnosis**:
```bash
# Check database integrity
sqlite3 agent-config/agent.db "PRAGMA integrity_check;"

# Check file permissions
ls -l agent-config/agent.db

# Check disk space
df -h
```

**Solutions**:
- Ensure write permissions on config directory
- Verify sufficient disk space available
- If corrupted, delete `agent.db` (will be recreated)
- Reduce `local_data_retention_days` if disk space limited

## 📊 Monitoring Agent Health

### Log Files

Logs written to `./logs/agent.log` (structured JSON):

```bash
# View recent logs
tail -f logs/agent.log

# Filter by level
grep '"level":"ERROR"' logs/agent.log

# Filter by task
grep '"task_name":"gateway"' logs/agent.log
```

### Local Database Queries

```bash
# Check recent metrics
sqlite3 agent-config/agent.db <<EOF
SELECT task_name, timestamp, success, value
FROM raw_metric_ping
ORDER BY timestamp DESC
LIMIT 10;
EOF

# Check aggregation status
sqlite3 agent-config/agent.db <<EOF
SELECT task_name, window_start, avg_value, success_count, failure_count
FROM agg_metric_ping
ORDER BY window_start DESC
LIMIT 10;
EOF

# Database size
du -h agent-config/agent.db
```

### Environment Variables

```bash
# Comprehensive debug logging
RUST_LOG=debug ./target/release/agent ./agent-config

# Module-specific logging
RUST_LOG=agent=info,scheduler=debug,task_http=trace ./target/release/agent ./agent-config

# JSON output for log aggregation
RUST_LOG=info RUST_LOG_FORMAT=json ./target/release/agent ./agent-config
```

## 🔒 Security Best Practices

1. **API Key Security**:
   - Use strong, randomly generated API keys
   - Rotate keys periodically
   - Store in environment variables for sensitive deployments
   - Never commit keys to version control

2. **File Permissions**:
   ```bash
   chmod 600 agent-config/agent.toml  # Protect API key
   chmod 644 agent-config/tasks.toml  # Read-only for others
   chmod 700 agent-config/            # Restrict directory access
   ```

3. **SQL Tasks**:
   - Use read-only database accounts
   - Limit query complexity and execution time
   - Avoid queries that modify data
   - Use dedicated monitoring database user

4. **Network Security**:
   - Use HTTPS for `central_server_url` in production
   - Implement network segmentation
   - Restrict outbound connections to server only
   - Consider VPN or encrypted tunnels
   - **Firewall Configuration**: Agent requires NO incoming ports to be open
     - All communication is outbound-initiated (agent → server)
     - Recommended firewall policy: **Deny all incoming, Allow outgoing**
     - Agent only needs outbound access to:
       - Central server (if using centralized mode)
       - Monitored targets (HTTP/HTTPS/DNS endpoints)
       - Database servers (if using SQL tasks)
     - This makes the agent ideal for restrictive network environments

5. **Systemd Hardening**:
   - Run as dedicated non-root user
   - Use `ProtectSystem=strict`
   - Limit file system access with `ReadWritePaths`
   - Enable `NoNewPrivileges`

## 📚 See Also

- [README.md](README.md) - Project overview and architecture
- [README_SERVER.md](README_SERVER.md) - Server setup and management
- [README_DATABASE.md](README_DATABASE.md) - Database schema details
- [TASK_*.md](TASK_PING.md) - Task-specific documentation
