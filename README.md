# unified-search-mcp

A lightweight Rust MCP server that searches Slack, Confluence, JIRA, and local files in parallel from a single tool call.

## Why

Developer knowledge is scattered across Slack threads, Confluence pages, JIRA tickets, and local codebases. Finding "what did we decide about X?" means manually searching 4+ systems.

This MCP server gives your AI assistant one `unified_search` tool that fans out queries to all sources in parallel, merges results with configurable ranking, and returns a single ranked list with comments included.

## Features

- **Parallel fan-out** -- all sources queried simultaneously via tokio
- **Cross-source ranking** -- weighted relevance scoring with deduplication
- **Comments included** -- JIRA and Confluence search results automatically include recent comments
- **Deep-dive lookups** -- `get_detail` fetches full JIRA tickets, Confluence pages, or Slack threads with all comments, linked issues, subtasks, child pages, and thread replies
- **Auto-detection** -- pass a JIRA key (`FIN-1234`), Atlassian URL, or Slack permalink and the tool figures out what to fetch
- **Metrics & adoption tracking** -- JSONL telemetry + `--stats` CLI to see how often the tool is used vs individual MCPs
- **5 source adapters** -- Slack, Confluence, JIRA, local files (ripgrep), with a plugin architecture for adding more
- **Lean** -- single binary, ~7MB, ~20MB RAM, <50ms startup
- **Preflight check** -- `--verify` validates all credentials, scopes, and paths before first use

## Prerequisites

**Rust toolchain** (1.75+):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo --version
```

**Optional:** [ripgrep](https://github.com/BurntSushi/ripgrep) for faster local file search:

```bash
brew install ripgrep   # macOS
# or: cargo install ripgrep
```

## Quick Start

```bash
git clone https://github.com/your-org/unified-search-mcp.git
cd unified-search-mcp
cargo build --release
```

Binary is at `target/release/unified-search-mcp` (~7MB).

### 1. Get credentials

You only need credentials for sources you want to enable. Skip any you don't use.

| Source | What you need | Where to get it |
|--------|--------------|-----------------|
| **Slack** | User token (`xoxp-...`) | [api.slack.com/apps](https://api.slack.com/apps) -- Create App -- OAuth -- Add scope `search:read` -- Install -- Copy **User** OAuth Token |
| **Confluence + JIRA** | Email + API token | [id.atlassian.com/manage-profile/security/api-tokens](https://id.atlassian.com/manage-profile/security/api-tokens) |
| **Local files** | File paths only | No credentials. Optional: install [ripgrep](https://github.com/BurntSushi/ripgrep) for speed. |

> **Slack note:** You must use a **user token** (`xoxp-...`), not a bot token (`xoxb-...`). The `search.messages` API requires user-level access.

### 2. Create config

```bash
cp config.example.yaml config.yaml
```

Edit `config.yaml`. Set `enabled: false` for any source you don't use. Credentials use `${ENV_VAR}` references so the config file is safe to commit:

```yaml
server:
  name: "unified-search"
  max_results: 20
  timeout_seconds: 10
  metrics_path: "~/.unified-search/metrics.jsonl"   # optional

sources:
  slack:
    enabled: true
    user_token: "${SLACK_USER_TOKEN}"
    weight: 1.0
    max_results: 10

  confluence:
    enabled: true
    base_url: "${ATLASSIAN_BASE_URL}"       # e.g., https://yourorg.atlassian.net
    email: "${ATLASSIAN_EMAIL}"
    api_token: "${ATLASSIAN_API_TOKEN}"
    spaces: []                               # empty = search all spaces
    weight: 1.0
    max_results: 10

  jira:
    enabled: true
    base_url: "${ATLASSIAN_BASE_URL}"
    email: "${ATLASSIAN_EMAIL}"
    api_token: "${ATLASSIAN_API_TOKEN}"
    projects: []                             # empty = search all projects
    weight: 1.0
    max_results: 10

  local_text:
    enabled: true
    paths:
      - "~/projects/my-repo"
    include_patterns:
      - "**/*.{rs,py,scala,java,js,ts,go,sql,sh,toml,yaml,yml,md,txt}"
    exclude_patterns:
      - "**/target/**"
      - "**/node_modules/**"
      - "**/.git/**"
    weight: 0.8
    max_results: 10
```

### 3. Verify setup

```bash
# Set env vars (or export them in your shell profile)
export SLACK_USER_TOKEN="xoxp-..."
export ATLASSIAN_BASE_URL="https://yourorg.atlassian.net"
export ATLASSIAN_EMAIL="you@example.com"
export ATLASSIAN_API_TOKEN="your-api-token"

