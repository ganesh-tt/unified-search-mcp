# unified-search-mcp

A lightweight Rust MCP server that searches Slack, Confluence, JIRA, and local files in parallel from a single tool.

## Why

Developer knowledge is scattered across Slack threads, Confluence pages, JIRA tickets, and local codebases. Finding "what did we decide about X?" means manually searching 4+ systems.

This MCP server exposes one `unified_search` tool that fans out queries to all sources in parallel, merges results with configurable ranking, and returns a single ranked list.

## Features

- **Parallel fan-out** — all sources queried simultaneously via tokio
- **Cross-source ranking** — weighted relevance scoring with deduplication
- **4 source adapters** — Slack, Confluence, JIRA, local files (ripgrep)
- **Lean** — single binary, ~7MB, ~20MB RAM, <50ms startup
- **Preflight check** — `--verify` validates all credentials, scopes, and paths before first use
- **Plugin architecture** — add new sources by implementing the `SearchSource` trait
- **Markdown table output** — token-efficient format for LLM consumption

## Installation

```bash
git clone https://github.com/ganesh-tt/unified-search-mcp.git
cd unified-search-mcp
cargo build --release
```

Binary is at `target/release/unified-search-mcp` (~7MB).

## Setup (3 steps)

### Step 1: Get your credentials

You only need credentials for the sources you want. Skip any you don't use.

| Source | What you need | Where to get it |
|--------|--------------|-----------------|
| **Slack** | User token (`xoxp-...`) | [api.slack.com/apps](https://api.slack.com/apps) → Create App → OAuth → Add scope `search:read` → Install → Copy user token |
| **Confluence + JIRA** | Email + API token | [id.atlassian.com/manage-profile/security/api-tokens](https://id.atlassian.com/manage-profile/security/api-tokens) → Create token |
| **Local files** | Just file paths | No credentials needed. Install [ripgrep](https://github.com/BurntSushi/ripgrep) for speed (optional). |

### Step 2: Create config

```bash
cp config.example.yaml config.yaml
```

Edit `config.yaml` — set `enabled: false` for any source you don't want. Credentials use env vars so the config file is safe to share:

```yaml
sources:
  slack:
    enabled: true           # set false to skip
    user_token: "${SLACK_USER_TOKEN}"
  confluence:
    enabled: true
    base_url: "${ATLASSIAN_BASE_URL}"
    email: "${ATLASSIAN_EMAIL}"
    api_token: "${ATLASSIAN_API_TOKEN}"
  jira:
    enabled: true
    base_url: "${ATLASSIAN_BASE_URL}"
    email: "${ATLASSIAN_EMAIL}"
    api_token: "${ATLASSIAN_API_TOKEN}"
  local_text:
    enabled: true
    paths:
      - "~/projects/my-repo"
      - "~/documents/notes"
```

### Step 3: Add to your MCP client

**Claude Code** — add to `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "unified-search": {
      "command": "ABSOLUTE_PATH_TO/unified-search-mcp",
      "args": ["--config", "ABSOLUTE_PATH_TO/config.yaml"],
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

Replace `ABSOLUTE_PATH_TO` with actual paths. That's it — Claude Code will now have a `unified_search` tool.

## Verify Setup

Before using, run the built-in preflight check:

```bash
./target/release/unified-search-mcp --verify --config ./config.yaml
```

This checks **everything** and tells you exactly what's wrong:

| Check | What it verifies |
|-------|-----------------|
| Config file | Exists, valid YAML, env vars resolved |
| Slack | Token format (`xoxp-` prefix), `auth.test` API call succeeds, `search:read` scope present |
| Confluence | Base URL reachable, Basic Auth accepted, at least 1 space accessible |
| JIRA | Base URL reachable, Basic Auth accepted, `/rest/api/3/myself` returns your user |
| Local paths | Each configured path exists and is readable |
| ripgrep | `rg --version` found in PATH (warns if missing, not fatal) |

Example output:
```
unified-search-mcp v0.1.0 — preflight check

[OK]  Config loaded from ./config.yaml (4 sources enabled)
[OK]  Slack: authenticated as @ganesh (scope: search:read)
[OK]  Confluence: connected to yourorg.atlassian.net (3 spaces accessible)
[OK]  JIRA: connected as you@example.com
[WARN] Local path ~/old-project does not exist — will be skipped
[OK]  Local path ~/projects/my-repo exists (1,247 files)
[OK]  ripgrep v14.1.0 found

Ready! 4 sources configured, 3 healthy.
```

If any check fails, it tells you exactly what to fix:
```
[FAIL] Slack: token rejected — "not_allowed_token_type"
       Fix: You're using a bot token (xoxb-). Slack search requires a user token (xoxp-).
       Get one at: api.slack.com/apps → OAuth → User Token Scopes → search:read
```

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
| Binary size | ~7MB |
| Runtime RAM | ~20MB |
| Startup time | <50ms |
| Query latency | ~1-3s (slowest source) |

## License

MIT
