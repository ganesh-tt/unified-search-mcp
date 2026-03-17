# unified-search-mcp

A lightweight Rust MCP server that searches Slack, Confluence, JIRA, and local files in parallel from a single tool.

## Why

Developer knowledge is scattered across Slack threads, Confluence pages, JIRA tickets, and local codebases. Finding "what did we decide about X?" means manually searching 4+ systems.

This MCP server exposes one `unified_search` tool that fans out queries to all sources in parallel, merges results with configurable ranking, and returns a single ranked list.

## Features

- **Parallel fan-out** — all sources queried simultaneously via tokio
- **Cross-source ranking** — weighted relevance scoring with deduplication
- **4 source adapters** — Slack, Confluence, JIRA, local files (ripgrep)
- **Lean** — single binary, ~10MB, ~20MB RAM, <50ms startup
- **Plugin architecture** — add new sources by implementing the `SearchSource` trait
- **Markdown table output** — token-efficient format for LLM consumption

## Quick Start

### 1. Build

```bash
cargo build --release
```

### 2. Configure

```bash
cp config.example.yaml config.yaml
# Edit config.yaml with your credentials
```

### 3. Set credentials as env vars

```bash
export SLACK_USER_TOKEN="xoxp-..."
export ATLASSIAN_BASE_URL="https://yourorg.atlassian.net"
export ATLASSIAN_EMAIL="you@example.com"
export ATLASSIAN_API_TOKEN="your-api-token"
```

### 4. Add to Claude Code

Add to your `.mcp.json`:
```json
{
  "mcpServers": {
    "unified-search": {
      "command": "/path/to/unified-search-mcp",
      "args": ["--config", "/path/to/config.yaml"]
    }
  }
}
```

## Auth Setup

### Slack
1. Create app at [api.slack.com/apps](https://api.slack.com/apps) → From Scratch
2. Add **User Token Scope**: `search:read`
3. Install to workspace → copy `xoxp-` token
4. Set `SLACK_USER_TOKEN` env var

### Atlassian (Confluence + JIRA)
1. Create API token at [id.atlassian.com](https://id.atlassian.com/manage-profile/security/api-tokens)
2. Set `ATLASSIAN_EMAIL`, `ATLASSIAN_API_TOKEN`, `ATLASSIAN_BASE_URL`

### Local Files
Configure paths in `config.yaml`. Install [ripgrep](https://github.com/BurntSushi/ripgrep) for best performance (falls back to built-in search).

## MCP Tools

| Tool | Description |
|------|-------------|
| `unified_search` | Search all enabled sources in parallel, return ranked Markdown table |
| `search_source` | Search a single named source, return detailed JSON |
| `list_sources` | Show enabled sources and their health status |
| `index_local` | Trigger vector index rebuild (Phase 2) |

## Configuration

See [config.example.yaml](config.example.yaml) for all options. Key settings:

- Per-source `enabled`, `weight`, `max_results`
- Global `timeout_seconds` (default 10)
- `${ENV_VAR}` interpolation for secrets
- Include/exclude glob patterns for local search

## Resource Usage

| Metric | Value |
|--------|-------|
| Binary size | ~10MB |
| Runtime RAM | ~20MB |
| Startup time | <50ms |
| Query latency | ~1-3s (slowest source) |

## License

MIT
