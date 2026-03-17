# Unified Search MCP Server — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking. This plan uses a **3-Agent TDD Pattern** — see Task 0 for the protocol.

**Goal:** Build a Rust MCP server that searches Slack, Confluence, JIRA, and local files in parallel from a single `unified_search` tool.

**Architecture:** Plugin-based trait adapters (`SearchSource`) behind a `tokio::join_all` orchestrator. Stdio JSON-RPC transport via `rmcp`. YAML config with env var interpolation. Phase 2 adds optional vector search via ONNX Runtime.

**Tech Stack:** Rust 2021, rmcp 1.x, tokio, reqwest, serde, serde_yml, wiremock, ripgrep (subprocess + grep-* fallback), chrono, tracing

**Spec:** `~/IdeaProjects/unified-search-mcp/docs/DESIGN.md`
**Phase Plan:** `~/IdeaProjects/unified-search-mcp/docs/PHASE_PLAN.md`
**Test Plan:** `~/IdeaProjects/unified-search-mcp/docs/TEST_PLAN.md`

**Repo:** `~/IdeaProjects/unified-search-mcp/` → push to `ganesh-tt/unified-search-mcp`

---

## Task 0: 3-Agent TDD Protocol (READ THIS FIRST)

Every task from Task 2 onward follows this protocol. **Agent C** (the main orchestrator session) never writes application or test code directly — it dispatches subagents.

### Roles

| Agent | What it does | What it receives | What it NEVER sees |
|-------|-------------|------------------|--------------------|
| **C** (you) | Dispatches A & B, validates, advances | Phase plan, all file paths | N/A |
| **A** (subagent) | Writes test file | Trait signature, module scope doc, fixture samples, test conventions | B's implementation |
| **B** (subagent) | Writes impl to pass tests | Trait signature, module scope doc, A's test file (read-only) | A's reasoning |

### Sequence (per task)

```
C: Extract contract (trait + scope) from this plan
C: Dispatch Agent A → "Write tests for {module} per this contract: {contract}. Test file: {path}. Fixtures at: {fixtures_path}. Tests must compile but may fail. Use wiremock for HTTP mocks, tempfile for filesystem."
C: Wait for A → verify: cargo test --no-run (or expect compile errors only in the impl, not the tests)
C: Dispatch Agent B → "Implement {module} to pass all tests in {test_file}. Contract: {contract}. Impl file: {path}. You CANNOT modify any test file. If a test seems wrong, respond with the issue instead of changing it."
C: Wait for B → run: cargo test {test_filter}
C: If green → commit, advance
C: If red → send failure output to B (max 2 retries). If B says test is wrong → C reviews test, dispatches fix to A if justified, then re-runs B.
C: If still red after 2 B retries + 1 A review → stop, escalate to human
```

### B Cannot Modify Tests — Enforcement

When dispatching Agent B, include this line: **"RULE: You MUST NOT modify any file in `tests/` or `fixtures/`. If a test seems wrong, respond with the issue description instead of changing it. Violation = task failure."**

### Agent A Dispatch Payload

