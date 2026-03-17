# Unified Search MCP Server — Design Spec

**Date**: 2026-03-17
**Status**: Draft
**Language**: Rust
**Repo**: `ganesh-tt/unified-search-mcp`
**Local path**: `~/IdeaProjects/unified-search-mcp/`

---

## 1. Problem

Developer knowledge is scattered across Slack, Confluence, JIRA, local codebases, and investigation docs. Finding "what did we decide about X?" requires manually searching 4–5 systems. No single tool queries all sources in parallel and returns merged, ranked results.

## 2. Solution

A lightweight Rust MCP server that exposes a `unified_search` tool to Claude Code (or any MCP client). On query, it fans out searches to all enabled sources in parallel via `tokio`, merges results with configurable ranking, and returns a single ranked list with source attribution and deep links.

## 3. Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | 5–20MB binary, 10–30MB RAM, <50ms startup. No Python/venv overhead. |
| Architecture | Plugin (trait-based adapters) | `SearchSource` trait — add sources without touching core. Independently testable. |
| Local text search | ripgrep subprocess (fallback: Rust regex) | Proven fast, respects `.gitignore`. No reinventing the wheel. |
| Local vector search | ONNX Runtime + hnsw_rs (Phase 2) | ~100MB total vs 2GB PyTorch. Lean embeddings. |
| MCP transport | stdio (JSON-RPC) | Standard MCP protocol. Works with Claude Code, Cursor, any MCP client. |
| Config | YAML with `${ENV_VAR}` interpolation | Secrets stay in env, config file is shareable. |
| Auth | Env vars (primary), macOS Keychain (optional future) | Simple, portable, no binary dependency on Keychain. |

## 4. MCP Tools

| Tool | Description | Parameters |
|------|-------------|------------|
| `unified_search` | Fan-out search across all enabled sources | `query: string`, `sources?: string[]` (filter), `max_results?: int` (default 20) |
| `search_source` | Search a single named source | `source: string`, `query: string`, `max_results?: int` |
| `index_local` | Trigger re-index of local vector store | `paths?: string[]` (default: all configured) |
| `list_sources` | Show enabled sources + health status | (none) |

## 5. Core Architecture

### 5.1 SearchSource Trait

```rust
#[async_trait]
pub trait SearchSource: Send + Sync {
    /// Unique identifier (e.g., "slack", "confluence")
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// Check connectivity and auth
    async fn health_check(&self) -> SourceHealth;

    /// Execute search — return results sorted by source-local relevance
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>>;
}
```

### 5.2 Data Models

```rust
pub struct SearchQuery {
    pub text: String,
    pub max_results: usize,
    pub filters: SearchFilters,
}

pub struct SearchFilters {
    pub sources: Option<Vec<String>>,     // restrict to specific sources
    pub after: Option<DateTime<Utc>>,     // time-bound
    pub before: Option<DateTime<Utc>>,
}

pub struct SearchResult {
    pub source: String,                    // "slack", "confluence", etc.
    pub title: String,                     // page title, message subject, file path
    pub snippet: String,                   // relevant excerpt with match context
    pub url: Option<String>,               // deep link
    pub timestamp: Option<DateTime<Utc>>,
    pub relevance: f32,                    // 0.0–1.0
    pub metadata: HashMap<String, String>, // source-specific (channel, space, author, etc.)
}

pub struct SourceHealth {
    pub source: String,
    pub status: HealthStatus,              // Healthy | Degraded | Unavailable
    pub message: Option<String>,           // "401 Unauthorized — check token"
    pub latency_ms: Option<u64>,
}

pub enum HealthStatus {
    Healthy,
    Degraded,    // works but slow / partial
    Unavailable, // auth failure, timeout, config missing
}

pub struct UnifiedSearchResponse {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,           // per-source errors/timeouts
    pub total_sources_queried: usize,
    pub query_time_ms: u64,
}
```

### 5.3 Orchestrator (core.rs)

```
unified_search("what is the broadcast threshold?")
    │
    ├─ tokio::spawn → slack.search(query)      ──→ Slack Web API (search.messages)
    ├─ tokio::spawn → confluence.search(query)  ──→ /wiki/rest/api/search?cql=...
    ├─ tokio::spawn → jira.search(query)        ──→ /rest/api/3/search?jql=text~"..."
    ├─ tokio::spawn → local_text.search(query)  ──→ rg --json "broadcast threshold" ~/projects/
    └─ tokio::spawn → local_vector.search(query)──→ ONNX embed → hnsw nearest
    │
    └─ join_all (with per-source timeout)
        │
        ├─ collect successes + warnings for failures
        ├─ normalize relevance scores (0.0–1.0 per source)
        ├─ apply source weights from config
        ├─ sort by weighted_relevance DESC, then timestamp DESC
        ├─ dedup (same URL or same snippet content hash)
        └─ truncate to max_results → return
```

