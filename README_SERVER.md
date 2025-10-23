# LinkSense Server - Installation and Configuration Guide

The LinkSense server is the central collection point for metrics from distributed agents. It manages agent configurations, coordinates bandwidth tests, and stores aggregated metrics. This guide covers server installation, configuration, and operation.

## 📦 Installation

### Build from Source

```bash
# Build server only
cargo build --release --bin server

# Binary location
./target/release/server
```

### Prerequisites

**Operating System Requirements**:
- Linux: Recommended
- macOS: Supported
- Windows: Supported

**Network Requirements**:
- Open port for agent connections (default: 8787)
- Sufficient bandwidth for agent communication
- Static IP or DNS name for reliable agent connections

**Storage Requirements**:
- SQLite database for metrics storage
- Disk space based on: (agents × metrics/min × retention_days × ~1KB)
- Example: 10 agents, 60 metrics/min, 30 days ≈ 26GB **Note:** The actual number depends on the size of one entry that can very between configurations.

## ⚙️ Configuration

The server requires a single TOML configuration file.

### server.toml - Server Configuration

**Minimal Configuration**:
```toml
listen_address = "0.0.0.0:8787"
api_key = "your-secure-server-api-key"
data_retention_days = 30
agent_configs_dir = "./agent-configs"
bandwidth_test_size_mb = 10
```

**Production Configuration**:
```toml
# Network binding
listen_address = "0.0.0.0:8787"

# Authentication
api_key = "production-secure-api-key-change-this"

# Data retention
data_retention_days = 90
cleanup_interval_hours = 24

# Agent configuration management
agent_configs_dir = "/etc/linksense/agent-configs"
reconfigure_check_interval_seconds = 10

# Bandwidth testing
bandwidth_test_size_mb = 50

# Security: Agent whitelist (empty = all agents allowed)
agent_id_whitelist = ["prod-agent-01", "prod-agent-02", "staging-agent-01"]

# Rate limiting
rate_limit_enabled = true
rate_limit_window_seconds = 60
rate_limit_max_requests = 100
```

### Configuration Options

| Option | Required | Default | Description |
|--------|----------|---------|-------------|
| `listen_address` | Yes | - | IP:port to bind server (e.g., "0.0.0.0:8787") |
| `api_key` | Yes | - | Authentication key for agents (must match agent configs) |
| `data_retention_days` | Yes | - | Days to retain metrics before cleanup |
| `agent_configs_dir` | Yes | - | Directory containing agent-specific task configurations |
| `bandwidth_test_size_mb` | Yes | - | Size of bandwidth test file in megabytes |
| `reconfigure_check_interval_seconds` | No | `10` | Interval to check for bulk reconfiguration requests (min: 1, max: 300) |
| `agent_id_whitelist` | No | `[]` | Allowed agent IDs (empty = all allowed) |
| `cleanup_interval_hours` | No | `24` | Hours between database cleanup runs |
| `rate_limit_enabled` | No | `true` | Enable rate limiting per agent |
| `rate_limit_window_seconds` | No | `60` | Rate limit time window |
| `rate_limit_max_requests` | No | `100` | Max requests per agent per window |

### Agent Configuration Directory Structure

The `agent_configs_dir` contains agent-specific task configurations:

```
agent-configs/
├── prod-agent-01.toml      # Tasks for prod-agent-01
├── prod-agent-02.toml      # Tasks for prod-agent-02
├── staging-agent-01.toml   # Tasks for staging-agent-01
└── dev-agent-01.toml       # Tasks for dev-agent-01
```

**Flat File Naming**: Configuration files must be named `{agent_id}.toml` where `agent_id` matches the agent's configured ID.

**Example agent configuration** (`prod-agent-01.toml`):
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
name = "dns-check"
schedule_seconds = 120
server = "8.8.8.8"
domain = "example.com"
record_type = "A"
```

## 🚀 Running the Server

### Basic Usage

```bash
# Start server with config file
./target/release/server ./server.toml

# With logging
RUST_LOG=info ./target/release/server ./server.toml

# Debug mode
RUST_LOG=debug ./target/release/server ./server.toml
```

### Command-Line Overrides

Override configuration values via command line (changes are persisted to `server.toml`):

```bash
# Override specific settings
./target/release/server ./server.toml \
  --listen-address 0.0.0.0:9090 \
  --api-key secure-production-key