Always include in Agent A's dispatch:
1. The contract block from this plan
2. The relevant section of `docs/TEST_PLAN.md` (copy the test specs, don't just reference)
3. Fixture file paths and their contents
4. Test conventions: `use wiremock` for HTTP, `use tempfile` for filesystem, `use pretty_assertions` for diffs

### Compilation Chicken-and-Egg

Task 1 creates **compilable type stubs** (empty structs/traits with correct signatures) so Agent A's tests can import types. Agent A writes tests against stubs. Agent B replaces stubs with real implementations. This means Agent A's tests compile from the start — `cargo test --no-run` must pass after Agent A.

### Parallel Task Merge Strategy

For Tasks 4–7 (parallel source adapters):
1. Each runs in its own git branch off `main` (after Task 3 merge)
2. Each adapter only creates/modifies its own `src/sources/{name}.rs` and `tests/test_{name}.rs`
3. After all 4 complete, Agent C merges each branch into `main` sequentially: `git merge --no-ff branch-4`, then 5, then 6, then 7
4. No conflicts expected since files don't overlap. If conflict occurs, resolve and continue.

---

## Task 1: Project Scaffold

**Agent:** C does this directly (no A/B split needed — no tests to write for scaffolding)
**Worktree:** `~/IdeaProjects/unified-search-mcp/`

### Files to create:

- [ ] **Step 1: Initialize git repo**

```bash
cd ~/IdeaProjects/unified-search-mcp
git init
```

- [ ] **Step 2: Create Cargo.toml**

Create: `Cargo.toml`

```toml
[package]
name = "unified-search-mcp"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
description = "Unified search MCP server — search Slack, Confluence, JIRA, and local files from one tool"
license = "MIT"

[dependencies]
rmcp = { version = "1", features = ["server", "transport-io"] }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
futures = "0.3"
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yml = "0.0.12"
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2"
shellexpand = "3"
regex = "1"
grep-regex = "0.1"
grep-searcher = "0.1"
walkdir = "2"

[dev-dependencies]
wiremock = "0.6"
tokio-test = "0.4"
tempfile = "3"
assert_matches = "1.5"
pretty_assertions = "1"
```

- [ ] **Step 3: Create source stubs**

Create all source files with minimal content so `cargo check` passes:

`src/main.rs`:
```rust
fn main() {
    println!("unified-search-mcp v0.1.0");
}
```

`src/lib.rs` (**REQUIRED** — integration tests import types via this):
```rust
pub mod models;
pub mod config;
pub mod core;
pub mod server;
pub mod sources;
```

`src/models.rs` (compilable stubs — Agent B replaces these):
```rust
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub max_results: usize,
    pub filters: SearchFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchFilters {
    pub sources: Option<Vec<String>>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub source: String,
    pub title: String,
    pub snippet: String,
    pub url: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub relevance: f32,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceHealth {
    pub source: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResponse {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    pub total_sources_queried: usize,
    pub query_time_ms: u64,
}

/// Error type used across all adapters and the orchestrator.
#[derive(thiserror::Error, Debug)]
pub enum SearchError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{source}: authentication failed — {message}")]
    Auth { source: String, message: String },
    #[error("{source}: rate limited — retry after {retry_after_secs}s")]
    RateLimited { source: String, retry_after_secs: u64 },
    #[error("{source}: {message}")]
    Source { source: String, message: String },
    #[error("Config error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self { text: String::new(), max_results: 20, filters: SearchFilters::default() }
    }
}
```

`src/config.rs`:
```rust
// YAML config loading with env var interpolation — Agent B implements
```

`src/core.rs`:
```rust
// SearchOrchestrator — fan-out, merge, rank, dedup — Agent B implements
```

`src/server.rs`:
```rust
// MCP tool registration and stdio transport — Agent B implements
```

`src/sources/mod.rs` (trait stub — compilable):
```rust
use async_trait::async_trait;
use crate::models::*;

pub mod slack;
pub mod confluence;
pub mod jira;
pub mod local_text;
// pub mod local_vector; // Phase 2 — uncomment when implementing

#[async_trait]
pub trait SearchSource: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn health_check(&self) -> SourceHealth;
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
}
```

`src/sources/slack.rs`, `src/sources/confluence.rs`, `src/sources/jira.rs`, `src/sources/local_text.rs`: empty files.

**Note:** `local_vector.rs` is NOT created in Phase 1 (the `pub mod` is commented out in `mod.rs`). It will be added in Task 11 (Phase 2).

`tests/common/mod.rs`:
```rust
// Shared test helpers
```

- [ ] **Step 4: Create .gitignore**

```
/target
config.yaml
*.swp
.DS_Store
```

- [ ] **Step 5: Create config.example.yaml**

```yaml
server:
  name: "unified-search"
  max_results: 20
  timeout_seconds: 10
  log_level: "info"

sources:
  slack:
    enabled: true
    user_token: "${SLACK_USER_TOKEN}"
    weight: 1.0
    max_results: 10

  confluence:
    enabled: true
    base_url: "${ATLASSIAN_BASE_URL}"
    email: "${ATLASSIAN_EMAIL}"
    api_token: "${ATLASSIAN_API_TOKEN}"
    spaces: []
    weight: 1.0
    max_results: 10

  jira:
    enabled: true
    base_url: "${ATLASSIAN_BASE_URL}"
    email: "${ATLASSIAN_EMAIL}"
    api_token: "${ATLASSIAN_API_TOKEN}"
    projects: []
    weight: 1.0
    max_results: 10

  local_text:
    enabled: true
    paths:
      - "~/projects/my-repo"
    include_patterns:
      - "**/*.{rs,py,scala,java,js,ts,go,sql,sh,toml,yaml,yml}"
    exclude_patterns:
      - "**/target/**"
      - "**/node_modules/**"
      - "**/.git/**"
    weight: 0.8
    max_results: 10

  # local_vector: (Phase 2 — uncomment when vector search is implemented)
  #   enabled: false
  #   paths: ["~/documents/notes"]
  #   include_patterns: ["**/*.{md,txt,pdf,rst,adoc}"]
  #   model_path: "~/.unified-search/models/all-MiniLM-L6-v2.onnx"
  #   index_path: "~/.unified-search/index"
  #   chunk_size: 500
  #   chunk_overlap: 100
```

- [ ] **Step 6: Create fixture files**

Create fixture JSON files at these paths. Content must match real API responses.

`fixtures/slack/search_messages_success.json`:
```json
{
  "ok": true,
  "query": "broadcast threshold",
  "messages": {
    "total": 3,
    "matches": [
      {
        "type": "message",
        "ts": "1710700800.123456",
        "text": "we decided 50K rows as the broadcast threshold after the OOM incident",
        "permalink": "https://workspace.slack.com/archives/C01ABC/p1710700800123456",
        "channel": {"id": "C01ABC", "name": "engineering"},
        "username": "ganesh",
        "score": 0.95
      },
      {
        "type": "message",
        "ts": "1710614400.654321",
        "text": "the broadcast threshold needs to be configurable per tenant",
        "permalink": "https://workspace.slack.com/archives/C01ABC/p1710614400654321",
        "channel": {"id": "C01ABC", "name": "engineering"},
        "username": "priya",
        "score": 0.82
      },
      {
        "type": "message",
        "ts": "1710528000.111111",
        "text": "broadcast threshold PR merged, default is 50000",
        "permalink": "https://workspace.slack.com/archives/C02DEF/p1710528000111111",
        "channel": {"id": "C02DEF", "name": "deployments"},
        "username": "ganesh",
        "score": 0.71
      }
    ]
  }
}
```

`fixtures/slack/search_messages_empty.json`:
```json
{"ok": true, "query": "xyznonexistent", "messages": {"total": 0, "matches": []}}
```

`fixtures/slack/search_messages_rate_limited.json`:
```json
{"ok": false, "error": "ratelimited"}
```

`fixtures/confluence/search_success.json`:
```json
{
  "results": [
    {
      "content": {"id": "12345", "type": "page", "title": "Broadcast Threshold Design"},
      "excerpt": "The <b>broadcast threshold</b> was set to <em>50K rows</em> after analysis of memory usage patterns.",
      "url": "/wiki/spaces/DEV/pages/12345",
      "lastModified": "2026-03-10T10:00:00.000Z",
      "resultGlobalContainer": {"title": "DEV", "displayUrl": "/wiki/spaces/DEV"}
    },
    {
      "content": {"id": "12346", "type": "page", "title": "Spark Configuration Guide"},
      "excerpt": "Configure the <b>broadcast</b> size <b>threshold</b> in the HOCON config under screening.broadcast.",
      "url": "/wiki/spaces/DEV/pages/12346",
      "lastModified": "2026-02-15T08:00:00.000Z",
      "resultGlobalContainer": {"title": "DEV", "displayUrl": "/wiki/spaces/DEV"}
    },
    {
      "content": {"id": "12347", "type": "page", "title": "OOM Post-Mortem Feb 2026"},
      "excerpt": "Root cause: unconditional <b>broadcast()</b> on watchlist DataFrame exceeding <b>threshold</b>.",
      "url": "/wiki/spaces/OPS/pages/12347",
      "lastModified": "2026-02-20T14:00:00.000Z",
      "resultGlobalContainer": {"title": "OPS", "displayUrl": "/wiki/spaces/OPS"}
    }
  ],
  "start": 0,
  "limit": 10,
  "size": 3,
  "totalSize": 3
}
```

`fixtures/confluence/search_empty.json`:
```json
{"results": [], "start": 0, "limit": 10, "size": 0, "totalSize": 0}
```

`fixtures/confluence/search_auth_failure.json`:
```json
{"statusCode": 401, "message": "Client must be authenticated to access this resource."}
```

`fixtures/jira/search_success.json`:
```json
{
  "startAt": 0,
  "maxResults": 10,
  "total": 3,
  "issues": [
    {
      "key": "FIN-10384",
      "self": "https://org.atlassian.net/rest/api/3/issue/10384",
      "fields": {
        "summary": "Remove broadcastRowThreshold callers and constant",
        "description": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Remove the broadcastRowThreshold configuration, SQL entries, and constant since we now use auto-broadcast with merge hints instead."}]}]},
        "status": {"name": "In Progress"},
        "updated": "2026-03-15T12:00:00.000+0000",
        "assignee": {"displayName": "Ganesh K"}
      }
    },
    {
      "key": "FIN-10385",
      "self": "https://org.atlassian.net/rest/api/3/issue/10385",
      "fields": {
        "summary": "Add WatchlistBroadcastRowThreshold seed SQL",
        "description": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Add the threshold configuration to dml.sql and the v6.3.4 migration scripts."}]}]},
        "status": {"name": "Done"},
        "updated": "2026-03-12T09:00:00.000+0000",
        "assignee": {"displayName": "Ganesh K"}
      }
    },
    {
      "key": "FIN-10071",
      "self": "https://org.atlassian.net/rest/api/3/issue/10071",
      "fields": {
        "summary": "Local E2E setup for screening pipeline",
        "description": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Set up local end-to-end testing environment for the screening pipeline with all services running in Docker."}]}]},
        "status": {"name": "To Do"},
        "updated": "2026-03-01T08:00:00.000+0000",
        "assignee": {"displayName": "Priya S"}
      }
    }
  ]
}
```

`fixtures/jira/search_empty.json`:
```json
{"startAt": 0, "maxResults": 10, "total": 0, "issues": []}
```

`fixtures/jira/search_auth_failure.json`:
```json
{"errorMessages": ["Client must be authenticated to access this resource."], "errors": {}}
```

`fixtures/local/sample_codebase/main.rs`:
```rust
use tokio;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub relevance: f32,
}

#[tokio::main]
async fn main() {
    println!("async main running");
}
```

`fixtures/local/sample_codebase/config.yaml`:
```yaml
server:
  timeout: 10
sources:
  slack:
    enabled: true
```

`fixtures/local/sample_codebase/target/debug/build.rs`:
```rust
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
```

`fixtures/local/sample_docs/design.md`:
```markdown
# Broadcast Threshold Design

The broadcast threshold was set to 50K rows after analysis.

## Ranking Algorithm

Results are ranked by weighted relevance, then timestamp.
```

`fixtures/local/sample_docs/notes.txt`:
```
JIRA ticket FIN-10384 tracks the broadcast threshold removal.
The fix was deployed in v6.3.4.
```

For `fixtures/local/sample_codebase/large_file.bin`: generate a 2MB file:
```bash
dd if=/dev/zero of=fixtures/local/sample_codebase/large_file.bin bs=1024 count=2048
```

Config fixtures — `fixtures/config/valid_full.yaml`:
```yaml
server:
  name: "test-server"
  max_results: 20
  timeout_seconds: 10
  log_level: "info"
sources:
  slack:
    enabled: true
    user_token: "${TEST_SLACK_TOKEN}"
    weight: 1.0
    max_results: 10
  confluence:
    enabled: true
    base_url: "https://test.atlassian.net"
    email: "test@example.com"
    api_token: "${TEST_ATLASSIAN_TOKEN}"
    spaces: ["DEV"]
    weight: 1.0
    max_results: 10
  jira:
    enabled: true
    base_url: "https://test.atlassian.net"
    email: "test@example.com"
    api_token: "${TEST_ATLASSIAN_TOKEN}"
    projects: ["FIN"]
    weight: 1.0
    max_results: 10
  local_text:
    enabled: true
    paths: ["~/projects/test-repo"]
    include_patterns: ["**/*.rs"]
    exclude_patterns: ["**/target/**"]
    weight: 0.8
    max_results: 10
```

`fixtures/config/valid_minimal.yaml`:
```yaml
sources:
  local_text:
    enabled: true
    paths: ["/tmp/test"]
```

`fixtures/config/missing_env_var.yaml`:
```yaml
sources:
  slack:
    enabled: true
    user_token: "${NONEXISTENT_VAR_12345}"
```

`fixtures/config/invalid_syntax.yaml`:
```yaml
sources:
  slack
    enabled: true
```

- [ ] **Step 7: Verify scaffold compiles**

Run: `cd ~/IdeaProjects/unified-search-mcp && cargo check`
Expected: compiles with no errors (warnings OK)

- [ ] **Step 8: Commit scaffold**

```bash
git add -A
git commit -m "feat: project scaffold with Cargo.toml, source stubs, fixtures, and config template"
```

---

## Task 2: Models (Phase 1)

**TDD Pattern:** Agent A → Agent B
**Note:** Task 1 created compilable type stubs in `src/models.rs`. Agent A writes tests against these stubs. Agent B replaces the stubs with full implementations (adding Ord, Display, etc.).

**Contract for both agents:**

```rust
// Models in src/models.rs (stubs already exist — Agent B completes them):
// - SearchQuery { text: String, max_results: usize, filters: SearchFilters }
// - SearchFilters { sources: Option<Vec<String>>, after: Option<DateTime<Utc>>, before: Option<DateTime<Utc>> }
// - SearchResult { source: String, title: String, snippet: String, url: Option<String>, timestamp: Option<DateTime<Utc>>, relevance: f32, metadata: HashMap<String, String> }
// - SourceHealth { source: String, status: HealthStatus, message: Option<String>, latency_ms: Option<u64> }
// - HealthStatus enum { Healthy, Degraded, Unavailable }
// - UnifiedSearchResponse { results: Vec<SearchResult>, warnings: Vec<String>, total_sources_queried: usize, query_time_ms: u64 }
// - SearchError enum { Http, Auth, RateLimited, Source, Config, Other } (via thiserror)
//
// ALL must: derive Serialize, Deserialize, Debug, Clone
// SearchResult must: impl PartialOrd + Ord (by relevance DESC, then timestamp DESC — None timestamps sort last)
// SearchQuery must: impl Default (max_results=20, filters empty)
// HealthStatus must: impl Display ("healthy", "degraded", "unavailable")
// SearchError: already stubbed with thiserror derives. Agent B may add variants as needed.
```

**Files:**
- Test: `tests/test_models.rs`
- Impl: `src/models.rs`

**Test spec reference:** `docs/TEST_PLAN.md` → "models.rs" section (11 test cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_models.rs`
- [ ] **Step 2:** C verifies: `cargo test --test test_models --no-run` (compiles, or fails only on missing impl)
- [ ] **Step 3:** C dispatches Agent B to implement `src/models.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_models`
  Expected: all 11 tests pass
- [ ] **Step 5:** Commit

```bash
git add src/models.rs tests/test_models.rs
git commit -m "feat: add data models with serde, ordering, and display"
```

---

## Task 3: Core Orchestrator + SearchSource Trait (Phase 2)

**TDD Pattern:** Agent A → Agent B
**Contract:**

```rust
// In src/sources/mod.rs — SearchSource trait:
//   fn name(&self) -> &str
//   fn description(&self) -> &str
//   async fn health_check(&self) -> SourceHealth
//   async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>>
//
// In src/core.rs — SearchOrchestrator:
//   pub struct SearchOrchestrator { sources: Vec<Box<dyn SearchSource>>, config: OrchestratorConfig }
//   pub struct OrchestratorConfig { timeout_seconds: u64, source_weights: HashMap<String, f32>, max_results: usize }
//   pub async fn search(&self, query: &SearchQuery) -> UnifiedSearchResponse
//   pub async fn health_check_all(&self) -> Vec<SourceHealth>
//
// Orchestrator behavior:
//   - Fan out to all sources via tokio::spawn with per-source timeout
//   - Collect successes, capture errors/timeouts as warnings
//   - Ranking: relevance * source_weight DESC, then timestamp DESC
//   - Dedup: same URL or same normalized snippet prefix (first 200 chars, whitespace-collapsed)
//   - Truncate to max_results
//   - sources filter in query: if set, only query named sources
//
// Agent A must create mock sources in the test file:
//   MockSource (configurable results, delay, error)
//   PanicSource (panics on search — must not crash orchestrator; use tokio::spawn to catch)
```

**Files:**
- Test: `tests/test_core.rs`
- Impl: `src/sources/mod.rs`, `src/core.rs`

**Test spec reference:** `docs/TEST_PLAN.md` → "core.rs" section (13 test cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_core.rs`
- [ ] **Step 2:** C verifies tests compile (mock sources defined in test file, impl stubs exist)
- [ ] **Step 3:** C dispatches Agent B to implement `src/sources/mod.rs` and `src/core.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_core`
  Expected: all 13 tests pass
- [ ] **Step 5:** Commit

```bash
git add src/sources/mod.rs src/core.rs tests/test_core.rs
git commit -m "feat: add SearchSource trait and SearchOrchestrator with fan-out, ranking, dedup"
```

---

## Tasks 4–7: Source Adapters (Phases 3–6) — RUN IN PARALLEL

These four tasks are **fully independent** and MUST be dispatched as parallel A→B pipelines. Each runs in its own worktree branch.

### Task 4: Local Text Search (Phase 3)

**TDD Pattern:** Agent A → Agent B
**Contract:**

```rust
// LocalTextSource implements SearchSource
// Config: paths: Vec<PathBuf>, include_patterns: Vec<String>, exclude_patterns: Vec<String>, max_file_size_bytes: u64
// Search: spawn `rg --json --max-count 5 --max-filesize 1M "{query}" {paths}` with --glob for patterns
// Fallback: if rg not in PATH, use grep-regex + walkdir crates
// Relevance: match_count / max_match_count (normalized 0.0–1.0)
// Time filter: after/before filter by file mtime
// URL: file:// scheme from absolute path
// Escape regex special chars in query before passing to rg
// Multiple matches in same file → single SearchResult
```

**Files:**
- Test: `tests/test_local_text.rs`
- Impl: `src/sources/local_text.rs`
- Fixtures: `fixtures/local/sample_codebase/`, `fixtures/local/sample_docs/`

**Test spec reference:** `docs/TEST_PLAN.md` → "local_text.rs" section (12 test cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_local_text.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to implement `src/sources/local_text.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_local_text`
- [ ] **Step 5:** Commit

```bash
git add src/sources/local_text.rs tests/test_local_text.rs
git commit -m "feat: add local text search adapter with ripgrep + fallback"
```

### Task 5: Confluence Search (Phase 4)

**TDD Pattern:** Agent A → Agent B
**Contract:**

```rust
// ConfluenceSource implements SearchSource
// Config: base_url: String, email: String, api_token: String, spaces: Vec<String>, max_results: usize
// API: GET {base_url}/wiki/rest/api/search?cql={cql}&limit={max_results}
//   (v1 API — v2 has no search endpoint)
// Auth: Basic Auth (base64 of "email:api_token")
// CQL: siteSearch ~ "{escaped_query}" optionally AND space IN ("S1","S2")
// Query escaping: " → \" in user input to prevent CQL injection
// Time filter: after → lastmodified >= "YYYY-MM-DD", before → lastmodified <= "YYYY-MM-DD"
// Excerpt: strip HTML tags for snippet
// Health check: GET {base_url}/wiki/rest/api/space?limit=1
// Relevance: position-based (first=1.0, last=lower)
```

**Files:**
- Test: `tests/test_confluence.rs`
- Impl: `src/sources/confluence.rs`
- Fixtures: `fixtures/confluence/`

**Test spec reference:** `docs/TEST_PLAN.md` → "confluence.rs" section (16 test cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_confluence.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to implement `src/sources/confluence.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_confluence`
- [ ] **Step 5:** Commit

```bash
git add src/sources/confluence.rs tests/test_confluence.rs
git commit -m "feat: add Confluence search adapter with CQL, HTML stripping, space filtering"
```

### Task 6: JIRA Search (Phase 5)

**TDD Pattern:** Agent A → Agent B
**Contract:**

```rust
// JiraSource implements SearchSource
// Config: base_url: String, email: String, api_token: String, projects: Vec<String>, max_results: usize
// API: GET {base_url}/rest/api/3/search?jql={jql}&maxResults={max_results}&fields=summary,description,comment,status,updated,assignee
// Auth: Basic Auth (base64 of "email:api_token")
// JQL: text ~ "{escaped_query}" optionally AND project IN ("P1","P2")
// Query escaping: " → \" in user input
// Time filter: after → updated >= "YYYY-MM-DD", before → updated <= "YYYY-MM-DD"
// Description: extract text from ADF (Atlassian Doc Format), truncate to 300 chars for snippet
// URL: {base_url}/browse/{issue_key}
// Health check: GET {base_url}/rest/api/3/myself
// Metadata: project key, status name, assignee display name
// Relevance: position-based
```

**Files:**
- Test: `tests/test_jira.rs`
- Impl: `src/sources/jira.rs`
- Fixtures: `fixtures/jira/`

**Test spec reference:** `docs/TEST_PLAN.md` → "jira.rs" section (17 test cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_jira.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to implement `src/sources/jira.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_jira`
- [ ] **Step 5:** Commit

```bash
git add src/sources/jira.rs tests/test_jira.rs
git commit -m "feat: add JIRA search adapter with JQL, ADF parsing, project filtering"
```

### Task 7: Slack Search (Phase 6)

**TDD Pattern:** Agent A → Agent B
**Contract:**

```rust
// SlackSource implements SearchSource
// Config: user_token: String (xoxp-), max_results: usize
// API: GET https://slack.com/api/search.messages?query={query}&count={max_results}
//   (NOT POST — Slack search.messages is GET with query params)
// Auth: Authorization: Bearer {user_token}
// Time filter: append "after:YYYY-MM-DD" / "before:YYYY-MM-DD" to query string
// Response: check "ok" field first. If false, return error with "error" field value.
// Special case: error="not_allowed_token_type" → hint about xoxp- vs xoxb-
// Result mapping: messages.matches[] → SearchResult
//   snippet = text, url = permalink, metadata = {channel: name, user: username}
// Relevance: Slack's score field normalized to 0.0–1.0
// Timestamp: parse ts field "1710700800.123456" as Unix epoch
// Health check: POST https://slack.com/api/auth.test (with user token)
// Rate limit: 429 → surface with Retry-After value
```

**Files:**
- Test: `tests/test_slack.rs`
- Impl: `src/sources/slack.rs`
- Fixtures: `fixtures/slack/`

**Test spec reference:** `docs/TEST_PLAN.md` → "slack.rs" section (12 test cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_slack.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to implement `src/sources/slack.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_slack`
- [ ] **Step 5:** Commit

```bash
git add src/sources/slack.rs tests/test_slack.rs
git commit -m "feat: add Slack search adapter with user token auth and score normalization"
```

---

## Task 8a: Config Loading (Phase 7, Part 1)

**Depends on:** Tasks 4–7 complete (all adapters must exist)
**TDD Pattern:** Agent A → Agent B

**Contract:**

```rust
// Config (src/config.rs):
//   pub fn load(path: Option<&str>) -> Result<AppConfig, SearchError>
//   - Read YAML file (default: ./config.yaml)
//   - Interpolate ${ENV_VAR} patterns → read from std::env
//   - Missing env var → SearchError::Config naming the variable
//   - Missing file → SearchError::Config pointing to config.example.yaml
//   - Invalid YAML → SearchError::Config with context
//   - Tilde expand paths: ~/foo → /Users/{user}/foo (use shellexpand crate)
//   - Apply defaults: max_results=20, timeout_seconds=10, log_level="info"
//   - Return AppConfig with typed source configs + server config
//
// AppConfig struct:
//   server: ServerConfig { name: String, max_results: usize, timeout_seconds: u64, log_level: String }
//   sources: SourcesConfig { slack: Option<SlackConfig>, confluence: Option<ConfluenceConfig>,
//            jira: Option<JiraConfig>, local_text: Option<LocalTextConfig> }
//   Each source config has `enabled: bool` — disabled sources not instantiated
//
// pub fn build_sources(config: &AppConfig) -> Vec<Box<dyn SearchSource>>
//   Instantiate only enabled sources. Skip disabled ones silently.
```

**Files:**
- Test: `tests/test_config.rs`
- Impl: `src/config.rs`
- Fixtures: `fixtures/config/`

**Test spec reference:** `docs/TEST_PLAN.md` → "config.rs" section (9 cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_config.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to implement `src/config.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_config`
  Expected: all 9 tests pass
- [ ] **Step 5:** Commit

```bash
git add src/config.rs tests/test_config.rs
git commit -m "feat: add YAML config loader with env var interpolation and validation"
```

---

## Task 8b: MCP Server Wiring (Phase 7, Part 2)

**Depends on:** Task 8a complete
**TDD Pattern:** Agent A → Agent B

**Contract:**

```rust
// Server (src/server.rs):
//   Register 4 MCP tools via rmcp: unified_search, search_source, index_local, list_sources
//   Wire tools to SearchOrchestrator
//   Handle stdio transport via rmcp::transport::io::stdio
//
// unified_search output format — Markdown table:
//   | # | Source | Title | Snippet | URL |
//   |---|--------|-------|---------|-----|
//   | 1 | confluence | Page Title | ...excerpt... | https://... |
//   | 2 | slack | #channel | user: message text... | https://... |
//   Footer: **Warnings**: {warnings} \n **Sources queried**: {count} | **Time**: {ms}ms
//
// search_source output format — JSON array with full detail:
//   Each result as full SearchResult JSON (not truncated for table)
//
// list_sources output — Markdown list:
//   - slack: healthy (42ms)
//   - confluence: unavailable — 401 Unauthorized
//
// index_local — Phase 1 behavior:
//   Return "Vector search not enabled. Set vector_index.enabled=true in config.yaml"
//
// Response truncation (>50 results or >40K chars):
//   1. Save full results to ~/.unified-search/last-search-results.json
//   2. Return top 20 in the response
//   3. Append note: "N total results. Full results saved to ~/.unified-search/last-search-results.json"
//
// main.rs:
//   Load config → build sources → build orchestrator → start MCP server on stdio
```

**Files:**
- Test: `tests/test_server.rs`
- Impl: `src/server.rs`, `src/main.rs`

**Test spec reference:** `docs/TEST_PLAN.md` → "server.rs" section (6 cases)

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_server.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to implement `src/server.rs` and `src/main.rs`
- [ ] **Step 4:** C runs: `cargo test --test test_server`
  Expected: all 6 tests pass
- [ ] **Step 5:** Verify binary builds: `cargo build --release`
- [ ] **Step 6:** Commit

```bash
git add src/server.rs src/main.rs tests/test_server.rs
git commit -m "feat: add MCP server with 4 tools, Markdown table output, and response truncation"
```

---

## Task 9: Integration Tests (Phase 9)

**Depends on:** Task 8b complete
**TDD Pattern:** Agent A → Agent B

**Contract:**

```rust
// End-to-end tests with ALL sources mocked via wiremock + tempfile
// Tests the full pipeline: config → sources → orchestrator → formatted output
// See docs/TEST_PLAN.md → "test_integration.rs" section (11 test cases)
//
// Key tests:
// - Full pipeline all sources → merged ranked results
// - Mixed success/failure → partial results + warnings
// - search_source single source
// - list_sources health check
// - Markdown table output format verification
// - Response truncation with file save (>50 results)
// - search_source richer detail format
```

**Files:**
- Test: `tests/test_integration.rs`
- Impl: glue code if needed

- [ ] **Step 1:** C dispatches Agent A to write `tests/test_integration.rs`
- [ ] **Step 2:** C verifies tests compile
- [ ] **Step 3:** C dispatches Agent B to fix any glue code needed for integration
- [ ] **Step 4:** C runs: `cargo test --test test_integration`
  Expected: all 11 tests pass
- [ ] **Step 5:** C runs full suite: `cargo test`
  Expected: ALL tests pass (models + core + 4 adapters + config + server + integration)
- [ ] **Step 6:** Commit

```bash
git add tests/test_integration.rs
git commit -m "feat: add integration tests for full pipeline"
```

---

## Task 10: README + Release

**Agent:** C does directly (no TDD needed)

- [ ] **Step 1:** Write `README.md` with:
  - One-line description
  - Quick start (install, configure, run)
  - Auth setup (Slack, Atlassian, Local) — copy from DESIGN.md §9
  - Config reference — point to config.example.yaml
  - MCP tool descriptions
  - Build from source instructions

- [ ] **Step 2:** Copy DESIGN.md into repo docs/

```bash
cp ~/IdeaProjects/product-amls/docs/superpowers/specs/2026-03-17-unified-search-mcp-design.md \
   ~/IdeaProjects/unified-search-mcp/docs/DESIGN.md
```

- [ ] **Step 3:** Final checks

```bash
cargo test          # all tests pass
cargo build --release   # release binary builds
ls -lh target/release/unified-search-mcp  # verify binary size
```

- [ ] **Step 4:** Commit

```bash
git add README.md docs/
git commit -m "docs: add README with setup guide and copy design spec"
```

- [ ] **Step 5:** Create GitHub repo and push

```bash
cd ~/IdeaProjects/unified-search-mcp
gh repo create ganesh-tt/unified-search-mcp --public --description "Unified search MCP server — search Slack, Confluence, JIRA, and local files in parallel" --source=.
git push -u origin main
```

- [ ] **Step 6:** Tag release

```bash
git tag v0.1.0
git push origin v0.1.0
```

---

## Task 11 (Future): Local Vector Search (Phase 8)

**Not in Phase 1 scope.** Placeholder for Phase 2 implementation.

**Depends on:** Task 3 (core) only — can be done independently
**Feature flag:** `vector` in Cargo.toml
**Files:** `src/sources/local_vector.rs`, `tests/test_local_vector.rs`
**Crates:** `ort = "2.0.0-rc.12"`, `hnsw_rs = "0.3"`, `tokenizers = "0.19"`

---

## Parallel Execution Map

```
Task 1 (Scaffold) ─── C direct
    │
Task 2 (Models) ─── A→B sequential
    │
Task 3 (Core) ─── A→B sequential
    │
    ├── Task 4 (LocalText) ─── A→B ─┐
    ├── Task 5 (Confluence) ── A→B ──┤ ALL 4 IN PARALLEL
    ├── Task 6 (JIRA) ──────── A→B ──┤ (each in own branch, merge sequentially after)
    └── Task 7 (Slack) ─────── A→B ──┘
                                     │
Task 8a (Config) ─── A→B sequential
    │
Task 8b (MCP Server) ─── A→B sequential
    │
Task 9 (Integration) ─── A→B sequential
    │
Task 10 (README + Release) ─── C direct
```

**Estimated total:** 11 tasks, ~100 test cases, ~15 source files
