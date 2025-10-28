# TLS Handshake Task

The **TLS Handshake** task implements standalone TLS connectivity testing that stops after completing the TLS handshake, without fetching HTTP content. It's simpler and faster than full HTTP GET requests while providing detailed TLS/SSL certificate information and connection timing.

## Implementation Details

### Custom TLS Implementation (via rustls)
**Component**: `agent/src/task_tls.rs` - Shared TLS connection primitives

This task uses **rustls** (a pure-Rust TLS implementation) for TLS connections, providing memory-safe, modern TLS support with detailed timing.

**Key Characteristics**:
- **Raw TCP + TLS**: Establishes TCP connection, then performs TLS handshake
- **rustls TLS stack**: Pure Rust, memory-safe TLS 1.2/1.3 implementation
- **Stops After Handshake**: No HTTP request/response, just TLS negotiation
- **Certificate Inspection**: Direct access to X.509 certificate details via x509-parser
- **Async/Non-blocking**: Built on Tokio runtime
- **Precise Timing**: Separate measurements for TCP and TLS phases

**Consequences**:
- ✅ **Faster Than HTTP**: No request/response overhead, just connection + handshake
- ✅ **Certificate Monitoring**: Track certificate expiry, validity, issuer
- ✅ **TLS Performance**: Isolate TLS handshake latency from application performance
- ✅ **Lower Resource Usage**: No content download, minimal memory
- ✅ **Memory Safe**: Pure Rust implementation eliminates OpenSSL CVEs
- ✅ **Direct TLS Services**: Works with any service using direct TLS (HTTPS, SMTPS, LDAPS)
- ⚠️ **No Application Testing**: Doesn't verify service responds correctly
- ⚠️ **Direct TLS Only**: Requires services with TLS from connection start (not plain HTTP)
- ⚠️ **No STARTTLS Support**: Cannot test services requiring protocol upgrade (SMTP port 587, plain IMAP→STARTTLS)
- ⚠️ **No Protocol Validation**: Cannot verify HTTP headers, SMTP commands, etc.

**Why TLS-Only Testing?**
- Certificate monitoring without HTTP overhead
- Faster than HTTP GET when you only care about TLS health
- Can test non-HTTP TLS services (SMTPS on port 465, LDAPS on port 636, etc.)
- Isolates TLS performance from backend application performance
- Useful baseline before attempting protocol-specific checks

**⚠️ IMPORTANT - Direct TLS vs STARTTLS:**
This task only supports **direct TLS** connections where TLS encryption starts immediately upon connection. It does **NOT** support STARTTLS protocol upgrade (plaintext connection → upgrade to TLS).

**Works with:**
- HTTPS (port 443)
- SMTPS - SMTP over TLS (port 465)
- LDAPS - LDAP over TLS (port 636)
- IMAPS - IMAP over TLS (port 993)
- POP3S - POP3 over TLS (port 995)

**Does NOT work with:**
- SMTP with STARTTLS (port 587) - requires EHLO then STARTTLS command
- IMAP with STARTTLS (port 143) - requires CAPABILITY then STARTTLS command
- Plain services that optionally upgrade to TLS

**TLS Handshake Flow (rustls implementation)**:
```rust
1. Parse hostname:port from configuration
2. Resolve hostname to IP via tokio DNS resolver
3. Establish TCP connection (measure tcp_timing_ms)
4. Start rustls TLS handshake
   - ClientHello (TLS 1.3/1.2, cipher suites, SNI extension)
   - ServerHello (chosen TLS version & cipher)
   - Certificate chain received from server
   - Certificate verification via x509-parser:
     * Parse DER-encoded certificate
     * Extract validity period (not_before, not_after)
     * Check expiry against current system time
     * If verify_ssl=true: validate chain with system root certs
     * If verify_ssl=false: skip chain validation, still extract info
   - Key exchange (ECDHE/RSA), Finished messages
5. Record TLS handshake time (measure tls_timing_ms)
6. Extract certificate information:
   - expires_at (SystemTime)
   - is_active (not expired)
   - ssl_cert_days_until_expiry (calculated)
7. Close TLS connection immediately (no application data sent)
```