# Override all settings
./target/release/server ./server.toml \
  --listen-address 0.0.0.0:8787 \
  --api-key production-key \
  --retention-days 90 \
  --agent-configs-dir /etc/linksense/agents \
  --bandwidth-size-mb 50
```

#### Command-Line Options

| Option | Description | Example |
|--------|-------------|---------|
| `--listen-address <ADDRESS>` | Override listen_address | `--listen-address 0.0.0.0:8787` |
| `--api-key <KEY>` | Override api_key | `--api-key secure-key` |
| `--retention-days <DAYS>` | Override data_retention_days | `--retention-days 90` |
| `--agent-configs-dir <DIR>` | Override agent_configs_dir | `--agent-configs-dir /etc/configs` |
| `--bandwidth-size-mb <MB>` | Override bandwidth_test_size_mb | `--bandwidth-size-mb 50` |

**Override Behavior**:
- Command-line arguments take precedence over config file
- All overrides are written back to `server.toml` for persistence
- Partial overrides supported (override only what you need)
- No file write occurs if value matches existing config
- Configuration validated before applying and persisting


## 🌐 API Endpoints

The server provides a RESTful API for agent communication. All endpoints require authentication.

### Authentication

All requests must include:
- `X-API-Key` header with the server's configured API key
- `X-Agent-ID` header with the agent's identifier

```bash
curl -H "X-API-Key: your-api-key" \
     -H "X-Agent-ID: agent1" \
     http://localhost:8787/api/v1/endpoint
```

### Endpoint Reference

#### POST /api/v1/metrics

Submit aggregated metrics to server.

**Headers**:
- `X-API-Key`: Server API key
- `X-Agent-ID`: Agent identifier
- `X-Config-Hash`: BLAKE3 hash of current agent config
- `Content-Type`: application/json

**Request Body**:
```json
{
  "ping": [
    {
      "task_name": "gateway",
      "window_start": "2024-01-15T10:00:00Z",
      "window_end": "2024-01-15T10:01:00Z",
      "min_rtt_ms": 1.2,
      "max_rtt_ms": 5.8,
      "avg_rtt_ms": 2.4,
      "stddev_rtt_ms": 0.8,
      "success_count": 30,
      "failure_count": 0,
      "sample_count": 30
    }
  ],
  "http_get": [...],
  "dns_query": [...]
}
```

**Responses**:
- `200 OK`: Metrics accepted, config hash matches
- `409 Conflict`: Metrics accepted, config hash mismatch (agent should update)
- `400 Bad Request`: Invalid request format
- `403 Forbidden`: Authentication failed or agent not whitelisted

#### GET /api/v1/config/verify

Download current configuration for agent.

**Headers**:
- `X-API-Key`: Server API key
- `X-Agent-ID`: Agent identifier

**Response**:
```json
{
  "config_data": "H4sIAAAAAAAA/6tWyk....",  // base64(gzip(tasks.toml))
  "config_hash": "blake3_hash_of_tasks_toml"
}
```

**Status Codes**:
- `200 OK`: Configuration returned
- `404 Not Found`: No config file for this agent
- `403 Forbidden`: Authentication failed or agent not whitelisted

#### POST /api/v1/config/upload

Upload configuration errors from agent.

**Headers**:
- `X-API-Key`: Server API key
- `X-Agent-ID`: Agent identifier
- `Content-Type`: application/json

**Request Body**:
```json
{
  "error": "Failed to parse TOML: expected newline, found comma at line 5"
}
```

**Status Codes**:
- `200 OK`: Error report accepted
- `403 Forbidden`: Authentication failed

#### POST /api/v1/bandwidth_test

Request bandwidth test coordination.

**Headers**:
- `X-API-Key`: Server API key
- `X-Agent-ID`: Agent identifier

**Response**:
```json
{
  "status": "approved",
  "test_size_mb": 10,
  "test_url": "/api/v1/bandwidth_download"
}
```

**Status Codes**:
- `200 OK`: Test approved, proceed with download
- `202 Accepted`: Test queued, wait and retry
- `403 Forbidden`: Authentication failed

#### GET /api/v1/bandwidth_download

Download test data for bandwidth measurement.

**Headers**:
- `X-API-Key`: Server API key
- `X-Agent-ID`: Agent identifier

**Response**: Binary data stream of configured size

**Status Codes**:
- `200 OK`: Test data stream
- `403 Forbidden`: Authentication failed or no active test

## 🔧 Configuration Management

### Server-Side Agent Configurations

Each agent's monitoring tasks are defined in `{agent_configs_dir}/{agent_id}.toml`:

```bash
# Directory structure
agent-configs/
├── agent1.toml
├── agent2.toml
└── prod-server-01.toml

