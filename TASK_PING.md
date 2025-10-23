# ICMP Ping Task

The **Ping** task implements ICMP echo request/reply testing, the most fundamental network connectivity diagnostic tool. It measures network reachability and round-trip time (RTT) latency to remote hosts using the ICMP protocol.

## Implementation Details

### Crate: `ping-async`
**Link**: [ping-async on crates.io](https://crates.io/crates/ping-async)

This task uses the **`ping-async`** crate, an unprivileged async ICMP ping library built on Tokio.

**Key Characteristics**:
- **Fully Async**: Non-blocking I/O using Tokio runtime
- **Low Overhead**: Minimal memory footprint (~few KB per ping)
- **Raw Sockets**: Direct ICMP packet construction (requires privileges)
- **High Concurrency**: Can handle hundreds of concurrent ping targets efficiently
- **IPv4 and IPv6**: Supports both protocols

**Consequences**:
- ✅ **Scalability**: Async design allows monitoring 100+ targets simultaneously without thread overhead
- ✅ **Performance**: Non-blocking execution means one slow target doesn't block others
- ✅ **Resource Efficient**: Very low CPU and memory usage even with frequent pings
- ✅ **Production Ready**: Stable library used in production monitoring systems
- ⚠️ **Privilege Requirements**: Requires CAP_NET_RAW capability or root (see System Requirements)
- ⚠️ **No System Cache**: Bypasses system resolver, tests actual network path

## Configuration

### Basic Configuration with IP Address

```toml
[[tasks]]
type = "ping"
name = "Google DNS Ping"
schedule_seconds = 10
host = "8.8.8.8"              # Direct IP address
```

### Basic Configuration with Hostname

```toml
[[tasks]]
type = "ping"
name = "Google Website Ping"
schedule_seconds = 10
host = "google.com"           # Hostname - will be resolved automatically
```

### Advanced Configuration

```toml
[[tasks]]
type = "ping"
name = "Production Gateway Ping"
schedule_seconds = 5
host = "10.0.1.1"
timeout_seconds = 2          # Override default 1s timeout
timeout = 3                   # Task-level timeout override (optional)
target_id = "datacenter-us-east"  # Optional grouping identifier
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"ping"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between ping tests (seconds) |
| `host` | string | ✅ | - | Target hostname or IP address (IPv4/IPv6). If hostname, DNS resolution is performed before each ping. |
| `timeout_seconds` | integer | ❌ | 5 | ICMP response timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "datacenter-1", "prod-servers") |

### Host Parameter Behavior

The `host` parameter accepts both IP addresses and hostnames:

- **IP Address** (e.g., `"8.8.8.8"`, `"2001:4860:4860::8888"`): Used directly for ICMP ping
- **Hostname** (e.g., `"google.com"`, `"internal-server.local"`): Resolved via DNS before each ping attempt

When using hostnames:
- DNS resolution is performed using `tokio::net::lookup_host`
- The first resolved IP address is used for the ping
- DNS resolution failures are recorded as failed ping attempts with error details
- Both the resolved IP and original hostname are stored in metrics
- DNS resolution time is NOT included in RTT measurement

### Configuration Examples

#### Monitor Multiple DNS Servers
```toml
[[tasks]]
type = "ping"
name = "Cloudflare DNS"
schedule_seconds = 10
host = "1.1.1.1"

[[tasks]]
type = "ping"
name = "Google DNS"
schedule_seconds = 10
host = "8.8.8.8"

[[tasks]]
type = "ping"
name = "Quad9 DNS"
schedule_seconds = 10
host = "9.9.9.9"
```

#### High-Frequency Latency Monitoring
```toml
[[tasks]]
type = "ping"
name = "Primary Gateway - Critical"
schedule_seconds = 2          # Check every 2 seconds
host = "192.168.1.1"
timeout_seconds = 1
```

#### Geographic Performance Testing
```toml
[[tasks]]
type = "ping"
name = "US East Endpoint"
schedule_seconds = 30
host = "us-east.example.com"

[[tasks]]
type = "ping"
name = "EU West Endpoint"
schedule_seconds = 30
host = "eu-west.example.com"

[[tasks]]
type = "ping"
name = "APAC Endpoint"
schedule_seconds = 30
host = "apac.example.com"
```

## Metrics

### Raw Metrics (`raw_metric_ping`)

Captured for each individual ping execution:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when ping was executed |
| `rtt_ms` | REAL | Round-trip time in milliseconds (NULL if failed) |
| `success` | BOOLEAN | Whether ping succeeded (1) or failed (0) |
| `error` | TEXT | Error message if ping failed (NULL on success) |
| `ip_address` | TEXT | IP address that was actually pinged (resolved if hostname was used) |
| `domain` | TEXT | Original hostname if `host` config was a domain (NULL if IP address) |
| `target_id` | TEXT | Optional target identifier from configuration (NULL if not specified) |


### Aggregated Metrics (`agg_metric_ping`)

60-second statistical summary of raw measurements:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of ping attempts in period |
| `avg_latency_ms` | REAL | Mean RTT of successful pings |
| `max_latency_ms` | REAL | Maximum RTT observed |
| `min_latency_ms` | REAL | Minimum RTT observed |
| `packet_loss_percent` | REAL | Percentage of failed pings (0-100) |
| `successful_pings` | INTEGER | Count of successful pings |
| `failed_pings` | INTEGER | Count of failed pings |
| `domain` | TEXT | Original hostname if `host` was a domain (first occurrence in period, NULL if IP) |
| `target_id` | TEXT | Optional target identifier from configuration (first occurrence in period, NULL if not specified) |



## Design Philosophy

### What It Does

Sends ICMP Echo Request packets to a target host and measures:
- **Reachability**: Can the target host be reached?
- **Latency**: How long does it take for packets to make the round trip?
- **Packet Loss**: What percentage of packets fail to return?
- **DNS Resolution**: Automatically resolves hostnames to IP addresses before pinging

### Strong Sides

1. **Universal Support**: ICMP is supported by virtually all networked devices (routers, switches, servers, workstations)
2. **Low Overhead**: Minimal bandwidth consumption, ideal for frequent monitoring
3. **Network Layer Testing**: Tests connectivity at Layer 3, bypassing application layer issues
4. **Fast Execution**: Typically completes in milliseconds, enabling high-frequency checks
5. **Baseline Metric**: RTT provides a fundamental baseline for network performance
6. **Flexible Targeting**: Accepts both IP addresses and hostnames with automatic DNS resolution

### Typical Use Cases

- **Availability Monitoring**: Track if critical infrastructure is reachable
- **Latency Baseline**: Establish normal RTT ranges for anomaly detection
- **Network Path Health**: Monitor connectivity to gateways, DNS servers, external services
- **SLA Verification**: Ensure network providers meet latency commitments
- **Geographic Performance**: Compare response times from different network locations
- **Early Warning**: Detect network degradation before application-level failures occur

### Limitations

- **ICMP Blocking**: Some networks/firewalls block ICMP, causing false negatives
- **Different Priority**: Routers may deprioritize ICMP vs. production traffic
- **No Application Context**: Success doesn't guarantee application availability
- **Requires Privileges**: Raw ICMP sockets require elevated permissions on most systems

## System Requirements

### Linux Permissions

ICMP ping requires raw socket privileges. Choose one option:

**Option 1: Add user to ping group (recommended)**
```bash
# Add monitoring user to ping group
sudo usermod -aG ping monitoring-agent

# Verify group membership
groups monitoring-agent
```

**Option 2: Grant CAP_NET_RAW capability**
```bash
# Grant capability to agent binary
sudo setcap cap_net_raw+ep /path/to/agent

# Verify capability
getcap /path/to/agent
```

**Option 3: Run as root (not recommended for production)**
```bash
sudo /path/to/agent /path/to/config
```

### Firewall Considerations

**Outbound ICMP (on agent host):**
```bash
# Allow outbound ICMP echo requests (iptables)
sudo iptables -A OUTPUT -p icmp --icmp-type echo-request -j ACCEPT
sudo iptables -A INPUT -p icmp --icmp-type echo-reply -j ACCEPT
```

**Target Host:**
- Must respond to ICMP echo requests
- Check firewall rules if pings fail
- Some cloud providers block ICMP by default (verify security group rules)

## Performance Characteristics

### Resource Usage
- **CPU**: Negligible (~0.1% per task)
- **Memory**: ~1KB per execution
- **Network**: 64-84 bytes per ping (32-42 bytes each direction)
- **Disk I/O**: Minimal (batch writes to SQLite)

### Execution Time
- **Typical**: 10-50ms (local network)
- **WAN**: 50-200ms (internet hosts)
- **Timeout**: Configurable (default 5s)

## Troubleshooting

### Common Issues

#### "Permission denied" Errors
**Symptom**: Task fails with permission errors
**Solution**: Ensure raw socket privileges (see System Requirements above)

#### "Request timeout" Failures
**Symptom**: All pings timeout but host is reachable
**Causes**:
- Firewall blocking ICMP on source or destination
- Host configured to ignore ICMP (security policy)
- Network congestion causing packet loss
**Debugging**:
```bash
# Test manually with system ping
ping -c 4 <target_host>

# Check firewall rules
sudo iptables -L -n -v | grep icmp
```

#### High Latency Spikes
**Symptom**: Occasional very high RTT values
**Causes**:
- Network congestion
- Route flapping
- CPU scheduling delays on agent host
**Solution**: Review `max_latency_ms` trends, consider increasing sample frequency

#### Hostname Resolution Failures
**Symptom**: "DNS resolution failed" errors in ping task
**Causes**:
- DNS server unreachable or misconfigured
- Invalid hostname or typo in configuration
- Network connectivity issues preventing DNS queries
- Hostname does not exist or has expired
**Solution**:
- Verify hostname with `nslookup` or `dig` command
- Check `/etc/resolv.conf` for correct DNS server configuration
- Use IP addresses for critical monitoring to eliminate DNS dependency
- Add separate DNS monitoring task to track DNS health independently
- Review error messages in raw metrics for specific DNS failure details

### Debugging Tips

**Enable verbose logging:**
```bash
RUST_LOG=debug ./agent /path/to/config
```

**Check raw metrics for patterns:**
```sql
SELECT timestamp, rtt_ms, success, error
FROM raw_metric_ping
WHERE task_name = 'Target Name'
ORDER BY timestamp DESC
LIMIT 100;
```

**Validate configuration:**
```bash
# Test ping manually
ping -c 10 -i 1 <host>

# Compare with agent behavior
```

## Best Practices

1. **Set Appropriate Schedules**:
   - Critical paths: 5-10 seconds
   - General monitoring: 30-60 seconds
   - Avoid < 2 seconds unless necessary

2. **Choose Between IP and Hostname Based on Use Case**:
   - **Use IP Addresses for**:
     - Critical infrastructure monitoring (eliminates DNS as failure point)
     - High-frequency checks (avoids DNS lookup overhead)
     - Testing specific network paths
     - When DNS resolution is monitored separately
   - **Use Hostnames for**:
     - Cloud services where IPs may change (AWS, Azure, GCP)
     - Load-balanced services (benefits from DNS round-robin)
     - CDN endpoints (DNS directs to nearest edge)
     - Monitoring DNS+connectivity as a combined health check

3. **Monitor Both Ways**:
   - Test gateway (internal)
   - Test external service (internet connectivity)
   - Helps isolate failure location

4. **Combine with Application Checks**:
   - Ping confirms reachability
   - HTTP/DNS tasks confirm service health
   - Together provide complete picture

5. **Set Realistic Timeouts**:
   - LAN: 1-2 seconds
   - Internet: 5 seconds (default)
   - Satellite/high-latency: 10+ seconds
   - **Note**: Timeout is now properly enforced - operations will not hang indefinitely

6. **Leverage Domain Tracking**:
   - Query `domain` field in aggregated metrics to identify which hostnames are being monitored
   - Use for auditing and documentation purposes
   - Compare IP changes over time by joining raw metrics with domain field

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- Task Types:
  - [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP availability monitoring
  - [TASK_DNS.md](TASK_DNS.md) - DNS resolution testing
  - [TASK_BANDWIDTH.md](TASK_BANDWIDTH.md) - Bandwidth measurement
