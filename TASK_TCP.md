# TCP Connection Task

The **TCP Connection** task implements the most fundamental network connectivity test - raw TCP socket connection to any host:port. It measures pure TCP connection establishment time without any protocol overhead, making it ideal for testing basic network reachability and port availability.

## Implementation Details

### Built on Tokio Async TCP (via shared TLS module)
**Component**: Reuses `get_tcp_timing()` from `task_tls` module

This task uses **Tokio's async TCP implementation** for low-level socket connections.

**Key Characteristics**:
- **Raw TCP Sockets**: Uses `tokio::net::TcpStream::connect()` directly
- **No Protocol Layer**: Pure socket connection, no HTTP/TLS/application protocol
- **Async/Non-blocking**: Built on Tokio runtime, zero thread overhead
- **DNS Resolution**: Uses standard library `to_socket_addrs()` (system resolver)
- **IPv6 Support**: Fully supports IPv6 addresses and automatic IPv4/IPv6 resolution
- **Connection-Only**: Establishes connection then immediately closes it
- **Precise Timing**: Measures exact TCP 3-way handshake duration

**Consequences**:
- ✅ **Extremely Fast**: No protocol overhead, just TCP handshake
- ✅ **Minimal Resource Usage**: ~10-20KB memory per connection (includes socket buffers)
- ✅ **Universal**: Works with any TCP service (SSH, databases, custom protocols)
- ✅ **Port Testing**: Perfect for checking if ports are open/firewalled
- ✅ **High Concurrency**: Can test 200+ ports simultaneously
- ⚠️ **No Protocol Validation**: Only tests TCP layer, not application health
- ⚠️ **Connection Refused vs Timeout**: Different failure modes reveal firewall vs. service issues
- ⚠️ **No Content**: Cannot verify service is responding correctly (just that port accepts connections)

**Why TCP-Only Testing?**
- Many monitoring scenarios need to verify port availability without caring about the protocol
- Faster than protocol-specific tests (no handshake, no request/response)
- Can monitor services where you don't know/control the application protocol
- Isolates network/firewall issues from application issues
- Useful baseline before attempting higher-level protocol tests

**TCP Connection Flow**:
```rust
1. Parse host:port string to socket address (supports IPv6 like "[2001:db8::1]:443")
2. Resolve hostname to IP (if needed) - uses first available address
3. Start timing
4. Attempt TCP connection (SYN → SYN-ACK → ACK)
5. Record connection time
6. Close connection immediately (explicit cleanup)
```

**Failure Modes**:
- **Connection Refused**: Port is closed, service not listening (fast failure)
- **Timeout**: Firewall blocking, network unreachable (slow failure)
- **DNS Failure**: Hostname doesn't resolve or invalid host format
- **Network Unreachable**: Routing issues

## Configuration

### Basic Configuration

```toml
[[tasks]]
type = "tcp"
name = "SSH Port Check"
schedule_seconds = 60
host = "server.example.com:22"
```

### Advanced Configuration

```toml
[[tasks]]
type = "tcp"
name = "Database Connection Check"
schedule_seconds = 30
host = "db.internal.example.com:5432"
timeout_seconds = 5
target_id = "postgres-primary"
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"tcp"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between checks (seconds) |
| `host` | string | ✅ | - | Target in format `host:port` (e.g., `"server.com:22"` or `"192.168.1.10:3306"`) |
| `timeout_seconds` | integer | ❌ | 5 | Connection timeout (seconds, enforced via Tokio timeout) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering (e.g., "production-db", "backup-ssh") |

### Configuration Examples

#### Monitor SSH Access
```toml
[[tasks]]
type = "tcp"
name = "Production SSH"
schedule_seconds = 60
host = "prod-server.example.com:22"
timeout_seconds = 5
target_id = "ssh-prod"
```

#### Database Connectivity Checks
```toml
# PostgreSQL
[[tasks]]
type = "tcp"
name = "PostgreSQL Primary"
schedule_seconds = 30
host = "db-primary.internal:5432"
timeout_seconds = 3
target_id = "postgres-primary"