# Create new agent config
cat > agent-configs/new-agent.toml <<EOF
[[tasks]]
type = "ping"
name = "test"
schedule_seconds = 30
host = "8.8.8.8"
EOF
```

**Configuration Validation**: Server validates all configs before distribution:
- TOML syntax checking
- Required field validation
- Task parameter constraints (e.g., bandwidth interval ≥60s)
- Rejects invalid configurations with detailed errors

**Agent Discovery**: Agents automatically download their config on first connection or when hash mismatches.

### Bulk Reconfiguration System

Update multiple agents simultaneously using the `reconfigure/` directory:

#### Quick Start

```bash
# 1. Create reconfigure directory (if not exists)
mkdir -p reconfigure

# 2. Create agent list
cat > reconfigure/agent_list.txt <<EOF
agent1
agent2
prod-server-01
EOF

# 3. Create new task configuration
cat > reconfigure/tasks.toml <<EOF
[[tasks]]
type = "ping"
name = "gateway"
schedule_seconds = 30
host = "192.168.1.1"

[[tasks]]
type = "http_get"
name = "api"
schedule_seconds = 60
url = "https://api.example.com/health"
EOF

# 4. Server processes within 10 seconds automatically
```

#### Reconfiguration Process

```
┌──────────────────────────────────────────────────────────┐
│ Server monitors reconfigure/ directory every 10 seconds  │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ Detect agent_list.txt + tasks.toml                       │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ Validate tasks.toml                                      │
│   ├─→ Valid: Continue                                    │
│   └─→ Invalid: Write error.txt, abort                    │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ For each agent in agent_list.txt:                        │
│   1. Backup existing config to backup/                   │
│   2. Copy tasks.toml to {agent_id}.toml                  │
│   3. Log success                                         │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ Cleanup: Delete agent_list.txt and tasks.toml            │
└──────────────────────────────────────────────────────────┘
```

#### Reconfiguration Files

**reconfigure/agent_list.txt** - List of agent IDs (one per line):
```text
agent1
agent2
prod-server-01
```

**reconfigure/tasks.toml** - New task configuration:
```toml
[[tasks]]
type = "ping"
name = "gateway"
schedule_seconds = 30
host = "192.168.1.1"
```

**reconfigure/error.txt** - Validation errors (auto-created on failure):
```text
Error validating configuration:
  - bandwidth task 'throughput': schedule_seconds must be >= 60
  - task name 'duplicate' appears multiple times
```

**reconfigure/backup/** - Automatic backups (auto-created):
```
reconfigure/backup/
├── agent1_20240115_100530.toml
├── agent2_20240115_100530.toml
└── prod-server-01_20240115_100530.toml
```

#### Safety Features

1. **Validation First**: Tasks.toml validated before any changes
2. **Automatic Backups**: Original configs saved with timestamp
3. **Atomic Updates**: All agents updated or none (on validation failure)
4. **Error Reporting**: Detailed error messages in error.txt
5. **Agent Whitelist Aware**: Only updates whitelisted agents (if configured)

**See [README_RECONFIGURE_FEATURE.md](README_RECONFIGURE_FEATURE.md) for complete details.**

### Dynamic Configuration Updates

Agents automatically detect config changes via hash comparison:

```
Agent Sends Metrics → Include Config Hash
                ↓
Server Compares Hash with Master
                ↓
        ┌───────┴───────┐
        ↓               ↓
    Hash Match      Hash Mismatch
        ↓               ↓
  HTTP 200 OK      HTTP 409 Conflict
                        ↓
              Agent Downloads New Config
                        ↓
              Agent Applies Update