**TLS vs HTTP GET**:
- **TLS Task**: TCP + TLS handshake (~100-200ms)
- **HTTP GET**: TCP + TLS handshake + HTTP request + response (~200-500ms)
- Use TLS task when you only need certificate/connection monitoring

## Configuration

### Basic Configuration

```toml
[[tasks]]
type = "tls_handshake"
name = "API TLS Check"
schedule_seconds = 60
host = "api.example.com:443"
```

### Advanced Configuration

```toml
[[tasks]]
type = "tls_handshake"
name = "Payment Gateway TLS"
schedule_seconds = 300
host = "payments.example.com:443"
verify_ssl = true              # Fail if certificate invalid (default: false)
target_id = "payment-prod"
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"tls_handshake"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between checks (seconds) |
| `host` | string | ✅ | - | Target host:port (e.g., `"example.com:443"`) |
| `verify_ssl` | boolean | ❌ | false | Certificate verification mode (see below) |
| `timeout` | integer | ❌ | 10 | Task-level timeout (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering (e.g., "api-prod", "cdn-us") |

**Certificate Verification Behavior (`verify_ssl`):**
- `verify_ssl = true`: Uses system root certificates to validate the certificate chain. Task **fails** if certificate is invalid (expired, wrong hostname, untrusted CA, incomplete chain).
- `verify_ssl = false` (default): Accepts any certificate without validation. Task **succeeds** even with invalid certificates, but the `ssl_valid` metric field still indicates whether the certificate would be valid. Useful for monitoring certificate status on dev/internal servers without causing task failures.

**Note**: Default timeout is 10 seconds (confirmed in code at `shared/src/config.rs:860`) - higher than other tasks since TLS handshakes can be slower than simple TCP connections.

### Configuration Examples

#### Monitor SSL Certificate Expiry
```toml
[[tasks]]
type = "tls_handshake"
name = "Main Website SSL"
schedule_seconds = 3600  # Check once per hour
host = "www.example.com:443"
verify_ssl = true
```

#### Test Development Server with Self-Signed Certificate
```toml
[[tasks]]
type = "tls_handshake"
name = "Dev API - Self-Signed"
schedule_seconds = 60
host = "dev-api.internal:443"
verify_ssl = false  # Don't fail on self-signed cert
target_id = "development"
```


#### Certificate Renewal Verification
```toml
# Monitor certificate after renewal
[[tasks]]
type = "tls_handshake"
name = "Post-Renewal Check"
schedule_seconds = 300
host = "renewed-cert.example.com:443"
verify_ssl = true
```

#### Non-HTTP TLS Services
```toml
# SMTP over TLS (direct TLS on port 465, NOT STARTTLS)
[[tasks]]
type = "tls_handshake"
name = "Mail Server TLS"
schedule_seconds = 300
host = "smtp.example.com:465"  # SMTPS (direct TLS)
verify_ssl = true

# Note: Port 587 with STARTTLS is NOT supported - use port 465 (SMTPS) instead
```

#### Compare TLS Performance Across Regions
```toml
[[tasks]]
type = "tls_handshake"
name = "API - Direct to Origin"
schedule_seconds = 120
host = "origin-api.example.com:443"
verify_ssl = true