# MySQL
[[tasks]]
type = "tcp"
name = "MySQL Replica"
schedule_seconds = 30
host = "db-replica.internal:3306"
timeout_seconds = 3
target_id = "mysql-replica"



#### Firewall Testing
```toml
# Test if firewall allows connection
[[tasks]]
type = "tcp"
name = "Firewall Test - External"
schedule_seconds = 120
host = "external-api.example.com:443"
timeout_seconds = 10

# Test internal network
[[tasks]]
type = "tcp"
name = "Firewall Test - Internal"
schedule_seconds = 120
host = "10.0.1.100:9000"
timeout_seconds = 3
```


## Metrics

### Raw Metrics (`raw_metric_tcp`)

Captured for each individual TCP connection attempt:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when connection was attempted |
| `connect_time_ms` | REAL | TCP connection establishment time (ms) - NULL if failed |
| `success` | BOOLEAN | Whether connection succeeded (1) or failed (0) |
| `error` | TEXT | Error message if connection failed (NULL on success) |
| `host` | TEXT | Host:port that was connected to |
| `target_id` | TEXT | Optional target identifier from configuration |


### Aggregated Metrics (`agg_metric_tcp`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of connection attempts in period |
| `avg_connect_time_ms` | REAL | Mean connection time (only successful connections) |
| `max_connect_time_ms` | REAL | Maximum connection time observed |
| `min_connect_time_ms` | REAL | Minimum connection time observed |
| `failure_percent` | REAL | Percentage of failed connections (0-100) |
| `successful_connections` | INTEGER | Count of successful connections |
| `failed_connections` | INTEGER | Count of failed connections |
| `host` | TEXT | Host:port being monitored |
| `target_id` | TEXT | Optional target identifier from configuration |


### Metrics Interpretation

#### Connection Time Analysis
- **< 1ms**: Local network (same machine/LAN)
- **1-10ms**: Fast local network
- **10-50ms**: Normal local/regional network
- **50-200ms**: Internet connection or congested network
- **> 200ms**: High latency, investigate network path

#### Success/Failure Patterns
- **100% success**: Healthy service and network
- **95-99% success**: Occasional network glitches (normal)
- **< 90% success**: Service or network issues
- **0% success**: Service down, port closed, or firewall blocking

#### Error Message Patterns
- **"Connection refused"**: Port is closed, service not running
  - Fast failure (< 1 second)
  - Firewall allows traffic but no service listening
- **"Connection timeout"**: Firewall blocking or network unreachable
  - Slow failure (matches timeout setting)
  - Packets being dropped
- **"Failed to parse host"**: Invalid host:port format or DNS resolution failure
- **"Failed to parse host: no addresses found"**: DNS resolved but no addresses returned

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_connect_time_ms > 100
  - failure_percent > 5
  - successful_connections < 90% of expected

CRITICAL:
  - avg_connect_time_ms > 500
  - failure_percent > 20
  - successful_connections == 0 (for 5 minutes)