### 5.4 Ranking Algorithm

1. Each source returns results with a source-local `relevance` score (0.0–1.0)
2. Multiply by source `weight` from config (default 1.0)
3. Final score: `weighted_relevance = relevance * source_weight`
4. Sort by `weighted_relevance` DESC
5. Tiebreak by `timestamp` DESC (more recent wins)
6. Dedup: take the first 200 characters of the snippet (or full snippet if shorter), normalize whitespace to single spaces, and compare for string equality. If two results share the same URL **or** the same normalized snippet prefix, keep the higher-scored one. No cryptographic hash needed.

## 6. Source Adapter Details

### 6.1 Slack (`slack.rs`)

- **API**: `GET https://slack.com/api/search.messages` (requires user token `xoxp-`, not bot token)
- **Auth**: `Authorization: Bearer ${SLACK_USER_TOKEN}`
- **Query**: Slack search syntax (supports `in:#channel`, `from:@user`, date ranges natively)
- **Pagination**: `page` + `count` query params (max 100 per page, we fetch 1 page)
- **Result mapping**: `messages.matches[]` → `SearchResult` (permalink as URL, channel as metadata)
- **Rate limit**: Tier 2 (20/min) — respect `Retry-After` header, surface error if hit
- **Health check**: `POST https://slack.com/api/auth.test` with user token
- **Time filter**: `after` → append `after:YYYY-MM-DD` to query string; `before` → `before:YYYY-MM-DD`

### 6.2 Confluence (`confluence.rs`)

- **API**: REST v1 `GET /wiki/rest/api/search?cql=siteSearch~"query"&limit=N` (v2 has no search endpoint)
- **Auth**: Basic Auth (`email:api_token` base64)
- **Query**: CQL — `siteSearch ~ "query"` or `text ~ "query" AND space IN (spaces)`
- **Query escaping**: Double quotes in user input must be escaped (`"` → `\"`) to prevent CQL injection
- **Result mapping**: `results[]` → `SearchResult` (page URL, space key as metadata, excerpt)
- **Gotcha**: Excerpt from API is HTML — strip tags to plaintext for snippet
- **Health check**: `GET /wiki/rest/api/space?limit=1` (v1 spaces endpoint)
- **Time filter**: `after` → `lastmodified >= "YYYY-MM-DD"` in CQL; `before` → `lastmodified <= "YYYY-MM-DD"`

### 6.3 JIRA (`jira.rs`)

- **API**: REST v3 `GET /rest/api/3/search?jql=text~"query"&maxResults=N`
- **Auth**: Basic Auth (same creds as Confluence)
- **Query**: JQL — `text ~ "query"` optionally scoped to `project IN (projects)`
- **Query escaping**: Double quotes in user input must be escaped for JQL
- **Result mapping**: `issues[]` → `SearchResult` (browse URL, project key + status as metadata)
- **Fields requested**: `summary,description,comment,status,updated,assignee` (keep payload small)
- **Health check**: `GET /rest/api/3/myself` (auth verification)
- **Time filter**: `after` → `updated >= "YYYY-MM-DD"` in JQL; `before` → `updated <= "YYYY-MM-DD"`

### 6.4 Local Text (`local_text.rs`)

- **Method**: Spawn `rg --json --max-count 5 --max-filesize 1M "query" path1 path2 ...`
- **Fallback**: If `rg` not found, use `grep` crate with `WalkDir` for Rust-native search
- **Include/exclude**: Pass `--glob` patterns from config
- **Result mapping**: JSON output → `SearchResult` (file path as title, matched line as snippet, `file://` URL)
- **Relevance**: Based on match count per file (more matches = higher relevance, normalized to 0.0–1.0)
- **Time filter**: `after`/`before` → filter by file modification time (`mtime`). Files outside range excluded from results.

### 6.5 Local Vector (`local_vector.rs`) — Phase 2

