# HTTP GET Task

The **HTTP GET** task implements comprehensive HTTP/HTTPS endpoint monitoring with detailed timing breakdowns. It measures not just availability, but the performance of each phase of the HTTP request lifecycle.

## Implementation Details

### Custom Low-Level HTTP Client (based on `http-timings`)
**Inspiration**: [http-timings crate](https://github.com/metrixweb/http-timings) by metrixweb

This task uses a **custom low-level HTTP implementation** built directly on raw TCP sockets and OpenSSL for maximum timing precision.

**Key Characteristics**:
- **Raw TCP Sockets**: Uses `tokio::net::TcpStream` directly for precise timing control
- **OpenSSL for TLS**: Direct TLS handshake via `openssl` crate for detailed TLS timing
- **Minimal HTTP GET**: Sends request, reads headers, discards body immediately
- **Async/Non-blocking**: Built on Tokio, zero thread overhead
- **Precise Timing Measurements**: Direct access to each connection phase (DNS, TCP, TLS, TTFB)
- **No High-Level Abstractions**: No automatic retries, redirects, or content processing

**Consequences**:
- ✅ **Extremely Fast**: No body reading/parsing overhead, focuses on connection metrics
- ✅ **Low Resource Usage**: Minimal memory (~10-50KB per request)
- ✅ **Detailed Timing**: Precise measurements at DNS, TCP, TLS, TTFB phases
- ✅ **High Concurrency**: Can monitor 50+ endpoints simultaneously on modest hardware
- ✅ **SSL Certificate Information**: Direct access to certificate details (issuer, expiry, validity)
- ⚠️ **No Content Reading**: Cannot validate response body (use HTTP Content task for that)
- ⚠️ **No Decompression**: Content-Encoding headers ignored, raw transfer only
- ⚠️ **No Automatic Retries**: Single request attempt (application handles retries at task level)
- ⚠️ **HTTP/1.1 Only**: No HTTP/2 or HTTP/3 support

**Why Custom Implementation Instead of `reqwest` or `hyper`?**
- High-level clients like `reqwest` add conveniences (decompression, redirects, body parsing) that obscure timing details
- Even low-level clients like `hyper` abstract away some connection phases
- For network monitoring, we need **precise timing breakdowns** at each protocol layer
- Raw socket control provides exact measurements of TCP and TLS phases
- `reqwest` is used in HTTP Content task where body processing is needed

**Timing Breakdown Provided**:
```rust
tcp_timing_ms          // TCP handshake (SYN, SYN-ACK, ACK)
tls_timing_ms          // TLS negotiation (only for HTTPS)
ttfb_timing_ms         // Server processing time (Time To First Byte)
content_download_ms    // Response body download (minimal, headers only)
total_time_ms          // Total request time (TCP + TLS + TTFB + content download)
```

**Note on DNS Resolution**:
DNS timing is **NOT measured** in this task. The system's local DNS resolver is used (which is typically cached), so measurements would not reflect true DNS performance. For accurate DNS performance monitoring, use the dedicated **DNS Query task** which can query specific DNS servers directly.

## Configuration

### Basic Configuration

```toml
[[tasks]]
type = "http_get"
name = "Homepage Check"
schedule_seconds = 30
url = "https://www.example.com"
```

### Advanced Configuration

```toml
[[tasks]]
type = "http_get"
name = "API Health Endpoint"
schedule_seconds = 60
url = "https://api.example.com/health"
timeout_seconds = 15           # HTTP request timeout (default: 30s)
verify_ssl = true              # Enforce valid SSL certificate (default: false)

[tasks.headers]
"Authorization" = "Bearer secret-token-123"
"User-Agent" = "LinkSense-Monitor/1.0"
"X-Custom-Header" = "monitoring"
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"http_get"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between checks (seconds) |
| `url` | string | ✅ | - | Target URL (must start with http:// or https://) |
| `timeout_seconds` | integer | ❌ | 30 | Request timeout (seconds) |
| `verify_ssl` | boolean | ❌ | false | If true, enforce valid SSL certificate; if false, collect cert info but don't fail on invalid certs |
| `headers` | table | ❌ | {} | Custom HTTP headers (key-value pairs) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "api-prod", "cdn-us-east") |

### Configuration Examples

#### Monitor Multiple Endpoints
```toml
[[tasks]]
type = "http_get"
name = "Production API"
schedule_seconds = 30
url = "https://api.prod.example.com/v1/status"

[[tasks]]
type = "http_get"
name = "Staging API"
schedule_seconds = 60
url = "https://api.staging.example.com/v1/status"

[[tasks]]
type = "http_get"
name = "CDN Edge - US"
schedule_seconds = 30
url = "https://us-cdn.example.com/health"

[[tasks]]
type = "http_get"
name = "CDN Edge - EU"
schedule_seconds = 30
url = "https://eu-cdn.example.com/health"
```

#### Authenticated API Monitoring
```toml
[[tasks]]
type = "http_get"
name = "Internal Admin API"
schedule_seconds = 60
url = "https://admin.internal.example.com/api/v2/health"
timeout_seconds = 20

[tasks.headers]
"Authorization" = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
"X-API-Version" = "2.0"
```

#### Custom User Agent and Headers
```toml
[[tasks]]
type = "http_get"
name = "Mobile API Simulation"
schedule_seconds = 45
url = "https://mobile-api.example.com/feed"

[tasks.headers]
"User-Agent" = "LinkSense/1.0 (Monitoring Bot)"
"Accept" = "application/json"
"Accept-Language" = "en-US"
```

#### High-Frequency Critical Monitoring
```toml
[[tasks]]
type = "http_get"
name = "Payment Gateway"
schedule_seconds = 10          # Every 10 seconds
url = "https://payments.example.com/status"
timeout_seconds = 5            # Fail fast
```

#### SSL Certificate Monitoring
```toml
[[tasks]]
type = "http_get"
name = "Production API - SSL Monitor"
schedule_seconds = 3600        # Check once per hour
url = "https://api.example.com"
verify_ssl = true              # Fail if certificate invalid
timeout_seconds = 10
```

#### Monitor Endpoint with Self-Signed Certificate
```toml
[[tasks]]
type = "http_get"
name = "Internal Dev Server"
schedule_seconds = 60
url = "https://dev.internal.example.com"
verify_ssl = false             # Don't fail on invalid cert, but still track it
timeout_seconds = 10
```

## Metrics

### Raw Metrics (`raw_metric_http`)

Captured for each individual HTTP request:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when request was initiated |
| `status_code` | INTEGER | HTTP status code (200, 404, 500, etc.) - NULL if request failed |
| `tcp_timing_ms` | REAL | TCP connection establishment time |
| `tls_timing_ms` | REAL | TLS handshake time (NULL for HTTP, value for HTTPS) |
| `ttfb_timing_ms` | REAL | Time to First Byte (server processing time) |
| `content_download_timing_ms` | REAL | Response body download time |
| `total_time_ms` | REAL | Total request time (excludes DNS resolution) |
| `success` | BOOLEAN | Whether request succeeded (1) or failed (0) |
| `error` | TEXT | Error message if request failed (NULL on success) |
| `ssl_valid` | BOOLEAN | Whether SSL certificate is valid (NULL for HTTP, true/false for HTTPS) |
| `ssl_cert_days_until_expiry` | INTEGER | Days until SSL certificate expires (NULL for HTTP, can be negative if expired) |
| `target_id` | TEXT | Optional target identifier from configuration (NULL if not specified) |


### Aggregated Metrics (`agg_metric_http`)

60-second statistical summary with status code distribution:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of requests in period |
| `success_rate_percent` | REAL | Percentage of successful requests (0-100) |
| `avg_tcp_timing_ms` | REAL | Mean TCP connection time |
| `avg_tls_timing_ms` | REAL | Mean TLS handshake time |
| `avg_ttfb_timing_ms` | REAL | Mean Time to First Byte |
| `avg_content_download_timing_ms` | REAL | Mean download time |
| `avg_total_time_ms` | REAL | Mean total request time (excludes DNS) |
| `max_total_time_ms` | REAL | Maximum request time observed (excludes DNS) |
| `successful_requests` | INTEGER | Count of successful requests |
| `failed_requests` | INTEGER | Count of failed requests |
| `status_code_distribution` | TEXT | JSON object: `{"200": 55, "503": 5}` |
| `ssl_valid_percent` | REAL | Percentage of requests with valid SSL certificates (0-100, NULL for HTTP) |
| `avg_ssl_cert_days_until_expiry` | REAL | Average days until SSL certificate expiry (NULL for HTTP) |
| `target_id` | TEXT | Optional target identifier from configuration (first occurrence in period, NULL if not specified) |


### Timing Breakdown Interpretation

**Note**: DNS timing is not measured. Use the dedicated DNS Query task for DNS performance monitoring.

#### TCP Connection (`tcp_timing_ms`)
- **< 20ms**: Local network
- **20-100ms**: Normal internet connection
- **> 200ms**: High latency network path

#### TLS Handshake (`tls_timing_ms`)
- **< 50ms**: Modern TLS 1.3 fast handshake
- **50-150ms**: Normal TLS 1.2 handshake
- **> 200ms**: Slow TLS, check certificate chain or server load

#### Time to First Byte (`ttfb_timing_ms`)
- **< 100ms**: Fast server response (cached/static content)
- **100-500ms**: Normal dynamic content generation
- **> 1000ms**: Slow backend processing, investigate server

#### Content Download (`content_download_timing_ms`)
- **< 50ms**: Small response (< 10KB)
- **50-500ms**: Medium response or slower network
- **> 1000ms**: Large response or bandwidth constraint

### HTTP Status Code Interpretation

| Code Range | Meaning | Action |
|------------|---------|--------|
| 2xx | Success | Normal operation |
| 3xx | Redirect | Review if expected (may increase latency) |
| 4xx | Client Error | Check request configuration (auth, headers) |
| 5xx | Server Error | Alert - backend issue |
| NULL | Request Failed | Network/DNS/connection problem |

### Alerting Thresholds (Examples)

```
WARNING:
  - avg_ttfb_timing_ms > 500
  - success_rate_percent < 99
  - status_code_distribution contains 5xx

CRITICAL:
  - avg_ttfb_timing_ms > 2000
  - success_rate_percent < 95
  - successful_requests == 0
```

## Design Philosophy

### What It Does

Performs HTTP GET requests to target URLs and measures:
- **Availability**: Is the endpoint reachable and responding?
- **Timing Breakdown**: Granular latency at each protocol layer (DNS, TCP, TLS, TTFB, download)
- **Response Codes**: HTTP status code distribution over time
- **End-to-End Performance**: Total request completion time

### Typical Use Cases

- **API Availability Monitoring**: Track uptime of REST/GraphQL APIs
- **Web Application Health**: Monitor website homepage/critical pages
- **CDN Performance**: Measure edge server response times across regions
- **SLA Verification**: Ensure response time commitments are met
- **Performance Regression Detection**: Alert when TTFB degrades
- **Multi-Stage Analysis**: Identify which layer is causing slowdowns
- **Third-Party Service Monitoring**: Track dependencies (payment gateways, auth services)

### Limitations

- **No Content Validation**: Only checks HTTP status, not response body correctness (use HTTP Content task for that)
- **GET Only**: Cannot test POST/PUT/DELETE operations
- **No JavaScript**: Won't execute client-side code (not a browser)
- **SSL/TLS Validation**: Certificate validation can be controlled via `verify_ssl` parameter (default: false for monitoring flexibility)

## Performance Characteristics

### Resource Usage
- **CPU**: Low (~1-5% per concurrent request)
- **Memory**: ~100KB per request (includes response buffer)
- **Network**: Depends on response size (typically 1-100KB)
- **Disk I/O**: Batch writes to SQLite

### Execution Time
- **Typical**: 100-500ms (internet services)
- **Fast APIs**: 50-200ms (optimized backends)
- **Slow Endpoints**: 1-10s (legacy systems)
- **Timeout**: Configurable (default 30s) - **Properly enforced** at HTTP client level

### Scalability
- Can monitor **50+ endpoints** simultaneously on modest hardware
- Async I/O allows high concurrency without thread overhead
- Recommended schedule: 30-60 seconds for most endpoints

## Troubleshooting

### Common Issues

#### "SSL Certificate Verification Failed"
**Symptom**: HTTPS requests fail with certificate errors (only when `verify_ssl = true`)
**Causes**:
- Expired SSL certificate
- Self-signed certificate
- Hostname mismatch
- Missing intermediate certificates
**Solutions**:
- Fix certificate on target server (recommended for production)
- Set `verify_ssl = false` to monitor endpoints with invalid certs (still collects cert info)

#### High Total Request Time
**Symptom**: `total_time_ms` consistently high
**Causes**:
- Network latency (check `tcp_timing_ms`)
- TLS overhead (check `tls_timing_ms`)
- Slow backend (check `ttfb_timing_ms`)
- Large response size (check `content_download_timing_ms`)
**Solutions**:
- Use timing breakdown to isolate bottleneck
- For DNS issues, use the dedicated DNS Query task
- Optimize based on which phase shows highest values

#### Inconsistent TTFB
**Symptom**: `ttfb_timing_ms` varies wildly (50ms → 2000ms → 100ms)
**Causes**:
- Backend autoscaling (cold starts)
- Database query performance
- Cache hit/miss variations
**Solution**: Review server-side metrics, optimize backend

#### 5xx Status Codes
**Symptom**: `status_code_distribution` shows server errors
**Causes**:
- Application crashes
- Resource exhaustion (CPU, memory)
- Database connectivity issues
**Solution**: Check application logs and infrastructure

### Debugging Tips

**Enable verbose HTTP logging:**
```bash
RUST_LOG=debug,reqwest=trace ./agent /path/to/config
```

**Analyze timing patterns:**
```sql
SELECT
  task_name,
  avg_tcp_timing_ms,
  avg_tls_timing_ms,
  avg_ttfb_timing_ms,
  avg_total_time_ms
FROM agg_metric_http
WHERE period_start > (strftime('%s', 'now') - 3600)
ORDER BY avg_ttfb_timing_ms DESC;
```

**Compare with curl:**
```bash
# Get detailed timing with curl
curl -w "@curl-format.txt" -o /dev/null -s https://example.com

# curl-format.txt content:
time_namelookup: %{time_namelookup}s
time_connect: %{time_connect}s
time_appconnect: %{time_appconnect}s
time_pretransfer: %{time_pretransfer}s
time_starttransfer: %{time_starttransfer}s
time_total: %{time_total}s
```

## Best Practices

1. **Use /health Endpoints**:
   - Create dedicated health check endpoints
   - Return minimal JSON: `{"status": "ok"}`
   - Keep response size small for faster checks

2. **Set Appropriate Timeouts**:
   - Fast APIs: 5-10 seconds
   - External services: 15-30 seconds
   - Prevent tasks from hanging indefinitely

3. **Monitor Critical User Paths**:
   - Homepage (first impression)
   - Login endpoint (authentication)
   - Core API endpoints (business logic)
   - Payment/checkout (revenue impact)

4. **Include Custom Headers for Identification**:
   ```toml
   [tasks.headers]
   "User-Agent" = "LinkSense-Monitor/1.0"
   "X-Monitoring" = "true"
   ```
   Helps ops teams identify monitoring traffic in logs

5. **Compare Timings Across Regions**:
   - Deploy agents in different locations
   - Compare same endpoint from multiple agents
   - Identify geographic performance issues

6. **Set Realistic Schedule Intervals**:
   - Critical: 10-30 seconds
   - Normal: 60-300 seconds
   - Avoid overwhelming servers with checks


## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_HTTP_CONTENT.md](TASK_HTTP_CONTENT.md) - Content validation with regex
- [TASK_PING.md](TASK_PING.md) - Network layer connectivity testing
- [TASK_DNS.md](TASK_DNS.md) - DNS resolution monitoring
