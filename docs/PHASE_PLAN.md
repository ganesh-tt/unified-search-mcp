# Phase Plan — Unified Search MCP Server

## Agent Roles

| Agent | Role | Rules |
|-------|------|-------|
| **C** (Orchestrator) | Dispatches A & B, validates green, advances phases | Never writes code. Only passes trait + scope. No context leaks between A/B. |
| **A** (Test Writer) | Writes comprehensive tests per module | Never sees B's implementation. Receives: trait, scope, fixtures. |
| **B** (Implementer) | Writes code to pass A's tests | Sees A's tests (read-only). **CANNOT modify tests.** Escalates to C if test seems wrong. |

## Phase Dependency Graph

```
Phase 0 (Scaffold)
    │
    ▼
Phase 1 (Models)
    │
    ▼
Phase 2 (Core + Trait)
    │
    ├──────────┬──────────┬──────────┐
    ▼          ▼          ▼          ▼
Phase 3     Phase 4    Phase 5    Phase 6
(LocalText) (Conflu.)  (JIRA)    (Slack)
    │          │          │          │
    └──────────┴──────────┴──────────┘
               │
               ▼
         Phase 7 (MCP Server)
               │
               ▼
         Phase 9 (Integration)

Phase 8 (Vector) ── independent, after Phase 2
```

---

## Phase 0: Project Scaffold

**Agent**: Single agent (C does this directly)
**Output**: Compilable empty project

### Tasks
- [ ] `Cargo.toml` with all dependencies
- [ ] `src/main.rs` — `fn main() { println!("unified-search-mcp"); }`
- [ ] `src/models.rs` — empty module
- [ ] `src/config.rs` — empty module
- [ ] `src/core.rs` — empty module
- [ ] `src/server.rs` — empty module
- [ ] `src/sources/mod.rs` — empty module with trait stub
- [ ] `src/sources/slack.rs` — empty module
- [ ] `src/sources/confluence.rs` — empty module
- [ ] `src/sources/jira.rs` — empty module
- [ ] `src/sources/local_text.rs` — empty module
- [ ] `src/sources/local_vector.rs` — empty module
- [ ] `config.example.yaml`
- [ ] `.gitignore` (target/, config.yaml, *.swp)
- [ ] `fixtures/` directory with sample JSON files
- [ ] `tests/common/mod.rs` — shared test helpers stub
- [ ] Verify: `cargo check` passes

---

## Phase 1: Models

**Agent sequence**: A → B
**Scope**: `src/models.rs`
**Test file**: `tests/test_models.rs`

### Contract (what A and B both receive)

```rust
// Models to implement:
// - SearchQuery { text, max_results, filters }
// - SearchFilters { sources, after, before }
// - SearchResult { source, title, snippet, url, timestamp, relevance, metadata }
// - SourceHealth { source, status, message, latency_ms }
// - HealthStatus enum { Healthy, Degraded, Unavailable }
// - UnifiedSearchResponse { results, warnings, total_sources_queried, query_time_ms }
//
// All must: derive Serialize + Deserialize + Debug + Clone
// SearchResult must: impl PartialOrd (by relevance DESC, then timestamp DESC)
// SearchQuery must: impl Default (max_results=20, empty filters)
```

### Agent A writes tests for
- [x] Serialization round-trip (struct → JSON → struct) for all types
- [x] SearchResult ordering (higher relevance first, then more recent)
- [x] SearchQuery defaults
- [x] SearchFilters with all fields, partial fields, no fields
- [x] HealthStatus display (Healthy → "healthy", etc.)
- [x] UnifiedSearchResponse with mixed results and warnings
- [x] Empty metadata HashMap
- [x] Edge cases: relevance = 0.0, relevance = 1.0, None timestamps

### Agent B implements
- All structs with serde derives
- `PartialOrd` / `Ord` for SearchResult
- `Default` for SearchQuery
- Display for HealthStatus

---

## Phase 2: Core + Trait

**Agent sequence**: A → B
**Scope**: `src/sources/mod.rs` (trait), `src/core.rs` (orchestrator)
**Test file**: `tests/test_core.rs`

### Contract

