# DNS Query Tasks

The **DNS Query** tasks (standard DNS and DNS-over-HTTPS) monitor DNS resolution performance and correctness. DNS is critical infrastructure - if DNS fails, everything fails - yet it's often overlooked in monitoring strategies.

## Implementation Details

### Crate: `hickory-dns` (formerly `trust-dns`)
**Link**: [hickory-dns on crates.io](https://crates.io/crates/hickory-dns)

This task uses **`hickory-dns`**, a pure Rust DNS client implementation.

**Key Characteristics**:
- **Direct DNS Queries**: Makes DNS queries directly to specified servers (UDP/TCP/DoH)
- **No System Resolver**: Bypasses OS DNS cache and `/etc/hosts` completely
- **Async/Non-blocking**: Built on Tokio, high concurrency support
- **Protocol Support**: UDP (default), TCP (fallback), DNS-over-HTTPS (DoH), DNS-over-TLS (DoT)
- **Full DNS Protocol**: Supports all record types (A, AAAA, MX, CNAME, TXT, NS, etc.)
- **IPv4 and IPv6**: Works with both IP versions

**Consequences**:
- ✅ **No System Cache**: Always tests actual DNS resolution, no stale cached results
- ✅ **Direct Server Testing**: Queries specific DNS server, not system default
- ✅ **Accurate Timing**: Measures actual query time without cache interference
- ✅ **DoH Privacy**: DNS-over-HTTPS encryption prevents ISP snooping
- ✅ **High Performance**: Async implementation allows 200+ concurrent queries
- ⚠️ **No `/etc/hosts`**: Won't resolve entries from hosts file (tests DNS only)
- ⚠️ **No Search Domains**: Doesn't use system-configured DNS search suffixes
- ⚠️ **Bypasses System Config**: Ignores `/etc/resolv.conf` (queries go to configured server only)

**Why `hickory-dns` Instead of System Resolver?**
- System resolver (`libc::getaddrinfo`) uses cached results, making timing unreliable
- System calls are blocking, requiring thread pools (Tokio threadpool would block)
- Cannot choose specific DNS server with system resolver
- Cannot measure pure DNS performance (includes cache, hosts file, etc.)

**DNS Query Flow**:

**Standard DNS (`dns_query`)**:
```rust
1. Parse server address (auto-append :53 if port not specified)
2. Build DNS query packet for domain + record type
3. Send UDP packet to specified DNS server (port 53)
4. Wait for response (with tokio::time::timeout wrapper)
5. Parse response, extract IP addresses/records
6. Clean up background task (bg_handle.abort())
7. Measure total query time
```

**DNS-over-HTTPS (`dns_query_doh`)**:
```rust
1. Build DNS query in DoH wire format (RFC 8484)
2. Encode query to binary (BinEncodable)
3. Create reqwest client with timeout configured
4. Send HTTPS POST to DoH server with DNS wire-format body
   - Content-Type: application/dns-message
   - Accept: application/dns-message
5. Wait for HTTPS response (timeout enforced by reqwest client)
6. Parse DNS response from HTTPS body
7. Measure total query time (includes HTTPS/TLS overhead)
```

**UDP vs DoH Performance**:
- Standard DNS (UDP): 10-50ms typical
- DoH (HTTPS): 50-200ms (adds TLS handshake, HTTP overhead)
- DoH provides privacy but adds ~50-150ms latency

## Configuration

### Standard DNS Configuration

```toml
[[tasks]]
type = "dns_query"
name = "Google DNS - example.com"
schedule_seconds = 60
server = "8.8.8.8"
domain = "www.example.com"
record_type = "A"
```

### DNS-over-HTTPS Configuration

```toml
[[tasks]]
type = "dns_query_doh"
name = "Cloudflare DoH - example.com"
schedule_seconds = 60
server_url = "https://cloudflare-dns.com/dns-query"
domain = "www.example.com"
record_type = "A"
```

### Configuration Parameters

#### Standard DNS (`dns_query`)

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"dns_query"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between queries (seconds) |
| `server` | string | ✅ | - | DNS server IP address (IPv4 or IPv6). Port defaults to 53 if not specified (e.g., `8.8.8.8` → `8.8.8.8:53`) |
| `domain` | string | ✅ | - | Domain name to resolve |
| `record_type` | string | ✅ | - | DNS record type: `A`, `AAAA`, `MX`, `CNAME`, `TXT`, `NS` |
| `timeout_seconds` | integer | ❌ | 5 | Query timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `expected_ip` | string | ❌ | - | Expected IP address for validation (detects DNS hijacking/changes) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "dns-internal", "dns-public") |

