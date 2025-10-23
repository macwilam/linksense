# Bandwidth Test Task

The **Bandwidth** task measures actual throughput between the agent and the central server by downloading a test file. Unlike other tasks that measure latency, bandwidth testing reveals the data transfer capacity of the network connection.

## Implementation Details

### Crate: `reqwest` (for HTTP download)
**Link**: [reqwest on crates.io](https://crates.io/crates/reqwest)

This task uses **`reqwest`** to download a test file directly from the central server.

**Key Characteristics**:
- **Direct Server Connection**: HTTP GET to `/api/v1/bandwidth_test` endpoint
- **Streaming Download**: Reads response body in chunks, discards data (not saved to disk)
- **No Decompression**: Transfer-Encoding disabled to measure raw bandwidth
- **Async I/O**: Non-blocking download via Tokio
- **Timing Measurement**: Measures download duration with high precision

**Consequences**:
- ✅ **Accurate Throughput**: Measures actual data transfer rate over the network
- ✅ **Production Path**: Tests same network path as real application traffic
- ✅ **No Disk I/O**: Data discarded during download, no write overhead
- ✅ **Async Efficiency**: Non-blocking download doesn't tie up threads
- ⚠️ **Server Dependency**: Requires central server to be available
- ⚠️ **Network Impact**: Consumes bandwidth during test (configurable size)
- ⚠️ **Single Direction**: Only tests download (agent ingress), not upload

**Test File Generation**:
- Server generates test data in memory using zero-filled bytes
- Generated in 64 KB chunks to avoid large memory allocations
- Size configured server-side only: `bandwidth_test_size_mb` in `server.toml`
- Data generated on-demand per request (not cached)
- Content-Encoding disabled (no gzip) to measure raw transfer

**Bandwidth Calculation**:
```rust
1. Record start_time
2. Download test file from server (streaming, discard chunks)
3. Record end_time and bytes_downloaded
4. duration_ms = end_time - start_time
5. bandwidth_mbps = (bytes_downloaded * 8) / (duration_ms / 1000) / 1_000_000
```

**Timeout Handling**:
```rust
// Two-phase timeout approach
let permission_timeout = min(timeout_seconds, 10);  // Max 10s for permission request
let download_timeout = timeout_seconds;              // Full timeout for download phase
```

**Server Coordination**:
The server maintains a sophisticated FIFO queue mechanism:
```rust
// Server-side pseudocode (BandwidthTestManager)
if current_test.is_none() {
    current_test = Some(agent_id, start_time)
    return BandwidthTestAction::Proceed { data_size_bytes }
} else if current_test.agent == agent_id {
    return BandwidthTestAction::Proceed { data_size_bytes }  // Already running
} else {
    waiting_queue.push(agent_id)
    delay_seconds = 60 + (waiting_queue.len() * 30)  // Base 60s + 30s per queued agent
    return BandwidthTestAction::Delay { delay_seconds }
}

// Auto-cleanup every request
if current_test.elapsed() > 120s {
    current_test = None
    auto_start_next_in_queue()
}
```

This prevents multiple agents from:
- Overwhelming server upload bandwidth
- Interfering with each other's measurements
- Causing inaccurate results

**Queue features**:
- FIFO ordering ensures fairness
- Automatic advancement when tests complete or timeout (120s)
- Intelligent delay suggestions based on queue position
- Old queue entries (>300s) automatically cleaned up

**Why Direct Server Connection?**
- Testing agent→server path is most relevant (same as metrics upload)
- No external dependencies (doesn't rely on third-party speed test services)
- Server controls test file size based on expected agent capabilities
- Simpler authentication (uses existing API key mechanism)

## Configuration

### Basic Configuration

```toml
[[tasks]]
type = "bandwidth"
name = "WAN Link Throughput"
schedule_seconds = 300          # Every 5 minutes (minimum 60s)
timeout_seconds = 60
```

### Advanced Configuration

```toml
[[tasks]]
type = "bandwidth"
name = "ISP Speed Test"
schedule_seconds = 600          # Every 10 minutes
timeout_seconds = 120           # Allow up to 2 minutes for download
timeout = 120                   # Task-level timeout (optional)
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"bandwidth"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between tests (≥60 seconds, enforced) |
| `timeout_seconds` | integer | ❌ | 60 | Download timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "wan-primary", "branch-office") |

**Important**: Test file size is configured **server-side only** via `bandwidth_test_size_mb` in `server.toml` (default: 10MB).

### Server Configuration

In `server.toml`:
```toml
bandwidth_test_size_mb = 10     # Test file size (MB)
```

Size recommendations:
- **1-5 MB**: Fast networks (>100 Mbps), frequent testing
- **10-25 MB**: General purpose (10-100 Mbps)
- **50-100 MB**: Slow links (<10 Mbps), infrequent testing

### Configuration Examples

#### ISP Speed Monitoring
```toml
[[tasks]]
type = "bandwidth"
name = "Internet Connection Speed"
schedule_seconds = 600          # Every 10 minutes
timeout_seconds = 90
```

#### Branch Office Link Monitoring
```toml
[[tasks]]
type = "bandwidth"
name = "Branch Office - Chicago"
schedule_seconds = 300          # Every 5 minutes
timeout_seconds = 120           # Allow 2 min for slow links
```

#### High-Frequency Testing (Development/Testing Only)
```toml
[[tasks]]
type = "bandwidth"
name = "Dev Environment Test"
schedule_seconds = 60           # Minimum allowed
timeout_seconds = 30
```

**Warning**: High-frequency bandwidth tests generate significant load!

#### Multi-Agent Deployment (Same Server)
```toml
# Agent 1 config
[[tasks]]
type = "bandwidth"
name = "Site A Bandwidth"
schedule_seconds = 300

# Agent 2 config
[[tasks]]
type = "bandwidth"
name = "Site B Bandwidth"
schedule_seconds = 300

# Server coordinates tests automatically - no manual offset needed
```

## Metrics

### Raw Metrics (`raw_metric_bandwidth`)

Captured for each individual bandwidth test:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when test started |
| `bandwidth_mbps` | REAL | Measured download speed (Mbps) - NULL if failed |
| `duration_ms` | REAL | Download duration (milliseconds) - NULL if failed |
| `bytes_downloaded` | INTEGER | Total bytes successfully downloaded - NULL if failed |
| `success` | BOOLEAN | Whether test succeeded (1) or failed (0) |
| `error` | TEXT | Error message if test failed (NULL on success) |
| `target_id` | TEXT | Optional target identifier from task configuration |

**Bandwidth Calculation**:
```
bandwidth_mbps = (bytes_downloaded * 8) / (duration_ms / 1000) / 1,000,000
```


### Aggregated Metrics (`agg_metric_bandwidth`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of tests in period |
| `avg_bandwidth_mbps` | REAL | Mean download speed (Mbps) |
| `max_bandwidth_mbps` | REAL | Peak download speed observed |
| `min_bandwidth_mbps` | REAL | Lowest download speed observed |
| `successful_tests` | INTEGER | Count of successful tests |
| `failed_tests` | INTEGER | Count of failed tests |
| `target_id` | TEXT | Optional target identifier from task configuration |


Note: With typical schedules (300-600s), most aggregation periods contain 0-1 samples.

### Metrics Interpretation

#### Bandwidth Ranges
- **> 100 Mbps**: Fast connection (fiber, enterprise WAN)
- **10-100 Mbps**: Medium speed (cable, DSL, business broadband)
- **1-10 Mbps**: Slow connection (rural broadband, congested network)
- **< 1 Mbps**: Very slow (dial-up era, severe congestion)

#### Performance Patterns
- **Stable bandwidth**: Healthy, consistent network
- **Gradual degradation**: Link saturation, capacity issues
- **Sudden drops**: Network event, routing change, QoS change
- **High variance**: Network congestion, shared bandwidth contention

#### Success Rate
- **100%**: Reliable test execution
- **< 90%**: Coordination issues, server overload, or network instability
- **0%**: Server unreachable or persistent queue contention

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_bandwidth_mbps < 80% of baseline
  - avg_bandwidth_mbps < contracted SLA speed
  - High variance (max - min > 50% of avg)

CRITICAL:
  - avg_bandwidth_mbps < 50% of baseline
  - successful_tests == 0 (for 30 minutes)
  - avg_bandwidth_mbps < minimum operational threshold
```

**Example SLA Alert**:
```
Contracted: 100 Mbps
WARNING: avg_bandwidth_mbps < 80 Mbps (80% of SLA)
CRITICAL: avg_bandwidth_mbps < 50 Mbps (50% of SLA)
```

## Design Philosophy

### What It Does

Downloads a test file from the central server and measures:
- **Download Speed**: Actual throughput in Mbps
- **Transfer Time**: How long the download takes
- **Data Volume**: Bytes successfully downloaded
- **Connection Stability**: Whether full transfer completes

### Strong Sides

1. **Real Throughput Measurement**: Tests actual data transfer, not just packet RTT
2. **Bottleneck Detection**: Identifies bandwidth constraints in network path
3. **Baseline Establishment**: Create performance baselines for network links
4. **Degradation Alerting**: Detect when bandwidth drops below SLA
5. **Server Coordination**: Prevents multiple agents from overwhelming server simultaneously
6. **Production Simulation**: Tests same network path as actual application traffic

### Typical Use Cases

- **WAN Link Monitoring**: Measure throughput on MPLS, VPN, or internet connections
- **ISP SLA Verification**: Confirm contracted bandwidth is delivered
- **Network Capacity Planning**: Identify when links approach saturation
- **CDN Performance**: Test edge server download speeds
- **Quality of Service (QoS) Validation**: Verify traffic prioritization works
- **Remote Site Monitoring**: Ensure branch offices have adequate bandwidth
- **Bandwidth Degradation Detection**: Alert when speeds drop unexpectedly

### Limitations

- **Server Resource Intensive**: Generates significant network/disk load on server
- **Coordination Required**: Server queues tests to prevent overwhelming resources
- **Minimum Schedule**: Must run ≥60 seconds apart (enforced by validation)
- **Download Only**: Tests ingress bandwidth to agent (not egress/upload)
- **Shared Network Impact**: May consume bandwidth needed by production traffic
- **No Path Isolation**: Tests complete network path (can't isolate specific segments)

## How It Works

### Test Coordination Flow

```
1. Agent: POST /api/v1/bandwidth_test
   - Headers: X-API-Key, Content-Type: application/json
   - Body: {"agent_id": "agent1", "timestamp_utc": "..."}
   ↓
2. Server: Validates & checks queue
   - Validates: API key, agent ID format, whitelist, rate limits
   - If slot available → Returns action: "proceed", data_size_bytes
   - If test running → Returns action: "delay", delay_seconds (60s + queue position)
   ↓
3a. If "proceed": Agent downloads from /api/v1/bandwidth_download?agent_id=X
   - Server validates agent has active test
   - Measures time and bytes downloaded
   - Calculates Mbps
   ↓
3b. If "delay": Agent waits suggested delay_seconds and retries step 1
   ↓
4. Agent: Stores metrics locally (success or failure)
   ↓
5. Agent: Sends aggregated metrics to server
   ↓
6. Server: Auto-cleanup after 120s timeout (releases slot for next agent)
```

### Coordination Mechanism

The server maintains a bandwidth test queue (`BandwidthTestManager`) to prevent:
- **Resource Exhaustion**: Too many simultaneous downloads
- **Network Saturation**: Overwhelming server uplink
- **Inaccurate Results**: Tests interfering with each other

**Queue Behavior**:
- Only **one agent** can download test file at a time
- **FIFO queue**: Waiting agents are queued in order of arrival
- **Auto-advancement**: Next agent in queue auto-starts when current test completes/times out
- **Intelligent delays**: Server suggests delay based on queue position (60s + 30s per agent)
- **Automatic cleanup**: Test slots released after 120s timeout
- **Expired queue entries**: Removed after 300s (max_delay)
- **No "queue full" errors**: Server always accepts requests and returns delay if needed

**Important**: The server responds with JSON action field, not HTTP error codes:
- `{"action": "proceed", "data_size_bytes": 10485760}` - Start download now
- `{"action": "delay", "delay_seconds": 90}` - Wait 90s and retry

## Performance Characteristics

### Resource Usage

**Agent**:
- **CPU**: Low (1-5% during download)
- **Memory**: ~(test_file_size + 10MB) buffer
- **Network**: Full test file download (e.g., 10 MB)
- **Disk**: Minimal (metrics only, test file not saved)

**Server**:
- **CPU**: Low (5-10% per concurrent test)
- **Memory**: ~(test_file_size * concurrent_tests)
- **Network**: Significant (test_file_size * tests_per_minute)
- **Disk**: Minimal (test file cached in memory)

### Execution Time
- **100 Mbps link, 10 MB file**: ~1 second
- **10 Mbps link, 10 MB file**: ~10 seconds
- **1 Mbps link, 10 MB file**: ~100 seconds
- **Timeout**: Two-phase approach
  - Permission request: Max 10 seconds
  - Download phase: Full `timeout_seconds` configured in task
  - Prevents slow permission requests from consuming entire timeout budget

### Scalability
- **Coordination limits**: Server queues tests (1 at a time by default)
- **Network impact**: Each test consumes test_file_size bandwidth
- **Recommended**: 3-10 agents per server for bandwidth testing
- **Schedule**: Longer intervals (300-600s) reduce server load

## Troubleshooting

### Common Issues

#### Delayed Test Execution
**Symptom**: Tests receive "delay" responses and take longer than scheduled
**Cause**: Multiple agents trying to test simultaneously - server queuing tests
**What happens**: Server returns `{"action": "delay", "delay_seconds": 90}` suggesting retry time
**Solutions**:
1. Stagger agent schedules (offset start times by 60-120s)
2. Increase schedule_seconds (longer intervals between tests)
3. Deploy multiple servers for different agent groups
**Note**: This is normal behavior, not an error - server is coordinating tests properly

#### Inconsistent Bandwidth Readings
**Symptom**: High variance in `bandwidth_mbps` (50 Mbps → 100 Mbps → 60 Mbps)
**Causes**:
- Network congestion (shared bandwidth)
- QoS policies prioritizing/deprioritizing test traffic
- Other applications consuming bandwidth during test
- ISP throttling or traffic shaping
**Solution**: Run tests during consistent times, analyze patterns

#### Tests Always Timeout
**Symptom**: All tests fail with timeout errors
**Causes**:
- timeout_seconds too short for link speed
- Network path blocking large downloads
- Server unreachable
**Solutions**:
```toml
# Increase timeout for slow links
timeout_seconds = 120

# Reduce server test file size
# In server.toml:
bandwidth_test_size_mb = 5
```

#### Lower Bandwidth Than Expected
**Symptom**: Measured speed < ISP advertised speed
**Causes**:
- ISP advertises "up to" speeds (not guaranteed)
- Shared server resources (server upload bandwidth saturated)
- Network path bottleneck (not ISP link itself)
- QoS throttling monitoring traffic
**Solution**: Compare with other speed test tools (speedtest.net), test at different times

### Debugging Tips

**Manual bandwidth test:**
```bash
# Step 1: Request permission
curl -X POST https://server:8787/api/v1/bandwidth_test \
  -H "X-API-Key: your-key" \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "test-agent", "timestamp_utc": "2024-01-01T00:00:00Z"}'
# Response: {"status":"success","action":"proceed","data_size_bytes":10485760}
# Or: {"status":"success","action":"delay","delay_seconds":90}

# Step 2: If action="proceed", download test data
time curl -o /dev/null https://server:8787/api/v1/bandwidth_download?agent_id=test-agent

# Calculate Mbps
# (file_size_MB * 8) / time_seconds = Mbps
```

**Analyze bandwidth trends:**
```sql
SELECT
  timestamp,
  bandwidth_mbps,
  duration_ms,
  bytes_downloaded
FROM raw_metric_bandwidth
WHERE task_name = 'WAN Link'
ORDER BY timestamp DESC
LIMIT 50;
```

**Check test frequency:**
```sql
SELECT
  COUNT(*) as test_count,
  AVG(bandwidth_mbps) as avg_speed,
  MIN(bandwidth_mbps) as min_speed,
  MAX(bandwidth_mbps) as max_speed
FROM raw_metric_bandwidth
WHERE task_name = 'WAN Link'
  AND timestamp > (strftime('%s', 'now') - 86400);
```

## Best Practices

1. **Set Realistic Schedules**:
   - Minimum: 60 seconds (enforced)
   - Typical: 300-600 seconds (5-10 minutes)
   - Low-bandwidth links: 900-1800 seconds (15-30 minutes)

2. **Size Test File Appropriately**:
   ```toml
   # In server.toml
   # Fast networks (>100 Mbps):
   bandwidth_test_size_mb = 25

   # Slow networks (<10 Mbps):
   bandwidth_test_size_mb = 5
   ```

3. **Avoid Peak Hours for Production**:
   - Don't saturate network during business-critical times
   - Schedule tests during off-peak hours if possible
   - Use longer intervals in production

4. **Monitor Baseline and Trends**:
   - Establish bandwidth baseline over weeks
   - Alert on deviations from baseline, not absolute values
   - Track long-term trends for capacity planning

5. **Coordinate Multiple Agents**:
   - Offset agent start times (Agent 1: 00:00, Agent 2: 00:15, Agent 3: 00:30)
   - Use different schedules (Agent 1: 300s, Agent 2: 330s)
   - Server coordination helps, but staggering still reduces contention

6. **Combine with Other Metrics**:
   ```toml
   # Ping for latency
   [[tasks]]
   type = "ping"
   name = "WAN Latency"
   schedule_seconds = 30
   host = "server-ip"

   # Bandwidth for throughput
   [[tasks]]
   type = "bandwidth"
   name = "WAN Throughput"
   schedule_seconds = 300
   ```

7. **Set Timeouts Based on Expected Speed**:
   ```toml
   # 100 Mbps link, 10 MB file: ~1s needed
   timeout_seconds = 30

   # 1 Mbps link, 10 MB file: ~80s needed
   timeout_seconds = 120
   ```

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_PING.md](TASK_PING.md) - Network latency monitoring
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP endpoint monitoring
- [TASK_DNS.md](TASK_DNS.md) - DNS resolution monitoring