```rust
// SearchSource trait:
//   fn name(&self) -> &str
//   fn description(&self) -> &str
//   async fn health_check(&self) -> SourceHealth
//   async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>>
//
// Orchestrator (core.rs):
//   pub struct SearchOrchestrator { sources: Vec<Box<dyn SearchSource>>, config: OrchestratorConfig }
//   pub async fn search(&self, query: &SearchQuery) -> UnifiedSearchResponse
//   pub async fn health_check_all(&self) -> Vec<SourceHealth>
//
// OrchestratorConfig:
//   timeout_seconds: u64
//   source_weights: HashMap<String, f32>
//
// Ranking: relevance * source_weight, then timestamp DESC, then dedup by URL
```

### Agent A writes tests for (using mock sources)
- [x] Single source, happy path → results returned
- [x] Multiple sources, all succeed → results merged and sorted by weighted relevance
- [x] One source times out → other results returned, warning included
- [x] One source errors → other results returned, warning with error message
- [x] All sources fail → empty results, all warnings
- [x] Source weights: source with weight=2.0 ranks above same-relevance source with weight=1.0
- [x] Dedup: two results with same URL → keep higher-scored one
- [x] max_results respected (more total results than limit → truncated)
- [x] sources filter in query → only named sources queried
- [x] health_check_all → returns health from all sources
- [x] Empty sources list → empty results, no crash

### Mock sources for testing

```rust
// A should create these in the test file:
// - SuccessSource: returns N predefined results
// - EmptySource: returns empty vec
// - ErrorSource: returns Err with message
// - SlowSource: sleeps for N seconds then returns results
// - PanicSource: panics (should not crash orchestrator)
```

### Agent B implements
- `SearchSource` trait in `sources/mod.rs`
- `SearchOrchestrator` in `core.rs` with `tokio::join_all` + timeout
- Ranking + dedup logic

---

## Phase 3: Local Text Search

**Agent sequence**: A → B (can run in PARALLEL with Phases 4–6)
**Scope**: `src/sources/local_text.rs`
**Test file**: `tests/test_local_text.rs`

### Contract

```rust
// LocalTextSource implements SearchSource
// Config: paths (Vec<PathBuf>), include_patterns, exclude_patterns, max_file_size_bytes
// Search method: spawn `rg --json` subprocess → parse JSON output → SearchResult
// Fallback: if rg not in PATH, use grep-regex + walkdir for Rust-native search
// Relevance: match_count / max_match_count across files (normalized 0.0–1.0)
```