#### DNS-over-HTTPS (`dns_query_doh`)

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"dns_query_doh"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between queries (seconds) |
| `server_url` | string | ✅ | - | DoH server URL (must be HTTPS) |
| `domain` | string | ✅ | - | Domain name to resolve |
| `record_type` | string | ✅ | - | DNS record type: `A`, `AAAA`, `MX`, `CNAME`, `TXT`, `NS` |
| `timeout_seconds` | integer | ❌ | 5 | Query timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `expected_ip` | string | ❌ | - | Expected IP address for validation (detects DNS hijacking/changes) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "dns-internal", "dns-public") |

### DNS Record Types

| Type | Description | Example Use Case |
|------|-------------|------------------|
| `A` | IPv4 address | Resolve `www.example.com` to `93.184.216.34` |
| `AAAA` | IPv6 address | Resolve `www.example.com` to `2606:2800:220:1:...` |
| `MX` | Mail exchanger | Resolve `example.com` to mail server `mail.example.com` |
| `CNAME` | Canonical name | Resolve alias `shop.example.com` to `cdn.provider.com` |
| `TXT` | Text records | Verify SPF, DKIM, domain verification records |
| `NS` | Name servers | Find authoritative name servers for `example.com` |

### Popular DoH Providers

```toml
# Cloudflare (1.1.1.1)
server_url = "https://cloudflare-dns.com/dns-query"

# Google Public DNS (8.8.8.8)
server_url = "https://dns.google/dns-query"

# Quad9 (9.9.9.9)
server_url = "https://dns.quad9.net/dns-query"

# AdGuard DNS (ad-blocking)
server_url = "https://dns.adguard.com/dns-query"
```

### Configuration Examples

#### Monitor Corporate DNS Server
```toml
[[tasks]]
type = "dns_query"
name = "Internal DNS - Employee Portal"
schedule_seconds = 30
server = "10.0.1.10"              # Internal DNS server
domain = "portal.company.internal"
record_type = "A"
timeout_seconds = 3
```

#### Compare Public DNS Resolvers
```toml
[[tasks]]
type = "dns_query"
name = "Cloudflare DNS"
schedule_seconds = 60
server = "1.1.1.1"
domain = "www.example.com"
record_type = "A"

[[tasks]]
type = "dns_query"
name = "Google DNS"
schedule_seconds = 60
server = "8.8.8.8"
domain = "www.example.com"
record_type = "A"

[[tasks]]
type = "dns_query"
name = "Quad9 DNS"
schedule_seconds = 60
server = "9.9.9.9"
domain = "www.example.com"
record_type = "A"
```

#### Verify DNS Propagation After Change
```toml
# Check authoritative NS
[[tasks]]
type = "dns_query"
name = "Auth NS - example.com"
schedule_seconds = 300
server = "ns1.example.com"        # Your authoritative server
domain = "www.example.com"
record_type = "A"

# Check public resolver
[[tasks]]
type = "dns_query"
name = "Public DNS - example.com"
schedule_seconds = 300
server = "8.8.8.8"
domain = "www.example.com"
record_type = "A"
```

#### Monitor Mail Server Resolution
```toml
[[tasks]]
type = "dns_query"
name = "MX Record Check"
schedule_seconds = 300
server = "8.8.8.8"
domain = "example.com"            # Note: MX queries use base domain
record_type = "MX"
```

#### Privacy-Focused DNS with DoH
```toml
[[tasks]]
type = "dns_query_doh"
name = "DoH Privacy Check"
schedule_seconds = 60
server_url = "https://cloudflare-dns.com/dns-query"
domain = "www.example.com"
record_type = "A"
```

#### IPv6 Resolution Monitoring
```toml
[[tasks]]
type = "dns_query"
name = "IPv6 Resolution"
schedule_seconds = 120
server = "2001:4860:4860::8888"   # Google DNS IPv6
domain = "www.example.com"
record_type = "AAAA"
```

#### DNS Resolution Validation with Expected IP
```toml
# Monitor critical service and alert if IP changes unexpectedly
[[tasks]]
type = "dns_query"
name = "API Gateway - Expected IP Check"
schedule_seconds = 60
server = "8.8.8.8"
domain = "api.example.com"
record_type = "A"
expected_ip = "203.0.113.42"      # Alert if resolution doesn't match

# Detect DNS hijacking attempts
[[tasks]]
type = "dns_query"
name = "Security - DNS Hijack Detection"
schedule_seconds = 30
server = "8.8.8.8"
domain = "login.example.com"
record_type = "A"
expected_ip = "198.51.100.10"     # Critical security endpoint
```

## Metrics

### Raw Metrics (`raw_metric_dns`)