- **Embedding**: ONNX Runtime (`ort` crate) + `all-MiniLM-L6-v2` ONNX model (~80MB)
- **Index**: `hnsw_rs` crate — in-memory HNSW graph, persisted to `~/.unified-search/index/`
- **Indexing pipeline**: Discover files → extract text (PDF via `pdf-extract`, MD/TXT direct) → chunk (500 tokens, 100 overlap) → embed → insert into HNSW
- **Search**: Embed query → k-nearest-neighbors → map chunks back to files → `SearchResult`
- **Reindex trigger**: `index_local` MCP tool, or auto on file change (optional `notify` crate watcher)
- **Relevance**: Cosine similarity from HNSW (already 0.0–1.0)

## 7. Configuration

See `config.example.yaml` in the repo root. Key features:
- `${ENV_VAR}` interpolation for secrets
- Per-source `enabled`, `weight`, `max_results`
- Local paths with include/exclude glob patterns
- Vector index opt-in with model/index path config
- Global `timeout_seconds` and `max_results`

## 8. Error Handling

| Scenario | Behavior |
|----------|----------|
| Source timeout | Other sources still return. Warning in response: `"slack: timed out after 10s"` |
| Auth failure (401/403) | Source auto-disabled for session. Warning with actionable message. |
| Rate limited (429) | Surface to user with `Retry-After` value. No retry loop. |
| Missing config.yaml | Print setup instructions, point to `config.example.yaml`. Exit cleanly. |
| Invalid YAML | Error with line number. Exit. |
| Missing env var | Error at startup naming the var. Not at query time. |
| Missing ripgrep | Fall back to Rust-native search. Log warning suggesting install. |
| Configured path missing | Skip with warning. Don't crash. |
| Binary/huge file | Respect max file size (1MB default). Skip with debug log. |
| Transient errors (5xx, connection reset) | No automatic retry in Phase 1. Source returns error immediately. Future: configurable 1-retry with 1s delay. |
| All sources fail | Return empty results + all warnings. Never panic. |

## 9. Auth Requirements

### Slack
1. Create Slack App at `api.slack.com/apps` → From Scratch
2. **User Token Scopes** (required): `search:read` (the `search.messages` API requires a user token — `search:messages` is not a real scope)
3. **Bot Token Scopes** (optional, for future enrichment like channel name resolution): `channels:read`, `groups:read`, `users:read`
4. Install to workspace → copy `xoxp-` (user) token. Bot token (`xoxb-`) is optional for Phase 1.
5. Set `SLACK_USER_TOKEN` env var (required). `SLACK_BOT_TOKEN` optional.

### Atlassian (Confluence + JIRA)
1. Create API token at `id.atlassian.com/manage-profile/security/api-tokens`
2. Set `ATLASSIAN_EMAIL`, `ATLASSIAN_API_TOKEN`, `ATLASSIAN_BASE_URL`
3. Ensure read access to target spaces/projects

### Local
1. Configure paths in `config.yaml`
2. Install `ripgrep` (`brew install ripgrep` / `cargo install ripgrep`) — optional but recommended
3. For Phase 2 vector: download ONNX model to `~/.unified-search/models/`

## 10. Project Structure

```
unified-search-mcp/
├── Cargo.toml
├── config.example.yaml
├── config.yaml                  # gitignored
├── README.md
├── .gitignore
├── docs/
│   ├── DESIGN.md                # this spec (copied into repo)
│   ├── PHASE_PLAN.md            # phase-by-phase build order
│   └── TEST_PLAN.md             # test specifications per module
├── src/
│   ├── main.rs                  # entry point: load config → register tools → start stdio server
│   ├── config.rs                # YAML parsing, env var interpolation, validation
│   ├── models.rs                # SearchQuery, SearchResult, SourceHealth, etc.
│   ├── core.rs                  # Orchestrator: fan-out, merge, rank, dedup
│   ├── server.rs                # MCP tool registration and dispatch
│   └── sources/
│       ├── mod.rs               # SearchSource trait definition
│       ├── slack.rs
│       ├── confluence.rs
│       ├── jira.rs
│       ├── local_text.rs
│       └── local_vector.rs      # Phase 2
├── tests/
│   ├── common/
│   │   └── mod.rs               # Shared test helpers (mock server setup, fixture loading)
│   ├── test_config.rs
│   ├── test_models.rs
│   ├── test_core.rs
│   ├── test_slack.rs
│   ├── test_confluence.rs
│   ├── test_jira.rs
│   ├── test_local_text.rs
│   ├── test_local_vector.rs     # Phase 2
│   └── test_integration.rs      # End-to-end with all sources mocked
└── fixtures/
    ├── slack/
    │   ├── search_messages_success.json
    │   ├── search_messages_empty.json
    │   └── search_messages_rate_limited.json
    ├── confluence/
    │   ├── search_success.json
    │   ├── search_empty.json
    │   └── search_auth_failure.json
    ├── jira/
    │   ├── search_success.json
    │   ├── search_empty.json
    │   └── search_auth_failure.json
    ├── local/
    │   ├── sample_codebase/      # small test files for ripgrep
    │   └── sample_docs/          # md, txt, pdf for doc search
    └── config/
        ├── valid_full.yaml
        ├── valid_minimal.yaml
        ├── missing_env_var.yaml
        └── invalid_syntax.yaml
```