```

## Design Philosophy

### What It Does

Attempts TCP connections to specified host:port combinations and measures:
- **Port Availability**: Is the port open and accepting connections?
- **Connection Latency**: How long does TCP handshake take?
- **Service Reachability**: Can we establish network connection?
- **Firewall Status**: Are connections being blocked or accepted?

### Strong Sides

1. **Universal Compatibility**: Works with any TCP-based service
2. **Lightweight**: Minimal overhead, fastest possible network test
3. **Firewall Detection**: Distinguishes between "refused" (service down) and "timeout" (firewall)
4. **Network Baseline**: Establishes pure network latency without protocol overhead
5. **Port Scanning**: Quickly verify which ports are accessible
6. **Pre-Protocol Testing**: Test connectivity before attempting application-level checks

### Typical Use Cases

- **SSH Monitoring**: Verify SSH port accessibility before attempting login
- **Database Connectivity**: Check database ports (PostgreSQL 5432, MySQL 3306, MongoDB 27017)
- **API Gateway Testing**: Verify API ports are open before HTTP checks
- **Load Balancer Health**: Check backend server port availability
- **Firewall Validation**: Confirm firewall rules allow required traffic
- **VPN Tunnel Status**: Verify VPN endpoints are reachable
- **Custom Protocol Services**: Monitor services using proprietary protocols
- **Network Baseline**: Measure pure network latency to various hosts
- **Service Discovery**: Identify which ports are open on a host
- **Failover Testing**: Monitor backup/standby servers

### Limitations

- **No Application Validation**: Only tests TCP layer, not service health
  - Port open doesn't mean service is working correctly
  - Use protocol-specific tasks (HTTP, TLS, SQL) for application health
- **No Authentication**: Cannot verify credentials or permissions
- **Connection-Only**: Doesn't send/receive data, just establishes connection
- **TCP Only**: Cannot test UDP services (DNS, SNMP, etc.)
- **Single Port**: Each task monitors one host:port combination

## Performance Characteristics

### Resource Usage
- **CPU**: Negligible (~0.1% per connection)
- **Memory**: ~10-20KB per connection (includes socket buffers)
- **Network**: Single SYN packet + ACK (< 100 bytes total)
- **Disk I/O**: Batch writes to SQLite

### Execution Time
- **LAN**: 0.5-5ms
- **Internet**: 20-100ms
- **Timeout**: 1-10 seconds (configurable)
- **Connection refused**: < 100ms (immediate rejection)

### IPv6 Support
- **Full Support**: IPv6 addresses like "[2001:db8::1]:443" work natively
- **Automatic Resolution**: System DNS resolver handles IPv4/IPv6 automatically
- **First Address**: Uses first available address from DNS resolution
- **Mixed Environments**: Works in pure IPv4, pure IPv6, and dual-stack networks

### Scalability
- Can monitor **200+ ports** simultaneously
- Extremely lightweight, minimal overhead
- Recommended schedule: 15-60 seconds per port

## Troubleshooting

### Common Issues

#### "Connection refused" Errors
**Symptom**: Connections consistently refused
**Causes**:
- Service not running
- Service crashed
- Port not configured to listen
- Service listening on different interface (localhost vs. 0.0.0.0)
**Solutions**:
```bash
# Check if service is running
systemctl status sshd
systemctl status postgresql

# Check which ports are listening
netstat -tlnp | grep :22
ss -tlnp | grep :5432

# Verify service configuration
sudo sshd -T | grep Port
```

#### "Connection timeout" Errors
**Symptom**: Connections timeout after configured seconds
**Causes**:
- Firewall blocking traffic
- Network routing issues
- Service host unreachable
- Wrong IP/hostname
**Solutions**:
```bash
# Test connectivity manually
nc -zv server.example.com 22
telnet server.example.com 22

# Check firewall rules
sudo iptables -L -n -v
sudo firewall-cmd --list-all

# Trace network path
traceroute server.example.com
mtr server.example.com
```

#### "Failed to resolve host" Errors
**Symptom**: DNS resolution fails
**Causes**:
- Hostname doesn't exist
- DNS server unreachable
- DNS misconfiguration
**Solutions**:
```bash
# Test DNS resolution
dig server.example.com
nslookup server.example.com

# Use IP address instead
host = "192.168.1.10:22"  # Direct IP
```

#### High Connection Latency
**Symptom**: `connect_time_ms` consistently high
**Causes**:
- Network congestion
- High latency network path
- Server under load
- Rate limiting/throttling
**Solutions**:
```bash
# Measure network latency
ping server.example.com
mtr --report server.example.com

# Compare with ICMP ping task
# TCP may be slower due to additional handshake
```

#### Intermittent Failures
**Symptom**: Alternating success/failure
**Causes**:
- Load balancer rotating between healthy/unhealthy backends
- Flaky network connection
- Service restarting
- Resource exhaustion on target
**Solutions**:
- Increase sample count (more frequent checks)
- Check server logs for service restarts
- Monitor server resources (CPU, memory, connections)

### Debugging Tips

**Test connection manually:**
```bash
# Test with netcat
nc -zv server.example.com 22