# Run preflight check
./target/release/unified-search-mcp --verify --config ./config.yaml
```

Example output:
```
unified-search-mcp v0.1.0 -- preflight check

[OK]  Config loaded from ./config.yaml (4 sources enabled)
[OK]  Slack: auth.test OK (320ms)
[OK]  Confluence: OK (180ms)
[OK]  JIRA: OK (150ms)
[OK]  Local text: paths accessible
       /Users/you/projects/my-repo -- directory, 1247 matching files
[OK]  ripgrep: ripgrep 14.1.0

Ready! 4 sources configured, 4 healthy.
```

### 4. Connect to your MCP client

**Claude Code** -- add to `~/.claude.json` (global) or `.mcp.json` (per-project):

```json
{
  "mcpServers": {
    "unified-search": {
      "command": "/absolute/path/to/unified-search-mcp",
      "args": ["--config", "/absolute/path/to/config.yaml"],
      "env": {
        "SLACK_USER_TOKEN": "xoxp-your-token",
        "ATLASSIAN_BASE_URL": "https://yourorg.atlassian.net",
        "ATLASSIAN_EMAIL": "you@example.com",
        "ATLASSIAN_API_TOKEN": "your-api-token"
      }
    }
  }
}
```

**Other MCP clients** -- any client that supports stdio transport can use this server. The command is `unified-search-mcp --config /path/to/config.yaml`.

## MCP Tools

| Tool | Description |
|------|-------------|
| `unified_search` | Search all enabled sources in parallel. Returns a ranked Markdown table with comments included. |
| `search_source` | Search a single named source (`slack`, `confluence`, `jira`, `local_text`). Returns JSON. |
| `get_detail` | Fetch full details for a specific item. Auto-detects JIRA keys, Atlassian URLs, Slack permalinks. Returns rich Markdown. |
| `list_sources` | Show enabled sources and their health/latency status. |
| `index_local` | Trigger vector index rebuild (Phase 2 -- not yet available). |

### `unified_search`

Searches all enabled sources in parallel and returns a ranked Markdown table.

```
Query: "broadcast threshold decision"

| # | Source | Title | Snippet | URL |
|---|--------|-------|---------|-----|
| 1 | confluence | Broadcast Threshold Design | We settled on 500 msg/s... --- Comments (2 total): [Bob, 2026-03-12]: threshold at 500... | https://... |
| 2 | jira | FIN-1234: Fix broadcast OOM | Queue grows unbounded... --- Comments (3 total): [Charlie, 2026-03-15]: Verified on staging... | https://... |
| 3 | slack | broadcast threshold... | We need to decide on the broadcast threshold... | https://... |

Sources: slack (320ms, 5 results, 12 comments) | jira (180ms, 8 results, 24 comments) | confluence (450ms, 3 results, 6 comments) | Total: 460ms
```

### `get_detail`

Fetches complete content for a single item. Accepts:
- JIRA key: `FIN-1234`
- JIRA URL: `https://yourorg.atlassian.net/browse/FIN-1234`
- Confluence URL: `https://yourorg.atlassian.net/wiki/spaces/PROD/pages/123456/Page+Title`
- Slack permalink: `https://yourorg.slack.com/archives/C06ABC/p1712000000123456`

Optional `source` parameter forces interpretation (e.g., `source: "confluence"` with a page title).

**JIRA response includes:** summary, description, status, assignee, reporter, labels, fix versions, linked issues, subtasks, all comments

**Confluence response includes:** full page body, labels, child pages, all comments

**Slack response includes:** original message, all thread replies, channel name, participant list

## Metrics & Adoption Tracking

Every tool call is logged to `~/.unified-search/metrics.jsonl` (configurable). View your adoption stats:

```bash
./target/release/unified-search-mcp --stats --days 7
```

```
=== Unified Search Adoption Report (last 7 days) ===

Tool Calls:
  unified_search:  45 calls  (avg 420ms, p50 380ms, p95 890ms)
  search_source:   12 calls  (avg 280ms, p50 250ms, p95 650ms)
  get_detail:       8 calls  (avg 350ms, p50 310ms, p95 700ms)

Bypasses (Claude used individual MCPs for search/read):
  jira_get:         6 calls
  conf_get:         3 calls

Adoption Rate: 88% (65 unified / 74 total search-like operations)
```

The stats command also scans Claude Code conversation logs (`~/.claude/projects/`) to detect when your AI assistant chose individual JIRA/Confluence/Slack MCP tools instead of unified-search. This helps measure whether unified-search is actually replacing the fragmented workflow.

## Configuration Reference