Captured for each individual DNS query:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when query was executed |
| `query_time_ms` | REAL | DNS query resolution time (ms) - NULL if failed |
| `success` | BOOLEAN | Whether query succeeded (1) or failed (0) |
| `record_count` | INTEGER | Number of records returned - NULL if failed |
| `resolved_addresses` | TEXT | JSON array of resolved addresses: `["1.1.1.1", "1.0.0.1"]` |
| `domain_queried` | TEXT | Domain name that was queried |
| `error` | TEXT | Error message if query failed (NULL on success) |
| `expected_ip` | TEXT | Expected IP address if configured (NULL if not set) |
| `resolved_ip` | TEXT | First resolved IP address (NULL if resolution failed) |
| `correct_resolution` | BOOLEAN | True if resolved_ip matches expected_ip (or if no expected_ip set) |
| `target_id` | TEXT | Optional target identifier from task configuration |


### Aggregated Metrics (`agg_metric_dns`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of queries in period |
| `success_rate_percent` | REAL | Percentage of successful queries (0-100) |
| `avg_query_time_ms` | REAL | Mean query resolution time |
| `max_query_time_ms` | REAL | Maximum query time observed |
| `successful_queries` | INTEGER | Count of successful queries |
| `failed_queries` | INTEGER | Count of failed queries |
| `all_resolved_addresses` | TEXT | JSON set of unique IPs seen: `["1.1.1.1","1.0.0.1"]` |
| `domain_queried` | TEXT | Domain name queried |
| `correct_resolution_percent` | REAL | Percentage matching expected_ip (0-100, or 100 if no expected_ip) |
| `target_id` | TEXT | Optional target identifier from task configuration |


### Metrics Interpretation

#### Query Time Analysis
- **< 10ms**: Cached result (local or nearby resolver)
- **10-50ms**: Normal uncached query
- **50-200ms**: Slow DNS server or distant resolver
- **> 200ms**: DNS server under load or network issues

#### Resolution Consistency
- **Stable `all_resolved_addresses`**: Normal operation
- **Changing IPs**: DNS load balancing (expected) or DNS hijacking (investigate)
- **Empty results**: DNS server misconfiguration or domain doesn't exist

#### Success Rate Patterns
- **100%**: Healthy DNS service
- **95-99%**: Occasional packet loss (UDP DNS)
- **< 90%**: DNS server issues or network problems
- **0%**: DNS server unreachable or domain doesn't exist

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_query_time_ms > 100
  - success_rate_percent < 95
  - resolved_addresses changed unexpectedly

CRITICAL:
  - avg_query_time_ms > 500
  - success_rate_percent < 80
  - successful_queries == 0 (for 5 minutes)
```

## Design Philosophy

### What It Does

Performs DNS queries against specified DNS servers and measures:
- **Resolution Success**: Can the domain be resolved?
- **Query Latency**: How long does DNS resolution take?
- **Record Accuracy**: Are the returned IP addresses correct?
- **DNS Server Health**: Is the configured DNS server responding?

### Task Types

**`dns_query`**: Standard DNS over UDP/TCP (port 53)
- Uses traditional DNS protocol
- Queries specified DNS server directly
- Fastest for local/internal DNS servers

**`dns_query_doh`**: DNS-over-HTTPS (DoH)
- Encrypted DNS queries via HTTPS
- Privacy-preserving (ISP can't see queries)
- Uses public DoH providers (Cloudflare, Google, Quad9)

### Strong Sides

1. **Infrastructure Validation**: DNS is foundational - catches issues before they impact users
2. **Resolution Tracking**: Monitor which IPs domains resolve to (detect unauthorized changes)
3. **Performance Baseline**: Establish normal DNS latency patterns
4. **Multi-Server Comparison**: Compare authoritative vs. caching resolvers
5. **Privacy Options**: DoH provides encrypted queries
6. **Propagation Verification**: Confirm DNS changes have propagated globally

### Typical Use Cases

- **DNS Server Health**: Monitor corporate DNS servers (Active Directory, BIND)
- **External DNS Validation**: Ensure public resolvers (8.8.8.8) work correctly
- **TTL/Propagation Checks**: Verify DNS changes reach all resolvers
- **Geo-DNS Verification**: Confirm correct IPs returned from different locations
- **Security Monitoring**: Detect DNS hijacking or unauthorized changes
- **CDN Validation**: Ensure CDN domains resolve to edge servers
- **Failover Testing**: Verify backup DNS servers respond
- **Privacy Compliance**: Use DoH where DNS privacy is required

### Limitations

- **Standard DNS Caching**: Results may be cached by intermediate resolvers
- **No DNSSEC Validation**: Doesn't verify cryptographic signatures (planned feature)
- **UDP Packet Loss**: Standard DNS over UDP may be affected by packet loss
- **DoH Overhead**: HTTPS adds latency compared to standard DNS
- **Single Record Type**: Each task queries one record type (A, AAAA, MX, etc.)

## Performance Characteristics

### Resource Usage
- **CPU**: Negligible (~0.1% per query)
- **Memory**: ~10KB per query (standard DNS), ~100KB per query (DoH)
- **Network**:
  - Standard DNS: 50-150 bytes per query
  - DoH: 500-1000 bytes per query (HTTPS overhead)
- **Disk I/O**: Batch writes to SQLite

### Execution Time
- **Cached DNS**: 1-10ms
- **Uncached DNS**: 20-100ms
- **DoH**: 50-200ms (HTTPS overhead)
- **Timeout**: Configurable (default 5s, defined in `shared/src/defaults.rs:20`)
  - Standard DNS: Enforced via `tokio::time::timeout()` wrapper
  - DoH: Enforced via reqwest client `.timeout()` method

### Scalability
- Can monitor **200+ DNS queries** simultaneously
- Standard DNS more efficient than DoH
- Recommended schedule: 30-120 seconds per domain

## Troubleshooting

### Common Issues

#### "DNS server timeout" Errors
**Symptom**: Queries consistently timeout
**Causes**:
- DNS server down or unreachable
- Firewall blocking UDP port 53
- Network routing issues
**Solutions**:
```bash
# Test DNS manually
dig @8.8.8.8 www.example.com
nslookup www.example.com 8.8.8.8