# Test with telnet
telnet server.example.com 22

# Test with timeout
timeout 5 bash -c 'cat < /dev/null > /dev/tcp/server.example.com/22'

# Test IPv6 connectivity
nc -6v [2001:db8::1]:443
```

**Analyze connection patterns:**
```sql
SELECT
  timestamp,
  connect_time_ms,
  success,
  error
FROM raw_metric_tcp
WHERE task_name = 'SSH Check'
ORDER BY timestamp DESC
LIMIT 100;
```

**Check for network issues:**
```sql
SELECT
  period_start,
  avg_connect_time_ms,
  failure_percent,
  successful_connections,
  failed_connections
FROM agg_metric_tcp
WHERE task_name = 'SSH Check'
  AND period_start > (strftime('%s', 'now') - 86400)
ORDER BY period_start DESC;
```

**Compare with ping task:**
```sql
-- TCP vs ICMP latency comparison
SELECT
  tcp.period_start,
  tcp.avg_connect_time_ms as tcp_ms,
  ping.avg_latency_ms as ping_ms,
  (tcp.avg_connect_time_ms - ping.avg_latency_ms) as overhead_ms
FROM agg_metric_tcp tcp
JOIN agg_metric_ping ping
  ON tcp.period_start = ping.period_start
WHERE tcp.task_name = 'SSH Check'
  AND ping.task_name = 'SSH Host Ping'
ORDER BY tcp.period_start DESC
LIMIT 20;
```

## Best Practices

1. **Use Before Protocol Tests**:
   # Test TCP connectivity before attempting HTTP/TLS/SQL
   # Faster feedback on network issues
   # Isolates network from application problems

8. **Consider IPv6 in Monitoring**:
   ```toml
   # IPv4 monitoring
   [[task]]
   type = "tcp"
   name = "IPv4 API Check"
   [task.params]
   host = "api.example.com:443"

   # IPv6 monitoring (if service supports it)
   [[task]]
   type = "tcp"
   name = "IPv6 API Check"
   [task.params]
   host = "[2001:db8::1]:443"
   ```

2. **Set Appropriate Timeouts**:
   - Local network: 1-3 seconds
   - Internet: 5-10 seconds
   - Faster timeout = faster failure detection

3. **Monitor Critical Ports**:
   - SSH (22) for remote access
   - Database ports (5432, 3306, 27017)
   - API endpoints (80, 443, 8080)
   - Custom application ports

4. **Combine with Protocol Tests**:
   ```toml
   # TCP layer
   [[task]]
   type = "tcp"
   name = "API Port Check"
   schedule_seconds = 30
   [task.params]
   host = "api.example.com:443"

   # Application layer
   [[task]]
   type = "http_get"
   name = "API Health Check"
   schedule_seconds = 60
   url = "https://api.example.com/health"
   ```

5. **Use target_id for Grouping**:
   - Group related services: `target_id = "database-cluster"`
   - Track environments: `target_id = "production"` vs `target_id = "staging"`
   - Easier filtering in queries and dashboards

6. **Test Load Balancer Backends Individually**:
   ```toml
   [[task]]
   type = "tcp"
   name = "Backend Server 1"
   [task.params]
   host = "10.0.1.10:8080"
   target_id = "backend-pool"

   [[task]]
   type = "tcp"
   name = "Backend Server 2"
   [task.params]
   host = "10.0.1.11:8080"
   target_id = "backend-pool"
   ```

7. **Establish Baselines**:
   - Monitor well-known services (8.8.8.8:53) for network baseline
   - Compare internal vs external latency
   - Track trends over time


## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_PING.md](TASK_PING.md) - ICMP ping monitoring (network layer)
- [TASK_TLS.md](TASK_TLS.md) - TLS handshake monitoring (adds TLS layer)
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP monitoring (adds HTTP layer)
- [TASK_DNS.md](TASK_DNS.md) - DNS resolution monitoring
