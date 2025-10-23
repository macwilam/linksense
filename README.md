<div align="center">
  <img src="link_sense_logo_cut_small.png" alt="LinkSense Logo" width="300">
</div>

# LinkSense - Network Health and Performance Monitoring System

A high-performance, async Rust-based distributed network monitoring solution designed for deploying multiple lightweight agents that collect and report network performance metrics to a centralized server.

**BETA VERSION:** LinkSense is currently in beta. The software is feature-complete and fully functional, but has undergone only preliminary testing. All agent and server tasks are being actively tested. Note: The SQL task feature is in alpha stage and requires the `sql-tasks` feature flag.

## 🎯 What is LinkSense?

LinkSense enables organizations to monitor network health and performance across distributed infrastructure through a simple yet powerful agent-server architecture:

- **Deploy agents** anywhere in your network infrastructure
- **Collect metrics** centrally from all agents
- **Monitor continuously** with minimal resource overhead
- **Scale effortlessly** to hundreds of monitoring targets per agent
- **Run standalone** when centralized collection isn't needed

## ✨ Monitoring Capabilities

LinkSense supports eight task types, each optimized for specific monitoring needs:

| Task Type | Purpose | Key Metrics |
|-----------|---------|-------------|
| **ICMP Ping** | Network connectivity | RTT, packet loss |
| **TCP** | TCP port connectivity | Connection time, success/failure |
| **TLS Handshake** | SSL/TLS certificate validation | Handshake time, certificate validity |
| **HTTP GET** | Web service health | DNS, TCP, TLS, TTFB timing |
| **HTTP Content** | Response validation | Status code, regex match |
| **DNS Query** | Resolution performance | Query time, record count |
| **Bandwidth** | Throughput testing | Mbps, transfer time |
| **SQL Query** | Database health | Query time, row count |

Each task type has detailed documentation in `TASK_*.md` files.

### Agent-Server Model

**Agents** are lightweight monitoring services that:
- Execute network tests (ping, TCP, TLS, HTTP, DNS, bandwidth, SQL)
- Store metrics locally in SQLite
- Aggregate raw measurements into 60-second summaries
- Send aggregated metrics to the central server
- Automatically sync configuration from server
- Can operate standalone without server connection

**Server** is the central coordination point that:
- Receives and stores metrics from all agents
- Manages agent-specific configurations
- Coordinates bandwidth tests to prevent conflicts
- Validates and distributes configuration updates
- Provides RESTful API for agent communication


## 📊 Information Flow

The system operates on a continuous cycle:

```
┌──────────────────────────────────────────────────────────┐
│ AGENT - Continuous Monitoring                            │
│                                                          │
│  1. Execute Tasks (ping, HTTP, DNS, etc.)               │
│     └─→ Store raw measurements in SQLite                │
│                                                          │
│  2. Every 60 seconds: Aggregate                          │
│     └─→ SQL GROUP BY → 1-minute summaries               │
│                                                          │
│  3. Send to Server                                       │
│     └─→ POST /api/v1/metrics                            │
│     └─→ Include BLAKE3 hash of current config           │
└──────────────────────────────────────────────────────────┘
                         │
                         ↓
┌──────────────────────────────────────────────────────────┐
│ SERVER - Central Collection                              │
│                                                          │
│  1. Authenticate agent (API key validation)              │
│                                                          │
│  2. Compare configuration hash                           │
│     ├─→ Match: Store metrics normally                   │
│     └─→ Mismatch: Agent downloads new config            │
│                                                          │
│  3. Store metrics in SQLite (tagged with agent_id)       │
│                                                          │
│  4. Update agent status (last_seen, metric counts)       │
└──────────────────────────────────────────────────────────┘
```

### Key Design Principles

**Local-First Architecture**: Agents store all metrics locally before sending to server. This ensures:
- No data loss if server is temporarily unavailable
- Agents continue monitoring during network issues
- Local querying capability for debugging

**Aggregation Strategy**: Raw measurements are aggregated into 60-second summaries:
- Reduces network bandwidth by 98%+
- Decreases server storage requirements
- Maintains statistical accuracy (min, max, avg, stddev)

**Configuration as Code**: All monitoring tasks defined in TOML files:
- Version-controllable configuration
- Bulk updates across multiple agents
- Automatic validation before distribution
- Hash-based change detection


## 🔄 Configuration Management