```

**Update Timeline**:
1. Server admin updates `{agent_id}.toml`
2. Agent sends next metric batch (within 60s)
3. Server detects hash mismatch, returns 409
4. Agent downloads new config immediately
5. Agent validates, backs up old config, applies new config
6. Agent reports errors if validation fails

**No Restart Required**: Configuration updates applied without agent restart.

## 🔐 Security Features

### Authentication

**API Key Validation**: All requests require valid `X-API-Key` header matching server configuration.

```bash
# Valid request
curl -H "X-API-Key: correct-key" \
     -H "X-Agent-ID: agent1" \
     http://localhost:8787/api/v1/metrics

# Invalid key → 403 Forbidden
curl -H "X-API-Key: wrong-key" \
     http://localhost:8787/api/v1/metrics
```

### Agent ID Whitelist

Restrict which agents can connect to server:

**Configuration**:
```toml
# Allow all agents (default)
agent_id_whitelist = []

# Allow specific agents only
agent_id_whitelist = ["prod-agent-01", "prod-agent-02", "staging-01"]
```

**Behavior**:
- **Empty whitelist** `[]`: All agents with valid API key allowed
- **Configured whitelist**: Only listed agent IDs permitted

**Enforcement**: Whitelist checked on all API endpoints:
- `/api/v1/metrics` - Metric submission
- `/api/v1/config/verify` - Config download
- `/api/v1/config/upload` - Error reporting
- `/api/v1/bandwidth_test` - Bandwidth coordination
- `/api/v1/bandwidth_download` - Test data download

**Response**:
```json
// Whitelisted agent → 200 OK
// Non-whitelisted agent → 403 Forbidden
{
  "error": "Agent ID 'unauthorized-agent' not in whitelist"
}
```

**Use Cases**:
- Lock down production server to known agents
- Prevent unauthorized monitoring agents
- Multi-tenant deployments with separate servers
- Compliance with security policies

### Rate Limiting

Protect server from excessive requests:

**Configuration**:
```toml
rate_limit_enabled = true
rate_limit_window_seconds = 60
rate_limit_max_requests = 100
```

**Behavior**:
- Per-agent rate limiting (agents tracked separately)
- Sliding window algorithm
- Applies to all API endpoints

**Response on Limit Exceeded**:
```json
// 429 Too Many Requests
{
  "error": "Rate limit exceeded. Try again later."
}
```

### Configuration Validation

All agent configurations validated before distribution:

**Validation Checks**:
- TOML syntax correctness
- Required fields present
- Valid task types
- Parameter constraints (e.g., bandwidth interval ≥60s)
- No duplicate task names
- Valid URLs, domains, and IP addresses

**Invalid Configuration Handling**:
- Rejected with detailed error message
- Written to `reconfigure/error.txt` (bulk updates)
- Reported via API response (direct updates)
- Original configuration preserved

### File System Security

**Directory Traversal Protection**:
- Agent IDs validated (alphanumeric, hyphens, underscores only)
- Path traversal attempts blocked (e.g., `../../../etc/passwd`)
- Config files restricted to `agent_configs_dir`

**File Permissions** (recommended):
```bash
chmod 700 /etc/linksense/agent-configs    # Server only
chmod 600 /etc/linksense/server.toml      # Protect API key
chmod 600 /var/lib/linksense/server.db    # Protect metrics
```

## 🏗️ Server Architecture

### Data Storage

Server uses SQLite for metrics storage:

**Aggregated Metrics Tables**:
- `agg_metric_ping` - Ping statistics from all agents
- `agg_metric_http_get` - HTTP timing from all agents
- `agg_metric_http_content` - Content checks from all agents
- `agg_metric_dns_query` - DNS queries from all agents
- `agg_metric_bandwidth` - Bandwidth tests from all agents
- `agg_metric_sql_query` - SQL query results from all agents

**Agent Tracking**:
- `agents` table - Agent registration and last-seen timestamps

**Schema**: Each metric table includes:
- `agent_id` - Agent identifier (foreign key)
- `task_name` - Task identifier
- `window_start`, `window_end` - Aggregation time window
- Statistical values: `min_*`, `max_*`, `avg_*`, `stddev_*`
- Counts: `success_count`, `failure_count`, `sample_count`

See [README_DATABASE.md](README_DATABASE.md) for complete schema.

### Bandwidth Test Coordination

Server coordinates bandwidth tests to prevent conflicts:

**Coordination Queue**:
```rust
// Conceptual model
BandwidthState {
    active_agent: Option<String>,
    queue: Vec<String>,
}