## 11. Development Workflow — TDD with 3-Agent Pattern

### Agent Roles

**Agent C (Orchestrator)**:
- Owns the phase plan
- Dispatches Agent A and Agent B per module
- Passes ONLY the trait signature + module scope to each
- Does NOT leak implementation details A→B or test internals B→A
- Validates: tests compile, implementation compiles, tests pass
- Advances phases

**Agent A (Test Writer)**:
- Receives: trait definition, module scope, fixture samples, test conventions
- Writes comprehensive test file with all cases (happy path, errors, edge cases)
- Tests must compile but fail (no implementation yet)
- Never sees Agent B's code

**Agent B (Implementer)**:
- Receives: trait definition, module scope, Agent A's test file (read-only)
- Writes minimal code to pass all tests
- **CANNOT modify tests** — if a test seems wrong, escalate to Agent C
- If tests are red after implementation, Agent C sends failing output back to B (max 2 retries)

### Phase Execution Order

```
Phase 0: Scaffold (single agent)
    └─ Cargo.toml, src stubs, CI config, .gitignore

Phase 1: Models (A→B sequential)
    └─ models.rs — structs, serde, display

Phase 2: Core + Trait (A→B sequential)
    └─ sources/mod.rs (trait), core.rs (orchestrator), ranking

Phase 3–6: Source Adapters (4x A→B in PARALLEL)
    ├─ Phase 3: local_text.rs
    ├─ Phase 4: confluence.rs
    ├─ Phase 5: jira.rs
    └─ Phase 6: slack.rs

Phase 7: MCP Server (A→B sequential, depends on 3–6)
    └─ server.rs, config.rs, main.rs

Phase 8: Local Vector (A→B, Phase 2 — independent)
    └─ local_vector.rs + ONNX integration

Phase 9: Integration Tests (A→B sequential, depends on all)
    └─ End-to-end with all sources mocked
```

### Agent C Sequence Per Module

1. Extract trait signature + module scope from phase plan
2. Dispatch Agent A (worktree) → receives trait + scope + fixtures
3. A returns test file path
4. C runs `cargo test --no-run` (or expects compilation with test failures)
5. C dispatches Agent B (same worktree) → receives trait + scope + A's test file (read-only)
6. B returns impl file path
7. C runs `cargo test` for that module
8. If green → merge to main branch, advance phase
9. If red → send failing test output to B (max 2 retries)
10. If B claims test is wrong → C reviews A's test logic independently. If test is genuinely wrong, C dispatches a corrected test request to A (not B), then re-runs B. B never modifies tests.
11. If still red after 2 B retries + 1 A review → escalate to human

## 12. Rust Dependencies (Cargo.toml)

```toml
[package]
name = "unified-search-mcp"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
description = "Unified search MCP server — search Slack, Confluence, JIRA, and local files from one tool"
license = "MIT"

[dependencies]
# MCP
rmcp = { version = "1", features = ["server", "transport-io"] }

# Async
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
futures = "0.3"

# HTTP
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yml = "0.0.12"         # serde_yaml is deprecated; this is the maintained successor

# Time
chrono = { version = "0.4", features = ["serde"] }

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Utils
thiserror = "2"
shellexpand = "3"          # ~/path expansion
regex = "1"

# Local text search fallback
grep-regex = "0.1"
grep-searcher = "0.1"
walkdir = "2"

# Phase 2: Vector search (optional)
# ort = { version = "2.0.0-rc.12", optional = true }  # v2 stable not released yet; pin RC
# hnsw_rs = { version = "0.3", optional = true }
# tokenizers = { version = "0.19", optional = true }

[dev-dependencies]
wiremock = "0.6"           # Mock HTTP servers for API tests
tokio-test = "0.4"
tempfile = "3"             # Temp dirs for local search tests
assert_matches = "1.5"
pretty_assertions = "1"

# [features]
# vector = ["ort", "hnsw_rs", "tokenizers"]
```