### Dynamic Configuration Updates

Agents automatically detect configuration changes through BLAKE3 hash comparison:

1. Agent sends current config hash with every metric batch
2. Server compares hash with master configuration
3. On mismatch, agent downloads new configuration
4. Agent validates, backs up old config, and applies new config
5. Configuration errors are reported back to server

This enables zero-downtime reconfiguration of monitoring tasks.

### Bulk Reconfiguration

Update multiple agents simultaneously by placing files in server's `reconfigure/` directory:

```bash
# Create agent list and new configuration
echo -e "agent1\nagent2\nagent3" > reconfigure/agent_list.txt
cp new_monitoring_tasks.toml reconfigure/tasks.toml

# Server processes within 10 seconds
# - Validates configuration
# - Backs up existing configs
# - Distributes to all listed agents
```

See [README_RECONFIGURE_FEATURE.md](README_RECONFIGURE_FEATURE.md) for details.

## 🔐 Security Model

**Authentication**: All agent-server communication requires API key validation via `X-API-Key` header.

**Authorization**: Optional agent ID whitelist restricts which agents can connect to server.

**Configuration Validation**: Server validates all configurations before distribution:
- TOML syntax checking
- Task parameter validation
- Interval constraint enforcement (e.g., bandwidth ≥60s)

**Backups**: Automatic configuration backups prevent data loss during updates.

**SQL Tasks**: Read-only database accounts recommended for monitoring queries.

## 📈 Scalability

The async architecture enables impressive scalability:

- **Single agent**: Monitors 100+ targets with minimal CPU/memory
- **Concurrent execution**: Tokio runtime handles thousands of simultaneous tasks
- **Staggered scheduling**: Prevents resource spikes and thundering herd
- **Efficient aggregation**: 60-second summaries reduce data volume by 98%+
- **Bandwidth coordination**: Server prevents test conflicts across agents

Real-world performance: A single agent can monitor hundreds of endpoints while using less than 50MB RAM.

## 🚀 Quick Start

### Prerequisites

- Rust 1.70+ and Cargo
- SQLite (bundled with Rust builds)
- Network connectivity between agents and server (for centralized mode)
- Linux: For ICMP ping, add user to ping group or grant CAP_NET_RAW

### Build

```bash
# Build all components
cargo build --release

# Build with SQL monitoring support
cargo build --release --features sql-tasks
```

### Installation

1. **Server Setup**: See [README_SERVER.md](README_SERVER.md)
2. **Agent Setup**: See [README_AGENT.md](README_AGENT.md)

## 📚 Documentation

### Getting Started
- **[README_AGENT.md](README_AGENT.md)** - Agent installation, configuration, and operation
- **[README_SERVER.md](README_SERVER.md)** - Server setup, API, and management

### Task Documentation
- **[TASK_PING.md](TASK_PING.md)** - ICMP ping monitoring
- **[TASK_TCP.md](TASK_TCP.md)** - TCP port connectivity monitoring
- **[TASK_TLS.md](TASK_TLS.md)** - TLS handshake and certificate validation
- **[TASK_HTTP_GET.md](TASK_HTTP_GET.md)** - HTTP/HTTPS endpoint monitoring
- **[TASK_HTTP_CONTENT.md](TASK_HTTP_CONTENT.md)** - HTTP response validation
- **[TASK_DNS.md](TASK_DNS.md)** - DNS query monitoring
- **[TASK_BANDWIDTH.md](TASK_BANDWIDTH.md)** - Bandwidth testing
- **[TASK_SQL.md](TASK_SQL.md)** - Database query monitoring

### Architecture and Implementation
- **[README_DATABASE.md](README_DATABASE.md)** - Database schema and design
- **[README_BANDWIDTH_IMPLEMENTATION.md](README_BANDWIDTH_IMPLEMENTATION.md)** - Bandwidth test coordination
- **[README_RECONFIGURE_FEATURE.md](README_RECONFIGURE_FEATURE.md)** - Bulk reconfiguration system


## 🐛 Troubleshooting

For component-specific issues:
- Agent problems: See [README_AGENT.md](README_AGENT.md) troubleshooting section
- Server problems: See [README_SERVER.md](README_SERVER.md) troubleshooting section


## 📝 License

MIT License - see LICENSE file for details.

---

**Built with Rust for performance, safety, and reliability.**
