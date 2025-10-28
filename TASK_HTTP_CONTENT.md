# HTTP Content Verification Task

The **HTTP Content** task goes beyond availability monitoring by validating that HTTP responses contain expected content patterns. It's designed to detect **silent failures** where an endpoint returns HTTP 200 but with incorrect, cached, or error content.

## Implementation Details

### Crate: `reqwest`
**Link**: [reqwest on crates.io](https://crates.io/crates/reqwest)

This task uses **`reqwest`**, a high-level async HTTP client with full response body processing.

**Key Characteristics**:
- **Full Body Download**: Reads complete response body into memory
- **Automatic Decompression**: Handles gzip, deflate, brotli transparently
- **Character Encoding**: Auto-detects and converts to UTF-8 (ISO-8859-1, UTF-16, etc.)
- **Async Streaming**: Efficient async body reading via Tokio
- **Regex Integration**: Works with `regex` crate for pattern matching

**Consequences**:
- ✅ **Complete Content Access**: Can validate response body, headers, and status
- ✅ **Decompression Built-in**: Automatically handles compressed responses (gzip, br)
- ✅ **Encoding Support**: Properly handles international characters and various encodings
- ✅ **Production Ready**: Most popular HTTP client in Rust ecosystem
- ✅ **Regex Validation**: Powerful pattern matching via `regex` crate (Rust regex engine)
- ✅ **Size Protection**: Automatic rejection of responses exceeding 100MB to prevent memory exhaustion
- ⚠️ **Higher Memory Usage**: Must buffer entire response (~response_size + overhead)
- ⚠️ **Slower Than HTTP GET**: Body download and processing adds latency
- ⚠️ **Size Limit**: Responses larger than 100MB are automatically rejected (configurable in code)

### Crate: `regex`
**Link**: [regex on crates.io](https://crates.io/crates/regex)

**Key Characteristics**:
- **Rust Regex Engine**: Fast, safe, guaranteed linear time complexity
- **Unicode Support**: Full UTF-8 and Unicode property support
- **No Backtracking**: Cannot have catastrophic backtracking (DoS protection)
- **Compiled Patterns**: Regex compiled once, reused for all checks
- ⚠️ **Syntax Differences**: Rust regex syntax differs slightly from PCRE/JavaScript
  - No lookahead/lookbehind (use alternatives)
  - No backreferences (use capturing groups differently)

**Response Processing Flow**:
```rust
1. reqwest: HTTP request sent (with timeout)
2. reqwest: Response headers received
3. Size check: Pre-flight check of Content-Length header
   - If Content-Length > 100MB: Reject immediately, close connection
   - If Content-Length ≤ 100MB or not present: Proceed to download
4. reqwest: Body downloaded (streamed, with decompression)
5. reqwest: Character encoding detection and UTF-8 conversion
6. Size check: Post-read verification of actual body size
   - If body > 100MB (no Content-Length was provided): Reject, free memory
7. regex: Pattern matching against body string (compiled per-request)
8. Memory: Explicit drop() of response body to free memory immediately
9. Result: regexp_match = true/false
```

**Memory Management Strategy** (code: `task_http_content.rs:15-127`):
- **MAX_RESPONSE_SIZE**: Hard limit of 100MB per response
- **Pre-flight check**: Inspect Content-Length header before reading body (saves bandwidth)
- **Post-read check**: Verify actual size for responses without Content-Length header
- **Explicit cleanup**: `drop(body_text)` immediately after regex matching to free memory
- **Connection termination**: `drop(response)` on oversized responses to prevent download

## Configuration

### Basic Configuration

```toml
[[tasks]]
type = "http_content"
name = "Homepage Integrity Check"
schedule_seconds = 60
url = "https://www.example.com"
regexp = "<title>Example Company</title>"
```

### Advanced Configuration

```toml
[[tasks]]
type = "http_content"
name = "API Health JSON Validation"
schedule_seconds = 30
url = "https://api.example.com/v1/health"
regexp = '"status"\\s*:\\s*"(ok|healthy)"'    # Matches "status": "ok" or "status":"healthy"
timeout_seconds = 15
```

### Configuration Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `type` | string | ✅ | - | Must be `"http_content"` |
| `name` | string | ✅ | - | Unique identifier for this task |
| `schedule_seconds` | integer | ✅ | - | Interval between checks (seconds) |
| `url` | string | ✅ | - | Target URL (must start with http:// or https://) |
| `regexp` | string | ✅ | - | Regular expression pattern to match in response body |
| `timeout_seconds` | integer | ❌ | 10 | Request timeout (seconds) |
| `timeout` | integer | ❌ | - | Task-level timeout override (seconds) |
| `target_id` | string | ❌ | - | Optional identifier for grouping/filtering targets (e.g., "content-prod", "api-staging") |

**⚠️ Response Size Limit:**
The agent enforces a **100MB maximum response size** to prevent memory exhaustion. Responses exceeding this limit are automatically rejected:
- **Pre-flight check**: If `Content-Length` header indicates size > 100MB, response is rejected before download
- **Post-read check**: If actual body size > 100MB (when no Content-Length provided), response is rejected and memory freed
- **Error reporting**: Size limit violations are reported in the `error` field with detailed messages
- **Configuration**: Limit is hardcoded in `agent/src/task_http_content.rs:15` as `MAX_RESPONSE_SIZE`

### Regular Expression Tips

#### Escaping in TOML
TOML requires escaping backslashes, so regex special characters need double escaping:

```toml
# Match "status": "ok" (with optional whitespace)
regexp = '"status"\\s*:\\s*"ok"'

# Match any valid email
regexp = '[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}'

# Match HTML title tag
regexp = '<title>.*Example.*</title>'
```

#### Recommended Patterns

**Simple text match:**
```toml
regexp = 'Welcome to Example'
```

**Case-insensitive match (use `(?i)` flag):**
```toml
regexp = '(?i)success'          # Matches "SUCCESS", "Success", "success"
```

**JSON field validation:**
```toml
regexp = '"version"\\s*:\\s*"\\d+\\.\\d+\\.\\d+"'    # Matches "version": "1.2.3"
```

**Match any of multiple values:**
```toml
regexp = 'status"\\s*:\\s*"(ok|healthy|running)"'
```

**Ensure absence of error keywords:**
```toml
regexp = '^((?!error|exception|fail).)*$'    # Fails if "error" found anywhere
```

### Configuration Examples

#### API Health Validation
```toml
[[tasks]]
type = "http_content"
name = "User Service Health"
schedule_seconds = 60
url = "https://api.example.com/users/health"
regexp = '"status"\\s*:\\s*"ok"'
timeout_seconds = 10
```

#### Homepage Defacement Detection
```toml
[[tasks]]
type = "http_content"
name = "Homepage Title Check"
schedule_seconds = 300          # Every 5 minutes
url = "https://www.example.com"
regexp = '<title>Example Corp - Leading Provider of.*</title>'
```

#### Error Message Detection
```toml
[[tasks]]
type = "http_content"
name = "API Error Detection"
schedule_seconds = 60
url = "https://api.example.com/v1/data"
regexp = '^((?!Internal Server Error|Database connection failed).)*$'
```

#### JSON Array Validation
```toml
[[tasks]]
type = "http_content"
name = "Products List Validation"
schedule_seconds = 120
url = "https://api.example.com/products"
regexp = '"products"\\s*:\\s*\\['    # Ensure products array exists
```

#### Version String Check
```toml
[[tasks]]
type = "http_content"
name = "API Version Check"
schedule_seconds = 600          # Every 10 minutes
url = "https://api.example.com/version"
regexp = '"version"\\s*:\\s*"2\\.[0-9]+\\.[0-9]+"'    # Matches version 2.x.x
```

#### Multi-Pattern Monitoring (use multiple tasks)
```toml
# Task 1: Check for success indicator
[[tasks]]
type = "http_content"
name = "API Success Indicator"
schedule_seconds = 60
url = "https://api.example.com/status"
regexp = '"status"\\s*:\\s*"ok"'

# Task 2: Check for NO error indicators
[[tasks]]
type = "http_content"
name = "API No Errors"
schedule_seconds = 60
url = "https://api.example.com/status"
regexp = '^((?!error|exception).)*$'
```

## Metrics

### Raw Metrics (`raw_metric_http_content`)

Captured for each individual HTTP content check:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task from configuration |
| `timestamp` | INTEGER | Unix epoch when request was initiated |
| `status_code` | INTEGER | HTTP status code (200, 404, 500, etc.) - NULL if request failed |
| `total_time_ms` | REAL | Total request completion time (ms) |
| `total_size` | INTEGER | Response body size in bytes - NULL if failed |
| `regexp_match` | BOOLEAN | Whether regex matched response body (1=yes, 0=no) |
| `success` | BOOLEAN | Whether request succeeded (1) or failed (0) |
| `error` | TEXT | Error message if request failed (NULL on success). See Error Message Types below. |
| `target_id` | TEXT | Optional target identifier from task configuration |


**Important**: `success=1` means HTTP request succeeded. Check `regexp_match` to see if content was valid!

#### Error Message Types

The `error` field contains one of the following message formats when a request fails:

| Error Type | Format | Example | Cause |
|------------|--------|---------|-------|
| Response Size (Pre-flight) | `Response size ({N} bytes) exceeds maximum of 100MB (detected from Content-Length header)` | `Response size (150000000 bytes) exceeds maximum of 104857600 bytes...` | Content-Length header indicates response > 100MB |
| Response Size (Post-read) | `Response size ({N} bytes) exceeds maximum of 100MB (Content-Length header not provided)` | `Response size (120000000 bytes) exceeds maximum of 104857600 bytes...` | Actual body size > 100MB, no Content-Length header |
| Body Read Failure | `Failed to read response body from {url}: {error}` | `Failed to read response body from https://api.example.com: connection reset` | Network error during body download |
| Timeout | `Request to {url} timed out after {N}s` | `Request to https://slow-api.com timed out after 10s` | Request exceeded timeout_seconds |
| Request Failure | `Request to {url} failed: {error}` | `Request to https://invalid.local failed: dns error` | Connection failed, DNS error, TLS error, etc. |
| Regex Compilation | (via anyhow error) | `Failed to compile regexp: [invalid(regex` | Invalid regex pattern in configuration |

**Note**: Regex compilation errors prevent task execution entirely and are reported through the task error mechanism (not in the `error` field).

### Aggregated Metrics (`agg_metric_http_content`)

60-second statistical summary:

| Field | Type | Description |
|-------|------|-------------|
| `id` | INTEGER | Auto-incrementing primary key |
| `task_name` | TEXT | Name of the task |
| `period_start` | INTEGER | Unix epoch of aggregation period start |
| `period_end` | INTEGER | Unix epoch of aggregation period end |
| `sample_count` | INTEGER | Total number of requests in period |
| `success_rate_percent` | REAL | Percentage of successful HTTP requests (0-100) |
| `avg_total_time_ms` | REAL | Mean request completion time |
| `max_total_time_ms` | REAL | Maximum request time observed |
| `avg_total_size` | REAL | Mean response body size (bytes) |
| `regexp_match_rate_percent` | REAL | Percentage of successful requests where regex matched |
| `successful_requests` | INTEGER | Count of successful HTTP requests |
| `failed_requests` | INTEGER | Count of failed HTTP requests |
| `regexp_matched_count` | INTEGER | Count of requests where pattern matched |
| `target_id` | TEXT | Optional target identifier from task configuration |


### Metrics Interpretation

#### Success vs. Content Match
Critical distinction:
- `success_rate_percent`: HTTP availability (server responding)
- `regexp_match_rate_percent`: Content validity (server responding *correctly*)

**Scenario Analysis:**
```
success_rate_percent=100%, regexp_match_rate_percent=100%
→ Perfect: Server up and serving correct content

success_rate_percent=100%, regexp_match_rate_percent=0%
→ CRITICAL: Server up but serving WRONG content (silent failure!)

success_rate_percent=50%, regexp_match_rate_percent=N/A
→ Server intermittently down (network issue)
```

#### Response Size Anomalies
- **avg_total_size drops significantly**: Possible error page being served
- **avg_total_size increases dramatically**: Possible content bloat or extra data
- **Consistent size**: Healthy, predictable responses

#### Performance Patterns
- **avg_total_time_ms**: Should be stable for same endpoint
- Sudden increases may indicate:
  - Backend database slowness
  - Larger response size
  - Network congestion

### Alerting Thresholds (Examples)

```
WARNING:
  - regexp_match_rate_percent < 95
  - avg_total_size deviates > 20% from baseline
  - avg_total_time_ms > 1000

CRITICAL:
  - regexp_match_rate_percent < 50
  - regexp_matched_count == 0 (for last 5 minutes)
  - success_rate_percent < 90
```

## Design Philosophy

### What It Does

Performs HTTP GET requests and validates response body against a regular expression pattern:
- **Availability**: Is the endpoint responding?
- **Content Validation**: Does the response contain expected text/pattern?
- **Performance Tracking**: How long does it take to fetch and validate?
- **Size Monitoring**: Track response body size over time

### Strong Sides

1. **Silent Failure Detection**: Catches issues HTTP status codes miss (wrong content served successfully)
2. **Regex Flexibility**: Validate complex patterns (JSON keys, HTML elements, error messages)
3. **Cache Validation**: Ensure CDN/proxy serving current content, not stale data
4. **API Contract Verification**: Confirm expected fields present in responses
5. **Security Monitoring**: Detect defacement or injected content
6. **Application-Level Health**: Goes beyond "server responding" to "server responding correctly"

### Typical Use Cases

- **API Response Validation**: Ensure JSON contains expected keys (`"status":"ok"`)
- **Web Page Integrity**: Verify homepage contains company name/logo text
- **Error Detection**: Alert if response contains error messages even with 200 status
- **CDN Cache Verification**: Confirm edge servers serving updated content
- **Configuration Checks**: Validate service returns correct feature flags
- **Security Monitoring**: Detect unauthorized changes to public pages
- **Multi-Tenant Validation**: Confirm correct customer content served (white-label apps)

### Limitations

- **Performance Impact**: Downloads full response body (unlike HTTP GET which can skip body)
- **Regex Complexity**: Poorly designed patterns may miss issues or false-positive
- **No JavaScript**: Won't execute client-side rendering (not a browser)
- **Memory Usage**: Large responses consume more memory during validation
- **Single Pattern**: Each task validates one pattern (create multiple tasks for multiple validations)

## Performance Characteristics

### Resource Usage
- **CPU**: Low (~1-5% per concurrent request)
- **Memory**: ~(response_size + 1MB) per request
  - Small API (10KB): ~1MB
  - Medium page (100KB): ~2MB
  - Large response (1MB): ~2-3MB
- **Network**: Downloads full response body
- **Disk I/O**: Batch writes to SQLite

### Execution Time
- **API endpoints**: 100-500ms (small JSON responses)
- **Web pages**: 200-1000ms (HTML + inline resources)
- **Large responses**: 1-5s (MB-sized content, up to 100MB max)
- **Timeout**: Configurable (default 10s, defined in `shared/src/defaults.rs:15`) - Applied at reqwest client level via `.timeout()` method
- **Regex compilation**: ~1-5ms per request (pattern compiled fresh each time)

### Scalability
- Can monitor **30-50 content endpoints** on modest hardware
- Limit depends on response sizes (more small responses = more concurrent tasks)
- **100MB response limit** prevents single endpoint from exhausting memory
- Recommended schedule: 60-300 seconds (content changes less frequently than availability)
- **Note**: Each response consumes ~(response_size + 1MB) during processing, then freed immediately

## Troubleshooting

### Common Issues

#### Regex Never Matches Despite Correct Content
**Symptom**: `regexp_match=0` even when manually viewing shows expected content
**Causes**:
- TOML escaping issues (missing `\\` for regex `\`)
- Case sensitivity (regex is case-sensitive by default)
- Whitespace variations (extra spaces/newlines in response)
- Response encoding issues

**Solutions**:
```toml
# Add (?i) for case-insensitive matching
regexp = '(?i)success'

# Use \s* for flexible whitespace
regexp = '"status"\\s*:\\s*"ok"'

# Test regex outside agent first
# Python: import re; re.search(r'pattern', content)
```

#### False Positives (Matches When It Shouldn't)
**Symptom**: Regex matches error pages or unexpected content
**Cause**: Pattern too generic
**Solution**: Make pattern more specific
```toml
# Too generic:
regexp = 'ok'           # Matches "error: ok to retry" (bad!)

# More specific:
regexp = '"status"\\s*:\\s*"ok"'    # Only matches JSON structure
```

#### Memory Usage Spikes
**Symptom**: Agent memory grows with this task enabled
**Causes**:
- Response bodies too large (multi-MB responses)
- Too many concurrent content checks
- Responses approaching or exceeding 100MB limit
**Solutions**:
- Check logs for "Response size exceeds maximum" warnings
- Use HTTP GET task instead (doesn't download body)
- Reduce check frequency or number of concurrent tasks
- Request smaller endpoint (e.g., /health instead of full data dump)
- Increase `schedule_seconds` to reduce overlap
**Note**: The agent automatically rejects responses > 100MB to prevent memory exhaustion

#### Inconsistent Match Rates
**Symptom**: `regexp_match_rate_percent` fluctuates (80% → 95% → 70%)
**Causes**:
- CDN serving different content versions
- A/B testing showing different variants
- Load balancer routing to servers with different content
**Solution**: Review application deployment/caching strategy

### Debugging Tips

**Validate regex pattern locally:**
```bash
# Fetch response
curl -s https://api.example.com/health > response.txt

# Test regex (Rust-compatible)
# Using ripgrep (rg) which uses Rust regex engine
rg -o '"status"\s*:\s*"ok"' response.txt
```

**Check recent mismatches:**
```sql
SELECT timestamp, status_code, total_size, regexp_match, error
FROM raw_metric_http_content
WHERE task_name = 'Homepage Check'
  AND regexp_match = 0
ORDER BY timestamp DESC
LIMIT 10;
```

**Enable debug logging:**
```bash
RUST_LOG=debug ./agent /path/to/config
# Look for logs showing response body (truncated)
```

## Best Practices

1. **Keep Patterns Simple and Specific**:
   ```toml
   # Good: Specific JSON structure
   regexp = '"status"\\s*:\\s*"ok"'

   # Avoid: Too generic
   regexp = 'ok'
   ```

2. **Test Regex Before Deploying**:
   - Use online regex testers (regex101.com)
   - Select "Rust" flavor
   - Test against actual response samples

3. **Monitor Response Size Changes**:
   - Sudden size drops may indicate error pages
   - Gradual increases may indicate data growth

4. **Use Multiple Tasks for Complex Validation**:
   - Task 1: Check for success indicator
   - Task 2: Check absence of error keywords
   - Task 3: Validate specific data field present

5. **Set Appropriate Schedules**:
   - Content validation: 60-300 seconds (less urgent than availability)
   - Static pages: 300-600 seconds (change infrequently)
   - API responses: 60-120 seconds (dynamic but predictable)

6. **Combine with HTTP GET for Complete Picture**:
   ```toml
   # Fast availability check
   [[tasks]]
   type = "http_get"
   name = "API Availability"
   schedule_seconds = 30
   url = "https://api.example.com/health"

   # Slower content validation
   [[tasks]]
   type = "http_content"
   name = "API Content Validation"
   schedule_seconds = 120
   url = "https://api.example.com/health"
   regexp = '"status"\\s*:\\s*"ok"'
   ```

## Related Documentation

- [DATABASE.md](DATABASE.md) - Complete database schema
- [TASK_HTTP_GET.md](TASK_HTTP_GET.md) - HTTP availability monitoring (faster, no body download)
- [TASK_PING.md](TASK_PING.md) - Network layer connectivity testing
- [TASK_DNS.md](TASK_DNS.md) - DNS resolution monitoring