## 13. Resource Budget

| Metric | Phase 1 (no vector) | Phase 2 (with vector) |
|--------|---------------------|----------------------|
| Binary size | ~5–10MB | ~15–20MB |
| Runtime RAM | 10–30MB | ~80–120MB |
| Disk (runtime data) | 0 | ~100MB (model + index) |
| Startup time | <50ms | <200ms (model load) |
| Query latency | Slowest source (~1–3s) | Same + ~50ms embedding |

## 14. Patterns Adapted From Prior Art

Lessons learned from researching the MCP ecosystem (sooperset/mcp-atlassian, aashari, nguyenvanduocit, Atlassian official):

### 14.1 HTML→Markdown Preprocessing (Confluence adapter)

Confluence returns XHTML storage format. Raw HTML wastes LLM tokens. Our Confluence adapter must:
1. Strip HTML tags from excerpts in search results
2. For full page content (if we ever add a `get_page` tool): resolve `@accountId` mentions → display names, protect code blocks with placeholders during conversion, convert to Markdown

For search snippets (Phase 1), simple tag stripping suffices. Full Markdown conversion is a future extension.

### 14.2 Scoped Query Injection (Defense-in-depth)

Always inject space/project filters at the query construction layer inside the adapter, not via LLM instructions. Even if the LLM ignores config, the filter is always applied:
```rust
// In confluence.rs
fn build_cql(&self, query: &str) -> String {
    let base = format!("siteSearch ~ \"{}\"", query);
    if self.config.spaces.is_empty() {
        base
    } else {
        let spaces = self.config.spaces.iter()
            .map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(",");
        format!("({}) AND space IN ({})", base, spaces)
    }
}
```

### 14.3 Token-Compact Result Format

Return `unified_search` results as a **Markdown table** by default (30-60% fewer tokens than JSON):
```markdown
| # | Source | Title | Snippet | URL |
|---|--------|-------|---------|-----|
| 1 | confluence | Broadcast Threshold Design | ...threshold was set to 50K rows... | https://... |
| 2 | slack | #engineering | ganesh: we decided 50K after the OOM... | https://... |
| 3 | jira | FIN-10384 | Remove broadcastRowThreshold callers | https://... |

**Warnings**: slack: rate limited (retry after 30s)
**Sources queried**: 4 | **Time**: 1.2s
```

Individual `search_source` results can return richer per-item detail.

### 14.4 Response Truncation with File Save

When local text search returns >50 results or >40K chars:
1. Save full results to `~/.unified-search/last-search-results.json`
2. Return top 20 in the MCP response
3. Include note: "50 total results. Full results saved to ~/.unified-search/last-search-results.json"
4. The LLM can read the file if needed

### 14.5 Descriptive Errors Always (Never Mask)

Every error returned to the LLM must be actionable:
- "confluence: 401 Unauthorized — check ATLASSIAN_EMAIL and ATLASSIAN_API_TOKEN env vars"
- "slack: search requires user token (xoxp-), not bot token (xoxb-) — check SLACK_USER_TOKEN"
- "local_text: path /Users/x/old-repo does not exist — update config.yaml paths"

Never return generic "Internal error" or "Request failed".

### 14.6 Why All-in-One Works Here (Lessons from nguyenvanduocit)

nguyenvanduocit's all-in-one MCP (99 stars) was abandoned because:
- 16+ env vars for unrelated services (users wanting Jira had to ignore 14 YouTube/RAG vars)
- 40+ tools polluted the LLM's tool selection (accuracy degrades with count)
- Every Jira fix forced re-release of YouTube and RAG tools

Our unified search avoids these traps:
- **4 tools, not 40** — one purpose (search), not a Swiss army knife
- **Sources are config, not tools** — adding Slack doesn't add a tool, it adds a config block
- **Env vars scale with what you use** — only Slack vars needed if only Slack is enabled
- **Independent release cadence is unnecessary** — all adapters serve one tool

## 15. Future Extensions (Not In Scope)

- Google Drive source adapter
- GitHub Issues/Discussions adapter
- Slack Socket Mode (real-time indexing)
- macOS Keychain auth (instead of env vars)
- Web UI dashboard
- Result caching layer
- Multi-user / team server mode
