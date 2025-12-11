# SNMP Query Task

> **Feature Flag Required**: SNMP task support requires the `snmp-tasks` feature flag.
>
> ```bash
> # Build with SNMP support
> cargo build --release --features snmp-tasks
> ```
>
> **Build Dependencies**:
> - Ubuntu/Debian: `apt install libssl-dev pkg-config`
> - Fedora/RHEL: `dnf install openssl-devel`
> - macOS: `brew install openssl` (usually pre-installed)
>
> **Runtime Dependencies**: `libssl` shared library must be available at runtime.

The **SNMP Query** task monitors network devices by querying single OID values using the Simple Network Management Protocol. SNMP is the standard protocol for network device monitoring - routers, switches, printers, UPS systems, and other infrastructure equipment expose operational metrics via SNMP.

## Implementation Details

### Crate: `snmp2`
**Link**: [snmp2 on crates.io](https://crates.io/crates/snmp2)

This task uses **`snmp2`**, an async Rust SNMP client implementation.

**Key Characteristics**:
- **Async/Non-blocking**: Built on Tokio, high concurrency support
- **Protocol Support**: SNMPv1, SNMPv2c, SNMPv3
- **Security Levels**: noAuthNoPriv and authNoPriv for SNMPv3
- **Authentication**: MD5, SHA-1, SHA-224, SHA-256, SHA-384, SHA-512
- **UDP Transport**: Standard SNMP over UDP port 161
- **Single OID Queries**: GET operation for individual OID values

**Consequences**:
- ✅ **Network Device Monitoring**: Query any SNMP-enabled device
- ✅ **High Concurrency**: Monitor hundreds of devices simultaneously
- ✅ **Flexible Security**: Community strings (v1/v2c) or user-based security (v3)
- ✅ **Low Overhead**: UDP-based, minimal network traffic per query
- ✅ **Pure Rust**: No C dependencies, easier cross-compilation
- ⚠️ **Single OID**: Each task queries one OID (use multiple tasks for multiple OIDs)
- ⚠️ **No SNMP Walk**: Only GET operations, no GetNext/GetBulk
- ⚠️ **No Privacy (authPriv)**: SNMPv3 encryption not supported, only authentication

**SNMP Query Flow**:

```
1. Parse host address (auto-append :161 if port not specified)
2. Parse OID string to binary OID format
3. Create SNMP session based on version:
   - v1/v2c: Use community string
   - v3: Configure security (username, auth protocol, password)
4. For v3: Perform engine discovery (init())
5. Send SNMP GET request (UDP)
6. Wait for response (with tokio::time::timeout wrapper)
7. Extract value and type from response varbind
8. Convert value to string representation
9. Measure total query time
```

**Version Differences**:

| Feature | SNMPv1 | SNMPv2c | SNMPv3 |
|---------|--------|---------|--------|
| Security | Community string | Community string | User-based |
| 64-bit counters | No | Yes | Yes |
| Authentication | None | None | MD5/SHA family |
| Encryption | None | None | Not supported |
| Engine discovery | No | No | Yes (automatic) |

## Configuration

### SNMPv2c Configuration (Recommended)

```toml
[[tasks]]
type = "snmp"
name = "Router Uptime"
schedule_seconds = 60
host = "192.168.1.1"
oid = "1.3.6.1.2.1.1.3.0"
version = "v2c"
community = "public"
```

### SNMPv3 Configuration (Secure)

```toml
[[tasks]]
type = "snmp"
name = "Switch Interface Status"
schedule_seconds = 60
host = "192.168.1.2:161"
oid = "1.3.6.1.2.1.2.2.1.8.1"
version = "v3"
username = "monitoring"
security_level = "auth_no_priv"
auth_protocol = "sha256"
auth_password = "secretpassword"
target_id = "core-switch"
```

### SNMPv1 Configuration (Legacy)

```toml
[[tasks]]
type = "snmp"
name = "Old Device Check"
schedule_seconds = 60
host = "192.168.1.100"
oid = "1.3.6.1.2.1.1.1.0"
version = "v1"
community = "public"
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"snmp"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between queries (≥60 seconds, enforced) |
| `host` | string | ✅ | - | Target host (IP or hostname). Port defaults to 161 if not specified |
| `oid` | string | ✅ | - | OID to query (e.g., `"1.3.6.1.2.1.1.1.0"`) |
| `version` | string | ❌ | `"v2c"` | SNMP version: `"v1"`, `"v2c"`, `"v3"` |
| `community` | string | ❌ | `"public"` | Community string (v1/v2c) |
| `username` | string | ❌ | - | Username (v3, required for v3) |
| `security_level` | string | ❌ | `"no_auth_no_priv"` | Security level: `"no_auth_no_priv"`, `"auth_no_priv"` |
| `auth_protocol` | string | ❌ | `"none"` | Auth protocol: `"none"`, `"md5"`, `"sha1"`, `"sha224"`, `"sha256"`, `"sha384"`, `"sha512"` |
| `auth_password` | string | ❌ | - | Authentication password (required for `auth_no_priv`) |
| `timeout_seconds` | integer | ❌ | 5 | Query timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets |

### Host Address Formats

```toml
# IPv4 without port (uses default 161)
host = "192.168.1.1"

# IPv4 with custom port
host = "192.168.1.1:10161"

# IPv6 without port
host = "2001:db8::1"

# IPv6 with port (requires brackets)
host = "[2001:db8::1]:161"

# Hostname (DNS resolution)
host = "router.example.com"
```

### OID Format

OIDs can be specified with or without leading dot:

```toml
# Both formats are valid
oid = "1.3.6.1.2.1.1.1.0"
oid = ".1.3.6.1.2.1.1.1.0"
```

### Common OIDs (MIB-II)

| OID | Name | Type | Description |
|-----|------|------|-------------|
| `1.3.6.1.2.1.1.1.0` | sysDescr | OctetString | System description |
| `1.3.6.1.2.1.1.3.0` | sysUpTime | Timeticks | Uptime in hundredths of seconds |
| `1.3.6.1.2.1.1.4.0` | sysContact | OctetString | Contact person |
| `1.3.6.1.2.1.1.5.0` | sysName | OctetString | System name (hostname) |
| `1.3.6.1.2.1.1.6.0` | sysLocation | OctetString | Physical location |
| `1.3.6.1.2.1.2.1.0` | ifNumber | Integer | Number of interfaces |
| `1.3.6.1.2.1.2.2.1.8.X` | ifOperStatus | Integer | Interface operational status (X=interface index) |
| `1.3.6.1.2.1.2.2.1.10.X` | ifInOctets | Counter32 | Input bytes (X=interface index) |
| `1.3.6.1.2.1.2.2.1.16.X` | ifOutOctets | Counter32 | Output bytes (X=interface index) |

### Configuration Examples

#### Monitor Router System Info
```toml
[[tasks]]
type = "snmp"
name = "Core Router - Description"
schedule_seconds = 300
host = "10.0.0.1"
oid = "1.3.6.1.2.1.1.1.0"
version = "v2c"
community = "monitoring"
target_id = "core-router"

[[tasks]]
type = "snmp"
name = "Core Router - Uptime"
schedule_seconds = 60
host = "10.0.0.1"
oid = "1.3.6.1.2.1.1.3.0"
version = "v2c"
community = "monitoring"
target_id = "core-router"
```

#### Monitor Interface Status
```toml
# Interface 1 operational status (1=up, 2=down)
[[tasks]]
type = "snmp"
name = "Switch Port 1 Status"
schedule_seconds = 60
host = "192.168.1.10"
oid = "1.3.6.1.2.1.2.2.1.8.1"
version = "v2c"
community = "public"
target_id = "access-switch"

# Interface 1 input traffic
[[tasks]]
type = "snmp"
name = "Switch Port 1 In Octets"
schedule_seconds = 60
host = "192.168.1.10"
oid = "1.3.6.1.2.1.2.2.1.10.1"
version = "v2c"
community = "public"
target_id = "access-switch"
```

#### Secure SNMPv3 Monitoring
```toml
[[tasks]]
type = "snmp"
name = "Firewall CPU Usage"
schedule_seconds = 60
host = "10.0.0.254"
oid = "1.3.6.1.4.1.2021.11.9.0"
version = "v3"
username = "monitor_user"
security_level = "auth_no_priv"
auth_protocol = "sha256"
auth_password = "MySecurePassword123"
target_id = "firewall"
```

#### Multiple Devices with Same OID
```toml
[[tasks]]
type = "snmp"
name = "Router A - Uptime"
schedule_seconds = 60
host = "10.0.1.1"
oid = "1.3.6.1.2.1.1.3.0"
version = "v2c"
community = "public"
target_id = "router-a"

[[tasks]]
type = "snmp"
name = "Router B - Uptime"
schedule_seconds = 60
host = "10.0.2.1"
oid = "1.3.6.1.2.1.1.3.0"
version = "v2c"
community = "public"
target_id = "router-b"

[[tasks]]
type = "snmp"
name = "Router C - Uptime"
schedule_seconds = 60
host = "10.0.3.1"
oid = "1.3.6.1.2.1.1.3.0"
version = "v2c"
community = "public"
target_id = "router-c"
```

#### UPS Monitoring
```toml
# Battery status (1=unknown, 2=normal, 3=low, 4=depleted)
[[tasks]]
type = "snmp"
name = "UPS Battery Status"
schedule_seconds = 60
host = "192.168.1.50"
oid = "1.3.6.1.2.1.33.1.2.1.0"
version = "v2c"
community = "public"
target_id = "ups-main"

# Battery charge remaining (percentage)
[[tasks]]
type = "snmp"
name = "UPS Battery Charge"
schedule_seconds = 60
host = "192.168.1.50"
oid = "1.3.6.1.2.1.33.1.2.4.0"
version = "v2c"
community = "public"
target_id = "ups-main"
```

## Metrics

### Raw Metrics (`raw_metric_snmp`)

Captured for each individual SNMP query:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when query was executed |
| `response_time_ms` | REAL | Query response time (ms) - NULL if failed |
| `success` | BOOLEAN | Whether query succeeded (1) or failed (0) |
| `value` | TEXT | Retrieved value as string - NULL if failed |
| `value_type` | TEXT | SNMP data type name (e.g., "Integer", "OctetString") |
| `oid_queried` | TEXT | OID that was queried |
| `error` | TEXT | Error message if query failed (NULL on success) |
| `target_id` | TEXT | Optional target identifier from task configuration |

### Aggregated Metrics (`agg_metric_snmp`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `success_rate_percent` | REAL | Percentage of successful queries (0-100) |
| `avg_response_time_ms` | REAL | Mean query response time |
| `successful_queries` | INTEGER | Count of successful queries |
| `failed_queries` | INTEGER | Count of failed queries |
| `first_value` | TEXT | First value captured in period |
| `first_value_type` | TEXT | Type of first_value |
| `oid_queried` | TEXT | OID that was queried |
| `target_id` | TEXT | Optional target identifier |

**Note**: Since SNMP tasks have a minimum 60-second interval, aggregations typically contain 1 sample per period.

### SNMP Value Types

The `value_type` field contains the SNMP/ASN.1 type name:

| Type | Description | Example Value |
|------|-------------|---------------|
| `Integer` | Signed 64-bit integer | `42` |
| `OctetString` | Byte string (UTF-8 or hex) | `"Cisco Router"` or `"00:1a:2b:3c:4d:5e"` |
| `ObjectIdentifier` | OID reference | `1.3.6.1.2.1.1.1` |
| `IpAddress` | IPv4 address | `192.168.1.1` |
| `Counter32` | 32-bit counter (wraps at 2^32) | `1234567890` |
| `Counter64` | 64-bit counter (SNMPv2c/v3) | `9876543210123456` |
| `Unsigned32` | Unsigned 32-bit integer (Gauge32) | `85` |
| `Timeticks` | Time in hundredths of seconds | `0d 5h 30m 45s (1984500 ticks)` |
| `Null` | No value | `null` |
| `NoSuchObject` | OID doesn't exist on device | `noSuchObject` |
| `NoSuchInstance` | OID exists but instance doesn't | `noSuchInstance` |
| `EndOfMibView` | End of MIB tree reached | `endOfMibView` |

### Value Conversion

- **OctetString**: Displayed as UTF-8 if printable, otherwise as hex (e.g., `00:1a:2b:3c`)
- **Timeticks**: Converted to human-readable format: `Xd Xh Xm Xs (ticks)`
- **IpAddress**: Formatted as dotted-decimal: `X.X.X.X`
- **Counters/Integers**: Displayed as numeric strings

### Metrics Interpretation

#### Response Time Analysis
- **< 5ms**: Excellent (local network, healthy device)
- **5-20ms**: Good (typical LAN performance)
- **20-100ms**: Acceptable (WAN or busy device)
- **> 100ms**: Slow (network congestion or overloaded device)
- **Timeout (5s default)**: Device unreachable or SNMP disabled

#### Success Rate Patterns
- **100%**: Healthy device, stable network
- **95-99%**: Occasional UDP packet loss (normal for WAN)
- **< 90%**: Network issues or device problems
- **0%**: Device down, wrong community, or firewall blocking

#### Value Interpretation
- **NoSuchObject**: OID doesn't exist (wrong OID or unsupported MIB)
- **NoSuchInstance**: Index doesn't exist (e.g., interface removed)
- **Null**: Device returned empty value

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_response_time_ms > 50
  - success_rate_percent < 95
  - value = "noSuchObject" or "noSuchInstance"

CRITICAL:
  - avg_response_time_ms > 200
  - success_rate_percent < 80
  - successful_queries == 0 (for 5 minutes)
```

## Design Philosophy

### What It Does

Performs SNMP GET queries against network devices and measures:
- **Device Reachability**: Can we query the device via SNMP?
- **Query Latency**: How long does the SNMP query take?
- **Value Monitoring**: What value does the OID return?
- **Device Health**: Are devices responding consistently?

### Strong Sides

1. **Universal Protocol**: Works with any SNMP-enabled device (routers, switches, servers, UPS, printers)
2. **Low Overhead**: UDP-based, minimal network and device impact
3. **Flexible Security**: Community strings for simple setups, SNMPv3 for secure environments
4. **Rich Type Support**: Handles all standard SNMP data types
5. **High Concurrency**: Monitor hundreds of devices simultaneously
6. **Human-Readable Output**: Timeticks converted to days/hours/minutes

### Typical Use Cases

- **Network Device Monitoring**: Router/switch uptime, interface status
- **Interface Traffic**: Bytes in/out counters for bandwidth tracking
- **System Health**: CPU, memory, disk usage (vendor-specific OIDs)
- **Environmental**: Temperature sensors, power status
- **UPS Monitoring**: Battery status, load percentage
- **Printer Status**: Page counts, toner levels

### Limitations

- **Single OID per Task**: No SNMP walk/bulk operations
- **No Privacy (authPriv)**: SNMPv3 encryption not supported
- **No Traps**: Only polling, no trap receiver
- **No SET Operations**: Read-only monitoring
- **Minimum 60s Schedule**: Prevents device overload
- **UDP Only**: No TCP transport option

## Performance Characteristics

### Resource Usage
- **CPU**: Negligible (~0.1% per query)
- **Memory**: ~5-10KB per query
- **Network**: 50-200 bytes per query (UDP)
- **Disk I/O**: Batch writes to SQLite

### Execution Time
- **Local network**: 1-10ms
- **WAN**: 20-100ms
- **SNMPv3 (with engine discovery)**: +5-20ms overhead
- **Timeout**: Configurable (default 5s)

### Scalability
- Can monitor **200+ devices** simultaneously
- SNMPv2c more efficient than v3 (no engine discovery)
- Recommended schedule: 60-300 seconds per OID

## Troubleshooting

### Common Issues

#### "Query timed out" Errors
**Symptom**: SNMP queries consistently timeout
**Causes**:
- Device unreachable (network issue)
- SNMP not enabled on device
- Wrong community string/credentials
- Firewall blocking UDP 161
**Solutions**:
```bash
# Test SNMP manually
snmpget -v2c -c public 192.168.1.1 1.3.6.1.2.1.1.1.0

# Check connectivity
ping 192.168.1.1

# Check firewall (allow UDP 161)
sudo iptables -A OUTPUT -p udp --dport 161 -j ACCEPT
```

#### "noSuchObject" or "noSuchInstance" Returned
**Symptom**: Query succeeds but value is `noSuchObject`
**Causes**:
- Wrong OID for this device
- MIB not implemented by device
- Interface/index doesn't exist
**Solutions**:
```bash
# Walk the OID tree to find valid OIDs
snmpwalk -v2c -c public 192.168.1.1 1.3.6.1.2.1.1

# Check device documentation for supported OIDs
```

#### SNMPv3 Authentication Failures
**Symptom**: "SNMPv3 initialization failed" errors
**Causes**:
- Wrong username
- Wrong auth protocol
- Wrong password
- User not configured on device
**Solutions**:
```bash
# Test SNMPv3 manually
snmpget -v3 -u username -l authNoPriv -a SHA -A password 192.168.1.1 1.3.6.1.2.1.1.1.0

# Verify user on device (Cisco example)
show snmp user
```

#### High Response Times
**Symptom**: `response_time_ms` consistently > 100ms
**Causes**:
- Network latency
- Device under heavy load
- SNMP daemon overloaded
**Solutions**:
- Check network path (traceroute)
- Increase polling interval
- Check device CPU/memory

### Debugging Tips

**Test SNMP manually:**
```bash
# SNMPv1
snmpget -v1 -c public 192.168.1.1 1.3.6.1.2.1.1.1.0

# SNMPv2c
snmpget -v2c -c public 192.168.1.1 1.3.6.1.2.1.1.1.0

# SNMPv3 with SHA auth
snmpget -v3 -u user -l authNoPriv -a SHA -A password 192.168.1.1 1.3.6.1.2.1.1.1.0

# Walk entire system subtree
snmpwalk -v2c -c public 192.168.1.1 1.3.6.1.2.1.1
```

**Analyze query patterns:**
```sql
SELECT
  task_name,
  success,
  response_time_ms,
  value,
  error
FROM raw_metric_snmp
WHERE task_name = 'Router Uptime'
ORDER BY timestamp DESC
LIMIT 20;
```

**Check success rate over time:**
```sql
SELECT
  date(period_start, 'unixepoch') as date,
  AVG(success_rate_percent) as avg_success,
  AVG(avg_response_time_ms) as avg_response
FROM agg_metric_snmp
WHERE task_name = 'Router Uptime'
GROUP BY date
ORDER BY date DESC;
```

## Best Practices

1. **Use SNMPv2c for Compatibility**:
   - Most devices support v2c
   - Simple configuration
   - Use v3 only when security is required

2. **Use SNMPv3 for Security-Sensitive Environments**:
   - Provides authentication (no eavesdropping of credentials)
   - Use SHA-256 or higher for auth protocol
   - Avoid MD5 (cryptographically weak)

3. **Group Related OIDs with target_id**:
   ```toml
   target_id = "core-router"
   ```
   - Enables filtering/grouping in dashboards
   - Simplifies multi-device analysis

4. **Set Appropriate Timeouts**:
   ```toml
   timeout_seconds = 5   # Local devices
   timeout_seconds = 10  # WAN devices
   ```

5. **Use Descriptive Task Names**:
   ```toml
   name = "Core Router - WAN Interface Traffic In"
   ```
   - Include device name and metric description
   - Makes logs and dashboards readable

6. **Monitor Counter Differences, Not Raw Values**:
   - Counter32/Counter64 wrap around
   - Calculate rate: `(current - previous) / interval`
   - Handle counter resets gracefully

7. **Secure Community Strings**:
   - Never use "public" in production
   - Use read-only community strings
   - Restrict config file permissions

## Testing Environment

A self-contained SNMP test environment is available in `snmp_test/`:

```bash
# Start test environment (Docker + Agent + Server)
./snmp_test/start.sh

# Check collected metrics
./snmp_test/check_db.sh

# Manual SNMP testing
./snmp_test/test_snmp.sh

# Stop environment
./snmp_test/stop.sh
```

See `snmp_test/README.md` for details.

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_PING.md](TASK_PING.md) - Network connectivity monitoring
- [TASK_DNS.md](TASK_DNS.md) - DNS resolution monitoring
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP endpoint monitoring