# Check firewall (allow UDP 53 outbound)
sudo iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
```

#### DoH Queries Fail but Standard DNS Works
**Symptom**: `dns_query_doh` fails, `dns_query` succeeds
**Causes**:
- HTTPS outbound blocked
- DoH server URL incorrect
- TLS/SSL certificate issues
**Solution**: Verify HTTPS connectivity to DoH server
```bash
curl -I https://cloudflare-dns.com/dns-query
```

#### Unexpected IP Addresses Returned
**Symptom**: `resolved_addresses` contains wrong IPs
**Causes**:
- DNS hijacking
- ISP transparent DNS proxy
- Geo-DNS returning different IPs per location
- Recent DNS change not propagated
**Solution**:
- Compare results from multiple resolvers
- Check authoritative NS directly
- Verify DNS records with domain registrar

#### High Query Latency
**Symptom**: `query_time_ms` consistently > 100ms
**Causes**:
- DNS server geographically distant
- DNS server under load
- Network congestion
- Recursive query taking long
**Solution**: Use closer/faster DNS servers, consider local caching resolver

### Debugging Tips

**Test DNS resolution manually:**
```bash
# Standard DNS query
dig @8.8.8.8 www.example.com A

# DoH query (using curl)
curl -H 'accept: application/dns-json' \
  'https://cloudflare-dns.com/dns-query?name=www.example.com&type=A'

# Measure query time
time dig @8.8.8.8 www.example.com
```

**Analyze resolution patterns:**
```sql
SELECT
  timestamp,
  query_time_ms,
  resolved_addresses,
  success
FROM raw_metric_dns
WHERE task_name = 'Google DNS'
ORDER BY timestamp DESC
LIMIT 50;
```

**Check for IP changes:**
```sql
SELECT DISTINCT
  date(period_start, 'unixepoch') as date,
  all_resolved_addresses
FROM agg_metric_dns
WHERE task_name = 'Google DNS'
ORDER BY date DESC;
```

## Best Practices

1. **Monitor Multiple Resolvers**:
   - Internal DNS + external fallback
   - Compare responses for consistency
   - Detect DNS-level failures

2. **Use Authoritative Servers for Critical Checks**:
   - Query your own NS for fastest/most accurate results
   - Bypass caching for propagation verification

3. **Set Appropriate Record Types**:
   - `A`/`AAAA` for web services
   - `MX` for email infrastructure
   - `CNAME` for CDN validation
   - `TXT` for domain verification

4. **Monitor Resolution Consistency**:
   - Track `all_resolved_addresses` over time
   - Alert on unexpected changes
   - Helps detect DNS hijacking or unauthorized modifications

5. **Choose DNS vs. DoH Based on Needs**:
   - **Standard DNS**: Faster, lower overhead, corporate environments
   - **DoH**: Privacy-preserving, encrypted, bypass restrictive networks

6. **Consider Caching**:
   - Longer schedules (60-300s) for static records
   - Shorter schedules (10-30s) for dynamic/load-balanced records

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_PING.md](TASK_PING.md) - Network connectivity monitoring
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP endpoint monitoring
- [TASK_BANDWIDTH.md](TASK_BANDWIDTH.md) - Bandwidth measurement
