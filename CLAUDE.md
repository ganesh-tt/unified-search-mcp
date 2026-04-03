# unified-search-mcp

## Tech Stack
- Rust 1.75+, tokio async runtime, rmcp for MCP protocol
- reqwest for HTTP (Slack, Confluence, JIRA), tokio::process for CLI subprocess (GitHub `gh`, local `rg`)
- serde_json for all JSON parsing, chrono for timestamps, regex for pattern matching

## Architecture
- `src/sources/*.rs` — each source implements `SearchSource` trait (name, description, health_check, search)
- `src/core.rs` — `SearchOrchestrator` fans out to sources in parallel, merges, ranks, deduplicates
- `src/server.rs` — `UnifiedSearchServer` holds orchestrator + individual source instances for `get_detail`
- `src/mcp.rs` — rmcp tool registration, stdio transport
- `src/resolve.rs` — identifier auto-detection (JIRA keys, URLs, Slack permalinks, GitHub URLs)
- `src/cache.rs` — in-memory LRU response cache with TTL
- `src/metrics.rs` — JSONL metrics logger (fire-and-forget via tokio::spawn)

## Testing
- All HTTP sources use `wiremock` for deterministic mocking — no real API calls in tests
- GitHub source uses configurable `gh_path` pointing to mock shell scripts in temp dirs
- Local text source tests use `tempfile` crate for temp directories with fixture files
- Test files mirror source files: `tests/test_jira.rs`, `tests/test_confluence.rs`, etc.
- Fixtures in `fixtures/` directory, loaded via `include_str!`

## Common Gotchas
- `SearchOrchestrator::new()` signature changes require updates in: main.rs, test_core.rs, test_server.rs, test_integration.rs
- `UnifiedSearchServer::new()` same — currently takes (orchestrator, jira, confluence, slack, github, metrics)
- `config.yaml` is gitignored (has secrets via env vars). Update `config.example.yaml` for new config fields.
- The MCP binary is loaded at Claude session start — `cargo build --release` mid-session doesn't take effect until next session
- `cargo test` runs ~196 tests across 14 test files. All must pass before committing.

## Build & Run
- `cargo build --release` — release binary at `target/release/unified-search-mcp`
- `cargo test` — run all tests
- `./target/release/unified-search-mcp --verify --config config.yaml` — preflight check
- `./target/release/unified-search-mcp --stats --days 7` — adoption report