[[tasks]]
type = "tls_handshake"
name = "API - via CDN"
schedule_seconds = 120
host = "api.example.com:443"
verify_ssl = true
```

## Metrics

### Raw Metrics (`raw_metric_tls`)

Captured for each individual TLS handshake:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when handshake was attempted |
| `tcp_timing_ms` | REAL | TCP connection time (ms) - NULL if connection failed |
| `tls_timing_ms` | REAL | TLS handshake time (ms) - NULL if handshake failed |
| `ssl_valid` | BOOLEAN | Whether SSL certificate is valid (NULL if handshake failed) |
| `ssl_cert_days_until_expiry` | INTEGER | Days until certificate expires (negative if expired, NULL if failed) |
| `success` | BOOLEAN | Whether handshake succeeded (1) or failed (0) |
| `error` | TEXT | Error message if handshake failed (NULL on success) |
| `target_id` | TEXT | Optional target identifier from configuration |


### Aggregated Metrics (`agg_metric_tls`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of handshake attempts in period |
| `success_rate_percent` | REAL | Percentage of successful handshakes (0-100) |
| `avg_tcp_timing_ms` | REAL | Mean TCP connection time |
| `avg_tls_timing_ms` | REAL | Mean TLS handshake time |
| `max_tcp_timing_ms` | REAL | Maximum TCP time observed |
| `max_tls_timing_ms` | REAL | Maximum TLS time observed |
| `successful_handshakes` | INTEGER | Count of successful handshakes |
| `failed_handshakes` | INTEGER | Count of failed handshakes |
| `ssl_valid_percent` | REAL | Percentage of handshakes with valid certificates (0-100) |
| `avg_ssl_cert_days_until_expiry` | REAL | Average days until certificate expiry |
| `target_id` | TEXT | Optional target identifier from configuration |


### Metrics Interpretation

#### Connection Time Analysis
- **tcp_timing_ms**: Same as TCP task (network latency)
  - < 10ms: Local network
  - 10-100ms: Normal internet
  - > 200ms: High latency
- **tls_timing_ms**: TLS handshake overhead
  - < 50ms: Fast TLS 1.3 handshake
  - 50-150ms: Normal TLS 1.2 handshake
  - > 200ms: Slow, investigate server or cipher suite

#### Certificate Health
- **ssl_valid = true**: Certificate is valid
- **ssl_valid = false**: Certificate invalid (expired, wrong hostname, untrusted CA)
- **ssl_cert_days_until_expiry**:
  - \> 30 days: Healthy
  - 7-30 days: Plan renewal soon
  - < 7 days: Urgent renewal needed
  - < 0: Certificate expired

#### Success Rate Patterns
- **100% success, 100% ssl_valid**: Healthy TLS service
- **< 100% ssl_valid with verify_ssl=false**: Certificate issues but handshake succeeds
- **< 95% success**: TLS configuration or server issues
- **0% success**: Service down or severe TLS misconfiguration

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_tls_timing_ms > 200
  - ssl_cert_days_until_expiry < 30
  - ssl_valid_percent < 100 (with verify_ssl=true)
  - success_rate_percent < 99

CRITICAL:
  - avg_tls_timing_ms > 500
  - ssl_cert_days_until_expiry < 7
  - ssl_cert_days_until_expiry < 0 (expired!)
  - success_rate_percent < 90
  - successful_handshakes == 0 (for 5 minutes)
```

## Design Philosophy

### What It Does

Performs TLS handshakes to HTTPS/TLS services and measures:
- **TLS Connectivity**: Can we establish encrypted connection?
- **Handshake Performance**: How long does TLS negotiation take?
- **Certificate Validity**: Is the SSL certificate valid and not expired?
- **Certificate Expiry Tracking**: When does the certificate expire?

### Strong Sides

1. **Certificate Monitoring**: Track SSL certificate expiry automatically
2. **Faster Than HTTP**: No application protocol overhead
3. **TLS Performance Isolation**: Separate TLS latency from backend processing
4. **Protocol Agnostic**: Works with any TLS service (HTTPS, SMTPS, LDAPS, etc.)
5. **Early Warning**: Detect certificate issues before they cause outages
6. **Detailed Timing**: Separate TCP and TLS phase measurements

### Typical Use Cases

- **SSL Certificate Expiry Monitoring**: Get notified before certificates expire
- **TLS Performance Baseline**: Measure TLS handshake latency
- **Certificate Deployment Verification**: Confirm new certificates are active
- **Load Balancer TLS Health**: Check TLS on backend servers
- **CDN TLS Validation**: Verify edge servers have valid certificates
- **Compliance Monitoring**: Ensure all services use valid TLS
- **Certificate Renewal Tracking**: Monitor certificate rotation
- **Non-HTTP TLS Services**: Test SMTP, LDAP, database TLS connections
- **TLS Version/Cipher Validation**: Ensure modern TLS in use