### Agent A writes tests for
- [x] Search finds matches in sample files (use fixtures/local/sample_codebase/)
- [x] Include patterns respected (*.rs matches, *.py skipped)
- [x] Exclude patterns respected (**/target/** skipped)
- [x] No matches → empty results, no error
- [x] Path doesn't exist → warning, no crash
- [x] Max file size respected (file >1MB skipped)
- [x] Snippet extraction: matched line with surrounding context
- [x] Multiple matches in same file → single result with highest match count
- [x] Relevance ordering: file with 5 matches scores higher than file with 1
- [x] file:// URL generation from absolute path
- [x] Regex special characters in query don't crash (escape them)
- [x] Empty query → empty results

### Fixtures needed
- `fixtures/local/sample_codebase/main.rs` (contains "SearchResult", "tokio", "async")
- `fixtures/local/sample_codebase/config.yaml` (contains "timeout", "slack")
- `fixtures/local/sample_codebase/target/debug/build.rs` (should be excluded)
- `fixtures/local/sample_codebase/large_file.bin` (>1MB, should be skipped)
- `fixtures/local/sample_docs/design.md` (contains "broadcast threshold", "ranking")
- `fixtures/local/sample_docs/notes.txt` (contains "JIRA", "FIN-10384")

### Agent B implements
- `LocalTextSource` struct + `SearchSource` impl
- ripgrep subprocess spawning with JSON output parsing
- Fallback Rust-native search
- Pattern matching, file size filtering

---

## Phase 4: Confluence Search

**Agent sequence**: A → B (PARALLEL with Phases 3, 5, 6)
**Scope**: `src/sources/confluence.rs`
**Test file**: `tests/test_confluence.rs`

### Contract

```rust
// ConfluenceSource implements SearchSource
// Config: base_url, email, api_token, spaces (optional filter)
// API: GET /wiki/rest/api/search?cql=siteSearch~"query"&limit=N (v1 — v2 has no search endpoint)
// Auth: Basic Auth (email:api_token base64)
// Result mapping: results[] → SearchResult (title, excerpt stripped of HTML, page URL)
```

### Agent A writes tests for (using wiremock)
- [x] Successful search → results mapped correctly (title, snippet, URL)
- [x] HTML stripped from excerpt
- [x] Space filter applied in CQL when spaces configured
- [x] Empty results → empty vec, no error
- [x] 401 response → descriptive error with auth hint
- [x] 403 response → permission error
- [x] 429 response → rate limit error with Retry-After value
- [x] 500 response → server error surfaced
- [x] Network timeout → timeout error
- [x] Malformed JSON response → parse error (not crash)
- [x] health_check → GET /wiki/rest/api/space?limit=1 succeeds
- [x] Relevance: use API's result order (first result = 1.0, last = lower)
- [x] Query with double quotes escaped in CQL (no injection)
- [x] Query with CQL operators (AND, OR) treated as literal text
- [x] Time filter after/before mapped to CQL lastmodified clauses

### Fixtures needed
- `fixtures/confluence/search_success.json` — 3 results with HTML excerpts
- `fixtures/confluence/search_empty.json` — 0 results
- `fixtures/confluence/search_auth_failure.json` — 401 response body

### Agent B implements
- `ConfluenceSource` struct with reqwest client
- CQL query construction
- HTML tag stripping for snippets
- Response parsing and mapping

---

## Phase 5: JIRA Search

**Agent sequence**: A → B (PARALLEL with Phases 3, 4, 6)
**Scope**: `src/sources/jira.rs`
**Test file**: `tests/test_jira.rs`

### Contract

```rust
// JiraSource implements SearchSource
// Config: base_url, email, api_token, projects (optional filter)
// API: GET /rest/api/3/search?jql=text~"query"&maxResults=N&fields=summary,description,comment,status,updated,assignee
// Auth: Basic Auth (email:api_token base64)
// Result mapping: issues[] → SearchResult (summary as title, description snippet, browse URL)
```

### Agent A writes tests for (using wiremock)
- [x] Successful search → results mapped (title=summary, URL=browse link)
- [x] Project filter applied in JQL when projects configured
- [x] Description truncated to first 300 chars for snippet
- [x] Empty results → empty vec
- [x] 401 response → auth error with hint
- [x] 403 response → permission error
- [x] 429 response → rate limit error
- [x] 500 response → server error
- [x] Network timeout → timeout error
- [x] Malformed JSON → parse error
- [x] health_check → GET /rest/api/3/myself succeeds
- [x] Metadata includes: project key, status, assignee
- [x] Relevance: API result order normalized
- [x] Query with double quotes escaped in JQL (no injection)
- [x] Query with JQL operators treated as literal text
- [x] Time filter after/before mapped to JQL updated clauses

### Fixtures needed
- `fixtures/jira/search_success.json` — 3 issues with full fields
- `fixtures/jira/search_empty.json` — 0 issues
- `fixtures/jira/search_auth_failure.json` — 401 body

### Agent B implements
- `JiraSource` struct with reqwest client
- JQL query construction
- Response parsing, description truncation
- Browse URL construction (`{base_url}/browse/{key}`)

---

## Phase 6: Slack Search

**Agent sequence**: A → B (PARALLEL with Phases 3, 4, 5)
**Scope**: `src/sources/slack.rs`
**Test file**: `tests/test_slack.rs`

### Contract

```rust
// SlackSource implements SearchSource
// Config: user_token (xoxp-), bot_token (xoxb-, optional for enrichment)
// API: GET https://slack.com/api/search.messages (user token required — NOT POST)
// Auth: Authorization: Bearer {user_token}
// Result mapping: messages.matches[] → SearchResult (text snippet, permalink, channel as metadata)
```

### Agent A writes tests for (using wiremock)
- [x] Successful search → results mapped (text as snippet, permalink as URL)
- [x] Channel name included in metadata
- [x] Username included in metadata
- [x] Empty results → empty vec
- [x] `ok: false` response → error with Slack's error message
- [x] Invalid token → auth error with token type hint (must be xoxp-, not xoxb-)
- [x] Rate limited (429) → error with Retry-After
- [x] Network timeout → timeout error
- [x] Malformed JSON → parse error
- [x] health_check → POST auth.test succeeds
- [x] Relevance: Slack's score field normalized to 0.0–1.0
- [x] Timestamp parsed from Slack's `ts` field (Unix epoch with decimal)

### Fixtures needed
- `fixtures/slack/search_messages_success.json` — 3 messages with permalinks
- `fixtures/slack/search_messages_empty.json` — 0 messages
- `fixtures/slack/search_messages_rate_limited.json` — 429 with headers

### Agent B implements
- `SlackSource` struct with reqwest client
- search.messages API call (GET with query params)
- Response parsing (Slack's non-standard `ok` field)
- Permalink extraction, timestamp conversion

---

## Phase 7: MCP Server + Config

**Agent sequence**: A → B
**Depends on**: Phases 3–6 complete
**Scope**: `src/server.rs`, `src/config.rs`, `src/main.rs`
**Test file**: `tests/test_config.rs` (config parsing), `tests/test_server.rs` (tool dispatch — optional, may be thin)

### Contract

```rust
// Config (config.rs):
//   pub fn load(path: Option<&str>) -> Result<AppConfig>
//   - Reads YAML, interpolates ${ENV_VAR}, validates, returns typed config
//   - AppConfig contains: server settings + per-source configs
//
// Server (server.rs):
//   - Registers MCP tools: unified_search, search_source, index_local, list_sources
//   - Wires tools to SearchOrchestrator
//   - Handles stdio JSON-RPC transport via rmcp
//
// main.rs:
//   - Load config → build sources from config → build orchestrator → start server
```

### Agent A writes tests for
- [x] Config: valid full YAML parses correctly
- [x] Config: minimal YAML (only local_text enabled) parses
- [x] Config: env var interpolation (`${FOO}` → value of FOO)
- [x] Config: missing env var → error naming the var
- [x] Config: invalid YAML → error with line number
- [x] Config: missing file → helpful error message
- [x] Config: disabled sources not instantiated
- [x] Config: tilde expansion in paths (`~/foo` → `/Users/x/foo`)
- [x] Config: default values applied when fields omitted

### Fixtures needed
- `fixtures/config/valid_full.yaml`
- `fixtures/config/valid_minimal.yaml`
- `fixtures/config/missing_env_var.yaml`
- `fixtures/config/invalid_syntax.yaml`

### Agent B implements
- YAML loading + env var interpolation
- Config validation
- Source instantiation from config
- MCP tool registration and dispatch

---

## Phase 8: Local Vector Search (Phase 2 — Independent)

**Agent sequence**: A → B
**Depends on**: Phase 2 only
**Scope**: `src/sources/local_vector.rs`
**Test file**: `tests/test_local_vector.rs`
**Feature flag**: `vector` (cargo feature, off by default)

### Contract

```rust
// LocalVectorSource implements SearchSource (behind "vector" feature flag)
// Config: model_path, index_path, paths, chunk_size, chunk_overlap, auto_reindex
// Index pipeline: discover files → extract text → chunk → embed (ONNX) → HNSW insert
// Search: embed query → k-nearest → map to files → SearchResult
// Also exposes: pub async fn reindex(&self) -> Result<IndexStats>
```

### Agent A writes tests for
- [x] Index creation from sample docs
- [x] Search returns relevant chunks
- [x] Cosine similarity scores in 0.0–1.0 range
- [x] PDF text extraction
- [x] Markdown text extraction
- [x] Chunk boundaries respect paragraph breaks
- [x] Reindex updates existing index
- [x] Missing model file → clear error
- [x] Empty document set → empty index, no crash
- [x] Large document → chunked correctly

### Agent B implements
- ONNX model loading
- Text extraction (PDF, MD, TXT)
- Chunking pipeline
- HNSW index build + search
- File watcher (optional)

---

## Phase 9: Integration Tests

**Agent sequence**: A → B
**Depends on**: All previous phases
**Scope**: `tests/test_integration.rs`

### Agent A writes tests for
- [x] Full pipeline: config load → build sources (all mocked) → unified_search → ranked results
- [x] Mixed source success/failure → partial results + warnings
- [x] search_source tool → single source queried
- [x] list_sources → all health statuses
- [x] Source filters in query respected
- [x] Time filters (after/before) passed through to sources
- [x] Result count respects global max_results
- [x] All sources disabled → empty results, appropriate message

### Agent B implements
- Integration test wiring
- Any glue code discovered during integration