// Only one test at a time
// Other agents queued and notified
```

**Request Flow**:
1. Agent sends POST /api/v1/bandwidth_test
2. Server checks if test in progress
   - No: Approve (200 OK), set agent as active
   - Yes: Queue (202 Accepted), agent waits
3. Agent downloads test data via /api/v1/bandwidth_download
4. Test completes, agent removed from active state
5. Next agent in queue can proceed

**Timeout Handling**: Active tests automatically cleared after completion.

See [README_BANDWIDTH_IMPLEMENTATION.md](README_BANDWIDTH_IMPLEMENTATION.md) for details.

### Data Retention

Automatic cleanup based on `data_retention_days`:

```sql
-- Cleanup runs every cleanup_interval_hours (default: 24)
DELETE FROM agg_metric_ping
WHERE timestamp < datetime('now', '-30 days');

-- Applied to all metric tables
```

**Manual Cleanup**:
```bash
# Check database size
du -h /var/lib/linksense/server.db

# Manual vacuum (reclaim space)
sqlite3 /var/lib/linksense/server.db "VACUUM;"
```

## 🐛 Troubleshooting

### Server Won't Start

**Symptom**: Server fails to start or exits immediately

**Diagnosis**:
```bash
# Check configuration syntax
cat server.toml

# Verify port not in use
sudo netstat -tlnp | grep :8787

# Check permissions
ls -l server.toml
ls -ld agent-configs/

# Test with debug logging
RUST_LOG=debug ./target/release/server ./server.toml
```

**Solutions**:
- Fix TOML syntax errors in server.toml
- Change `listen_address` if port in use
- Ensure write permissions for database directory
- Create `agent_configs_dir` if missing

### Agents Can't Connect

**Symptom**: Agents report connection failures

**Diagnosis**:
```bash
# Test server endpoint
curl -v http://server:8787/api/v1/health

# Check firewall
sudo iptables -L -n | grep 8787
sudo firewall-cmd --list-ports

# Check server logs
grep ERROR logs/server.log
```

**Solutions**:
- Verify server running: `ps aux | grep server`
- Check firewall allows port 8787
- Ensure `listen_address` binds correctly (0.0.0.0 vs 127.0.0.1)
- Verify agents using correct `central_server_url`

### Authentication Failures

**Symptom**: Agents receive 403 Forbidden errors

**Diagnosis**:
```bash
# Check API key configuration
grep api_key server.toml

# Test with curl
curl -H "X-API-Key: test-key" \
     -H "X-Agent-ID: test-agent" \
     http://localhost:8787/api/v1/config/verify

# Check server logs
grep "authentication failed" logs/server.log
```

**Solutions**:
- Ensure agent `api_key` matches server `api_key` exactly
- Check for whitespace or special characters
- If using whitelist, verify agent ID in `agent_id_whitelist`
- Restart server after configuration changes

### Configuration Updates Not Applied

**Symptom**: Agents don't receive new configurations

**Diagnosis**:
```bash
# Check agent config exists
ls -l agent-configs/{agent_id}.toml

# Check file permissions
ls -l agent-configs/

# Verify bulk reconfiguration errors
cat reconfigure/error.txt

# Test config download
curl -H "X-API-Key: key" -H "X-Agent-ID: agent1" \
     http://localhost:8787/api/v1/config/verify
```

**Solutions**:
- Ensure config file named exactly `{agent_id}.toml`
- Verify TOML syntax in agent config file
- Check `reconfigure/error.txt` for validation errors
- Ensure server has read permissions on config files

### Database Performance Issues

**Symptom**: Slow metric ingestion or queries

**Diagnosis**:
```bash
# Check database size
du -h server.db

# Check table sizes
sqlite3 server.db "SELECT
  name,
  SUM(pgsize) as size
FROM dbstat
GROUP BY name
ORDER BY size DESC;"

# Check active connections
lsof | grep server.db
```

**Solutions**:
- Reduce `data_retention_days` if disk space limited
- Run manual VACUUM to reclaim space
- Increase `cleanup_interval_hours` to reduce cleanup overhead
- Consider archiving old metrics before deletion

### Rate Limit Issues

**Symptom**: Agents receiving 429 Too Many Requests

**Diagnosis**:
```bash
# Check rate limit config
grep rate_limit server.toml