All settings in `config.yaml`:

```yaml
server:
  name: "unified-search"          # Server name reported to MCP clients
  max_results: 20                  # Global max results per query
  timeout_seconds: 10              # Per-source timeout
  log_level: "info"                # (reserved for future use)
  metrics_path: "~/.unified-search/metrics.jsonl"  # Metrics log path

sources:
  slack:
    enabled: true/false
    user_token: "xoxp-..."        # Must be user token, not bot token
    weight: 1.0                   # Relevance multiplier (higher = ranked higher)
    max_results: 10               # Max results from this source per query

  confluence:
    enabled: true/false
    base_url: "https://..."       # Your Atlassian instance URL
    email: "you@example.com"
    api_token: "..."
    spaces: ["DEV", "OPS"]        # Optional: restrict to specific spaces (empty = all)
    weight: 1.0
    max_results: 10

  jira:
    enabled: true/false
    base_url: "https://..."
    email: "you@example.com"
    api_token: "..."
    projects: ["FIN", "PLAT"]     # Optional: restrict to specific projects (empty = all)
    weight: 1.0
    max_results: 10

  local_text:
    enabled: true/false
    paths:                        # Directories to search (tilde expanded)
      - "~/projects/my-repo"
    include_patterns:             # Glob patterns to include
      - "**/*.{rs,py,js,ts,md}"
    exclude_patterns:             # Glob patterns to exclude
      - "**/target/**"
      - "**/node_modules/**"
      - "**/.git/**"
    weight: 0.8
    max_results: 10
```

**Environment variable interpolation:** Use `${VAR_NAME}` syntax in any string value. Missing env vars for disabled sources are silently ignored; missing vars for enabled sources produce a config error.

## Architecture

```
                          MCP Client (Claude Code, etc.)
                                    |
                              stdio transport
                                    |
                             +-----------+
                             | McpServer |  (mcp.rs — rmcp tool routing)
                             +-----------+
                                    |
                         +-------------------+
                         | UnifiedSearchServer| (server.rs — handler logic)
                         +-------------------+
                          /        |         \
                  unified_search  get_detail  list_sources
                         |         |
                +----------------+ |
                | SearchOrchestrator| (core.rs — fan-out, merge, rank, dedup)
                +----------------+
                 /    |     |    \
              Slack  Conf  JIRA  LocalText     (sources/*.rs — SearchSource trait)
```

**Adding a new source:** Implement the `SearchSource` trait (4 methods: `name`, `description`, `health_check`, `search`) and register it in `main.rs`. See `src/sources/slack.rs` for a complete example.

## Development

```bash
# Run tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Build debug
cargo build

# Build release
cargo build --release

# Preflight check
cargo run -- --verify --config config.yaml

# View adoption stats
cargo run -- --stats --days 7
```

### Test structure

| File | Tests | What it covers |
|------|-------|---------------|
| `tests/test_jira.rs` | 21 | Search, comments, get_detail, auth, errors |
| `tests/test_confluence.rs` | 20 | Search, comment enrichment, get_detail, errors |
| `tests/test_slack.rs` | 13 | Search, get_detail_thread, auth, rate limiting |
| `tests/test_core.rs` | 14 | Orchestrator fan-out, ranking, dedup, timeouts, per-source stats |
| `tests/test_server.rs` | 8 | MCP tool dispatch, get_detail wiring, error paths |
| `tests/test_resolve.rs` | 11 | Identifier auto-detection, URL parsing, force_source |
| `tests/test_metrics.rs` | 3 | JSONL logging, serialization |
| `tests/test_models.rs` | 13 | Data model serialization, ordering |
| `tests/test_config.rs` | 9 | YAML parsing, env var interpolation, validation |
| `tests/test_local_text.rs` | 12 | Ripgrep + fallback search, glob matching |
| `tests/test_integration.rs` | 9 | End-to-end flows |

All HTTP-based tests use [wiremock](https://crates.io/crates/wiremock) for deterministic mocking.

## Resource Usage

| Metric | Value |
|--------|-------|
| Binary size | ~7MB |
| Runtime RAM | ~20MB |
| Startup time | <50ms |
| Query latency | 400ms-1.5s (parallel fan-out, bounded by slowest source) |

## CLI Reference

```
unified-search-mcp [OPTIONS]

Options:
  --config <PATH>    Config file path (default: config.yaml)
  --verify           Run preflight checks and exit
  --stats            Show adoption report and exit
  --days <N>         Days to include in stats report (default: 7, used with --stats)
```

Without flags, the server starts on stdio and waits for MCP JSON-RPC messages.

## License

MIT
