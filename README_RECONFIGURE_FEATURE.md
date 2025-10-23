# Multi-Agent Reconfiguration Feature

## Overview

Bulk update task configurations for multiple agents by placing files in a monitored `reconfigure/` folder. The server validates, backs up existing configs, and applies changes automatically.

## Folder Structure

The server monitors a `reconfigure` folder located adjacent to (sibling of) the `agent_configs_dir`:
```
/etc/monitoring-server/
├── agent-configs/          # Individual agent configuration files (flat structure)
│   ├── agent1.toml
│   ├── agent2.toml
│   ├── web-server-01.toml
└── reconfigure/             # Monitored folder for bulk updates (sibling of agent-configs/)
    ├── agent_list.txt       # List of agents to update (place here to trigger)
    ├── tasks.toml          # New task configuration (place here to trigger)
    └── error.txt           # Error log (created by server if validation fails)
```

## Usage

#### `agent_list.txt`
Contains either:
- **All agents**: Single line with `ALL AGENTS`
- **Specific agents**: One agent ID per line

Example for specific agents:
```
agent1
agent2
web-server-01
```

# Provide new configuration
cp new_tasks.toml reconfigure/tasks.toml
```

**Update all agents:**
```bash
echo "ALL AGENTS" > reconfigure/agent_list.txt
cp new_tasks.toml reconfigure/tasks.toml
```

Server checks for reconfiguration requests at regular intervals (configurable via `reconfigure_check_interval_seconds` in server.toml, default: 10 seconds, range: 1-300). Processing occurs on the next check cycle after files are placed. On success, files are removed. On error, `error.txt` is created with timestamped details.

## Validation Rules

- Valid TOML syntax
- Supported task types:
  - `ping`, `http_get`, `http_content`, `dns_query`, `dns_query_doh`, `bandwidth`
  - `sql_query` (requires `--features sql-tasks` at build time)
- Minimum schedule requirements:
  - Bandwidth tasks: `schedule_seconds >= 60`
  - SQL query tasks: `schedule_seconds >= 60` (when feature enabled)
- Unique task names within configuration
- Valid agent IDs (alphanumeric, hyphens, underscores only)
- Agent config files must exist on server
- No duplicate agent IDs in agent_list.txt
- agent_list.txt cannot be empty

## Automatic Backups

Before applying changes, existing configurations are backed up with timestamps:
```
{agent_configs_dir}/{agent_id}.toml.backup.{timestamp}
```

Example: `agent1.toml.backup.1696800000`

Up to 10 most recent backups are retained per agent. Older backups are automatically cleaned up when the limit is exceeded.

## Partial Failure Handling

When updating multiple agents, the server attempts to update all agents even if some fail. The error.txt file will contain:
- Count of successfully updated agents
- Count of failed agents
- Detailed error messages for each failure

This allows successful updates to be applied while providing visibility into any failures.

## Error Handling

Previous error.txt is cleared at the start of each new reconfiguration attempt. If validation fails or agents fail to update, error.txt is created with timestamped error details. For multi-agent updates, all errors are included in a single error.txt file.

## Implementation

- `server/src/reconfigure.rs` - Core logic
- `shared/src/config.rs` - Validation (`TasksConfig::validate_from_toml()`)
- Runs at configurable intervals in server background loop (default: 10 seconds)