# Monitor agent behavior
grep "429" logs/server.log
```

**Solutions**:
- Increase `rate_limit_max_requests` if legitimate traffic
- Increase `rate_limit_window_seconds` for larger window
- Investigate agents sending excessive requests
- Temporarily disable: `rate_limit_enabled = false`

## 📊 Monitoring Server Health

### Log Files

Logs written to `./logs/server.log` (structured JSON):

```bash
# View recent logs
tail -f logs/server.log

# Filter by level
grep '"level":"ERROR"' logs/server.log

# Filter by endpoint
grep '"/api/v1/metrics"' logs/server.log

# Count requests per agent
grep agent_id logs/server.log | cut -d'"' -f8 | sort | uniq -c
```

### Database Queries

```bash
# Agent status
sqlite3 server.db <<EOF
SELECT agent_id, last_seen, total_metrics_received
FROM agents
ORDER BY last_seen DESC;
EOF

# Recent metrics
sqlite3 server.db <<EOF
SELECT agent_id, task_name, window_start, avg_rtt_ms
FROM agg_metric_ping
ORDER BY window_start DESC
LIMIT 10;
EOF

# Database statistics
sqlite3 server.db <<EOF
SELECT
  (SELECT COUNT(*) FROM agg_metric_ping) as ping_metrics,
  (SELECT COUNT(*) FROM agg_metric_http_get) as http_metrics,
  (SELECT COUNT(*) FROM agents) as agent_count;
EOF
```

### System Metrics

```bash
# Server process status
ps aux | grep server
top -p $(pgrep -f "server.*toml")

# Network connections
netstat -an | grep :8787

# Disk usage
df -h | grep linksense
du -sh /var/lib/linksense/

# File descriptors
lsof -p $(pgrep -f "server.*toml") | wc -l
```

### Health Monitoring

Implement external health checks:

```bash
# Simple availability check
curl -f http://localhost:8787/ || alert "Server down"

# Endpoint response time
curl -w "%{time_total}\n" -o /dev/null -s http://localhost:8787/

# Agent check-in monitoring
sqlite3 server.db "SELECT agent_id FROM agents
  WHERE last_seen < datetime('now', '-5 minutes');"
```

## 🔒 Security Best Practices

1. **API Key Management**:
   - Generate strong, random API keys (32+ characters)
   - Rotate keys periodically
   - Never commit keys to version control
   - Use environment variables for CI/CD

2. **Network Security**:
   - Use reverse proxy (nginx, Apache) for HTTPS
   - Implement TLS/SSL for agent-server communication
   - Restrict access via firewall rules
   - Consider VPN for agent connections

3. **File Permissions**:
   ```bash
   chmod 600 server.toml                  # Protect API key
   chmod 700 agent-configs/               # Restrict config access
   chmod 600 /var/lib/linksense/server.db # Protect metrics
   chown monitoring:monitoring /etc/linksense -R
   ```

4. **Agent Whitelist**:
   - Use whitelist in production environments
   - Only add verified agent IDs
   - Review and audit regularly
   - Document authorized agents

5. **Rate Limiting**:
   - Enable rate limiting in production
   - Monitor for rate limit violations
   - Investigate agents hitting limits
   - Adjust limits based on legitimate usage

6. **Database Security**:
   - Regular backups of server.db
   - Encrypt backups at rest
   - Restrict database file access
   - Monitor database size and growth

7. **Logging**:
   - Enable appropriate log levels
   - Rotate logs regularly
   - Monitor for authentication failures
   - Alert on suspicious activity

8. **System Hardening**:
   - Run server as non-root user
   - Use systemd security features
   - Keep Rust toolchain updated
   - Apply OS security patches

## 📚 See Also

- [README.md](README.md) - Project overview and architecture
- [README_AGENT.md](README_AGENT.md) - Agent setup and configuration
- [README_DATABASE.md](README_DATABASE.md) - Database schema details
- [README_BANDWIDTH_IMPLEMENTATION.md](README_BANDWIDTH_IMPLEMENTATION.md) - Bandwidth test coordination
- [README_RECONFIGURE_FEATURE.md](README_RECONFIGURE_FEATURE.md) - Bulk reconfiguration details
