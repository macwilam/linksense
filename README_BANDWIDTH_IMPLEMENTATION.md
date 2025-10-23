# Bandwidth Measurement Implementation

## Overview

Bandwidth testing measures network throughput between agent and central server using coordinated downloads to prevent concurrent tests and ensure accurate measurements.

## Key Features

### Server Coordination
- `BandwidthTestManager` ensures only one agent performs bandwidth test at a time
- FIFO queue with automatic advancement when tests complete
- Server responds with "proceed" (with data size) or "delay" (with suggested retry time)
- Automatic cleanup of expired test states (120s timeout) and old queue entries (300s max)
- Intelligent delay calculation: 60s base + 30s per queued agent (capped at 300s)

### Configuration
- **Server**: `bandwidth_test_size_mb` in `server.toml` (default: 10MB) - **server-controlled only**
- **Agent**: Minimum 60-second `schedule_seconds` enforced (validation in `TaskConfig::validate()`)
- **Security**: Agent cannot influence download size - server controls all test parameters

### Measurement Method
- Measures time from first to last byte of download
- Calculates bandwidth: `(bytes * 8) / (seconds) / 1_000_000` = Mbps
- Two-phase timeout: Permission request (max 10s) + Download (full `timeout_seconds`)
- Test data: Zero-filled bytes generated in 64 KB chunks

## API Flow

1. **Request Permission**: Agent → `POST /api/v1/bandwidth_test`
   - Headers: `X-API-Key`, `Content-Type: application/json`
   - Body: `{"agent_id": "agent1", "timestamp_utc": "2024-01-01T00:00:00Z"}`
   - Server validates: API key, agent ID format, whitelist, rate limits
   - Server checks queue state
   - Returns action (`proceed`/`delay`), delay_seconds, and data_size_bytes

2. **Download Test Data**: Agent → `GET /api/v1/bandwidth_download?agent_id=X`
   - Query params: Only `agent_id` (no size parameter - server-controlled)
   - Server validates: Agent has active test in progress
   - Agent measures download time
   - Calculates bandwidth and stores metric

3. **Automatic Cleanup**: Server releases test slot after 120s timeout or when next agent requests test

## Queue Mechanism Details

The `BandwidthTestManager` maintains sophisticated queue state:

- **Current Test Tracking**: Stores agent ID and start time
- **Waiting Queue**: FIFO queue of waiting agents with timestamps
- **Auto-Advancement**: When test completes or times out, first queued agent auto-starts
- **Test Timeout**: 120 seconds - tests exceeding this are cleaned up automatically
- **Queue Expiry**: Queue entries older than 300s (max_delay) are removed
- **Delay Calculation**: Base 60s + (queue_length × 30s), capped at 300s

**Important**: Server never returns "queue full" errors. It always accepts requests and returns either:
- `action: "proceed"` + `data_size_bytes` - Agent can start immediately
- `action: "delay"` + `delay_seconds` - Agent should retry after suggested delay

## Validation and Security

- **API Key**: Required via `X-API-Key` header
- **Agent ID Validation**: Alphanumeric, hyphens, underscores only; max 128 chars
- **Agent Whitelist**: Optional `agent_id_whitelist` in server.toml restricts access
- **Rate Limiting**: Configurable per-agent rate limits (if enabled)
- **Active Test Verification**: Download endpoint requires agent to have active test
- **Server-Controlled Size**: Agent cannot influence test data size in any way

## Implementation Files

- `server/src/bandwidth_state.rs` - Test coordination and queue management
- `server/src/api.rs` - API endpoints: `handle_bandwidth_test()`, `handle_bandwidth_download()`
- `agent/src/tasks.rs` - `perform_bandwidth_measurement()`
- `shared/src/api.rs` - Request/response types (`BandwidthTestRequest`, `BandwidthTestResponse`, `BandwidthTestAction`)
- `shared/src/config.rs` - Validation (60s minimum schedule)
