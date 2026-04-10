# unified-search-mcp

## Tech Stack
- Rust 1.80+, tokio async runtime, rmcp for MCP protocol
- reqwest for HTTP (Slack, Confluence, JIRA), tokio::process for CLI subprocess (GitHub `gh`, local `rg`)
- serde_json for all JSON parsing, chrono for timestamps, regex for pattern matching

## Architecture
- `src/sources/*.rs` — each source implements `SearchSource` trait (name, description, health_check, search)
- `src/core.rs` — `SearchOrchestrator` fans out to sources in parallel, merges, ranks, deduplicates
- `src/server.rs` — `UnifiedSearchServer` holds orchestrator + individual source instances for `get_detail`
- `src/mcp.rs` — rmcp tool registration, stdio transport
- `src/resolve.rs` — identifier auto-detection (JIRA keys, URLs, Slack permalinks, GitHub URLs)
- `src/cache.rs` — in-memory LRU response cache with TTL
- `src/metrics.rs` — JSONL metrics logger (awaited spawn_blocking for sequential writes)

## Testing
- All HTTP sources use `wiremock` for deterministic mocking — no real API calls in tests
- GitHub source uses configurable `gh_path` pointing to mock shell scripts in temp dirs
- Local text source tests use `tempfile` crate for temp directories with fixture files
- Test files mirror source files: `tests/test_jira.rs`, `tests/test_confluence.rs`, etc.
- Fixtures in `fixtures/` directory, loaded via `include_str!`

## Common Gotchas
- **ETXTBSY on Linux**: `NamedTempFile` keeps write fd open — exec fails on Linux. Use `TempDir` + `std::fs::write` for mock scripts.
- **MSRV must match features**: `LazyLock` requires 1.80. Clippy enforces this via MSRV lint. Bump `Cargo.toml` rust-version if adding newer APIs.
- **`cargo clippy --fix`** auto-fixes most lint issues but NOT: dead code, identical blocks, redundant conditions — those need manual fixes.
- **Dedup must stay O(n)**: `core.rs` dedup uses HashSet for URL + snippet lookups. Never revert to pairwise comparison — caused 37s p95 at O(n²).
- **Don't background cache writes**: `tokio::spawn` for cache put breaks `cache_returns_cached_results` test (race). Cache put is microseconds — keep synchronous.
- **External tools**: `rg` (ripgrep) for local_text, `gh` CLI for GitHub. Both already optimal — no speedup from switching tools.
- `SearchOrchestrator::new()` signature changes require updates in: main.rs, test_core.rs, test_server.rs, test_integration.rs
- `UnifiedSearchServer::new()` same — currently takes (orchestrator, jira, confluence, slack, github, metrics)
- `config.yaml` is gitignored (has secrets via env vars). Update `config.example.yaml` for new config fields.
- The MCP binary is loaded at Claude session start — `cargo build --release` mid-session doesn't take effect until next session
- `cargo test` runs ~207 tests across 15 test files. All must pass before committing.

## Async Patterns (MUST follow)
- All MCP tool handlers MUST have a `tokio::time::timeout` safety net (30s for detail, 45s for enriched search)
- Sync file I/O (`std::fs::*`, `walkdir`, `grep_searcher`) → `tokio::task::spawn_blocking`, never on async runtime
- HTTP clients: use `Source::build_client()` + `new_with_client()` to share `reqwest::Client` between search and detail paths
- `tokio::sync::Mutex` for cache, not `std::sync::Mutex` — prevents deadlock if lock held across `.await`
- String normalization: single-pass with `String::with_capacity` — never `split_whitespace().collect::<Vec>().join()` (3 allocations per call)
- Regexes: `LazyLock<Regex>` at module level, never compile inside functions
- JSON arrays: borrow via `.as_array().unwrap_or(&empty)`, don't `.cloned()`

## MCP Tool Tiers
- **Fast** (no extra API calls): `unified_search`, `search_source`, `get_detail`, `list_sources`
- **Deep** (enriched, max 10 results, 45s timeout): `search_confluence_comments`, `search_jira_comments`, `search_slack_threads`
- Deep tools use `futures::stream::buffered(5)` for bounded concurrency

## Confluence URL Patterns
- `resolve.rs` handles 4 URL forms: `/wiki/spaces/*/pages/ID`, `/spaces/*/pages/ID`, `/wiki/rest/api/content/ID`, `/wiki/api/v2/pages/ID`
- Some Confluence pages 404 on v2 API — the v1 REST fallback path must always work

## Logging
- `RUST_LOG=unified_search_mcp=info` — shows get_detail timing, timeout events, large response warnings
- `RUST_LOG=unified_search_mcp=debug` — adds HTTP response status/size per request
- Logs go to stderr (stdout = MCP JSON-RPC channel)

## Build & Run
- `cargo build --release` — release binary at `target/release/unified-search-mcp`
- `cargo test` — run all tests
- `./target/release/unified-search-mcp --verify --config config.yaml` — preflight check
- `./target/release/unified-search-mcp --stats --days 7` — adoption report
- `gh release create vX.Y.Z --target master` — triggers release.yml, builds 4 platform binaries (linux/macos × x86_64/aarch64)
- **Release checklist**: bump `version` in Cargo.toml → add CHANGELOG entry → commit+push → `gh release create vX.Y.Z`
- `gh repo edit --visibility public --accept-visibility-change-consequences` — required flag for visibility changes
- Release assets auto-upload as `.tar.gz` to the GitHub release
- To re-trigger: `gh release delete vX.Y.Z --yes && git push origin :refs/tags/vX.Y.Z` then recreate