### Limitations

- **HTTPS Only**: Requires TLS/SSL service (cannot test plain HTTP)
- **No Application Testing**: Doesn't verify service functionality
- **No Request/Response**: Cannot validate application behavior
- **Certificate Details Limited**: Tracks expiry/validity but not full chain details
- **Single Endpoint**: Each task tests one host:port

## Performance Characteristics

### Resource Usage
- **CPU**: Low (~1-3% per handshake)
- **Memory**: ~50KB per connection
- **Network**: TLS handshake packets (~3-5KB total)
- **Disk I/O**: Batch writes to SQLite

### Execution Time
- **TLS 1.3**: 50-100ms (1-RTT handshake, rustls default preference)
- **TLS 1.2**: 100-200ms (2-RTT handshake, fallback if server doesn't support 1.3)
- **Timeout**: 10 seconds (default, hardcoded in `shared/src/config.rs:860`)
- **TCP + TLS Total**: Typically 150-300ms

### Scalability
- Can monitor **100+ TLS endpoints** simultaneously
- Lighter than HTTP GET tasks
- Recommended schedule: 60-300 seconds per endpoint

## Troubleshooting

### Common Issues

#### "SSL certificate verification failed"
**Symptom**: Handshakes fail with SSL errors (when verify_ssl=true)
**Causes**:
- Certificate expired
- Self-signed certificate
- Hostname mismatch
- Untrusted CA
- Incomplete certificate chain
**Solutions**:
```bash
# Check certificate manually
openssl s_client -connect example.com:443 -servername example.com

# View certificate details
echo | openssl s_client -connect example.com:443 2>/dev/null | openssl x509 -noout -dates

# Test with curl
curl -v https://example.com
```
**Configuration fix**:
```toml
# Allow invalid certs but still monitor them
verify_ssl = false  # Will set ssl_valid=false but not fail task
```

#### High TLS Handshake Time
**Symptom**: `tls_timing_ms` consistently > 200ms
**Causes**:
- Server using old TLS 1.2 (slower than TLS 1.3)
- Complex certificate chain validation
- Server CPU overload
- Network latency (affects handshake RTTs)
**Solutions**:
```bash
# Check TLS version
openssl s_client -connect example.com:443 -tls1_3
openssl s_client -connect example.com:443 -tls1_2

# Measure handshake time
time openssl s_client -connect example.com:443 < /dev/null
```

#### Certificate Expiry Not Updating
**Symptom**: `ssl_cert_days_until_expiry` doesn't change after renewal
**Causes**:
- Old certificate still cached
- Certificate renewal didn't deploy
- Load balancer serving old certificate
**Solutions**:
```bash
# Force check current certificate
openssl s_client -connect example.com:443 -servername example.com < /dev/null \
  | openssl x509 -noout -dates

# Clear any TLS session cache
# (Usually not needed, each connection is fresh)
```

#### Intermittent SSL Failures
**Symptom**: Alternating success/failure
**Causes**:
- Load balancer with mixed certificate configurations
- Certificate renewal in progress
- SNI (Server Name Indication) issues
**Solutions**:
- Check all backend servers have correct certificate
- Verify load balancer TLS configuration
- Test with explicit SNI: `-servername` flag

#### "Connection timeout" Before TLS
**Symptom**: Fails at TCP level, never reaches TLS
**Causes**:
- Firewall blocking port 443
- Service down
- Wrong hostname/port
**Solution**: Use TCP task first to verify port connectivity

### Debugging Tips

**Test TLS manually (using OpenSSL CLI):**

> **Note**: The agent uses rustls (not OpenSSL), but OpenSSL CLI tools are useful for manual verification. Behavior should be similar, though rustls may be stricter about certificate validation and cipher suite selection.

```bash
# Full TLS connection test
openssl s_client -connect example.com:443 -servername example.com

# Check certificate expiry
echo | openssl s_client -connect example.com:443 2>/dev/null \
  | openssl x509 -noout -dates

# Test specific TLS versions
openssl s_client -connect example.com:443 -tls1_3
openssl s_client -connect example.com:443 -tls1_2

# Check certificate chain
openssl s_client -connect example.com:443 -showcerts
```

**Key differences between rustls (agent) and OpenSSL (CLI tools):**
- **Root certificates**: rustls loads system native certs via `rustls-native-certs`, OpenSSL uses its own cert store
- **Cipher suites**: rustls only supports modern, secure ciphers (no RC4, 3DES, export ciphers)
- **TLS versions**: rustls supports TLS 1.2 and 1.3 only (no SSL 2.0/3.0, TLS 1.0/1.1)
- **SNI**: rustls always sends SNI (Server Name Indication), which matches modern standards
- **Memory safety**: rustls is immune to OpenSSL-specific CVEs (Heartbleed, etc.)

**Analyze timing patterns:**
```sql
SELECT
  timestamp,
  tcp_timing_ms,
  tls_timing_ms,
  (tcp_timing_ms + tls_timing_ms) as total_ms,
  ssl_cert_days_until_expiry,
  success
FROM raw_metric_tls
WHERE task_name = 'API TLS'
ORDER BY timestamp DESC
LIMIT 50;
```

**Track certificate expiry:**
```sql
SELECT
  task_name,
  MIN(ssl_cert_days_until_expiry) as days_left,
  MAX(timestamp) as last_check
FROM raw_metric_tls
WHERE success = 1
GROUP BY task_name
HAVING days_left < 30
ORDER BY days_left ASC;
```

**Compare TCP vs TLS overhead:**
```sql
SELECT
  period_start,
  avg_tcp_timing_ms,
  avg_tls_timing_ms,
  (avg_tls_timing_ms / (avg_tcp_timing_ms + avg_tls_timing_ms) * 100) as tls_overhead_percent
FROM agg_metric_tls
WHERE task_name = 'API TLS'
ORDER BY period_start DESC
LIMIT 20;
```

## Best Practices

1. **Monitor Certificate Expiry Early**:
   ```toml
   schedule_seconds = 3600  # Check hourly
   verify_ssl = true         # Alert on invalid certs
   ```
   Set alerts for < 30 days until expiry

2. **Use verify_ssl Strategically**:
   - `verify_ssl = true`: Production services (enforce valid certs)
   - `verify_ssl = false`: Development/internal (monitor but don't fail)

3. **Combine with HTTP Tasks**:
   ```toml
   # TLS layer
   [[task]]
   type = "tls_handshake"
   name = "API TLS Check"
   schedule_seconds = 300
   [task.params]
   host = "api.example.com:443"

   # Application layer
   [[task]]
   type = "http_get"
   name = "API Health"
   schedule_seconds = 60
   url = "https://api.example.com/health"
   ```

4. **Test All Certificate Endpoints**:
   - Main domain (www.example.com)
   - API endpoints (api.example.com)
   - CDN edges (cdn.example.com)
   - Admin interfaces (admin.example.com)

5. **Monitor Certificate Rotation**:
   - Track `ssl_cert_days_until_expiry` over time
   - Alert if value doesn't reset after expected renewal
   - Verify all load balancer backends use same certificate

6. **Set Appropriate Schedules**:
   - Certificate monitoring: 1-24 hours (changes slowly)
   - Performance monitoring: 1-5 minutes (varies with load)
   - Don't over-check certificates (they change infrequently)

7. **Use target_id for Multi-Environment Tracking**:
   ```toml
   # Production
   [task.params]
   host = "api.prod.example.com:443"
   target_id = "production"

   # Staging
   [task.params]
   host = "api.staging.example.com:443"
   target_id = "staging"
   ```

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_TCP.md](TASK_TCP.md) - TCP connection monitoring (network layer)
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP monitoring (adds HTTP layer)
- [TASK_PING.md](TASK_PING.md) - ICMP ping monitoring
- [TASK_DNS.md](TASK_DNS.md) - DNS resolution monitoring
