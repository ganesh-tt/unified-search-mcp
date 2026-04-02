# Contributing to unified-search-mcp

## Getting Started

```bash
git clone https://github.com/ganesh-tt/unified-search-mcp.git
cd unified-search-mcp
cargo build
cargo test
```

Requires Rust 1.75+. Install via [rustup](https://rustup.rs/).

## Project Structure

```
src/
  main.rs          # CLI entry point, arg parsing, source wiring
  lib.rs           # Module declarations
  config.rs        # YAML config loading, env var interpolation
  core.rs          # SearchOrchestrator — fan-out, merge, rank, dedup
  server.rs        # UnifiedSearchServer — MCP tool handlers
  mcp.rs           # rmcp tool registration, stdio transport
  models.rs        # Data types (SearchResult, SearchQuery, etc.)
  resolve.rs       # Identifier auto-detection (JIRA key, URLs, Slack permalinks)
  metrics.rs       # JSONL metrics logger
  stats.rs         # --stats CLI adoption report
  sources/
    mod.rs         # SearchSource trait
    slack.rs       # Slack search + get_detail_thread
    confluence.rs  # Confluence search + comment enrichment + get_detail_page
    jira.rs        # JIRA search + comment extraction + get_detail_issue
    local_text.rs  # Local file search (ripgrep + grep-regex fallback)

tests/             # One test file per module, wiremock for HTTP mocking
fixtures/          # JSON fixtures for wiremock responses
```

## Adding a New Source

1. Create `src/sources/your_source.rs`
2. Implement the `SearchSource` trait:

```rust
use async_trait::async_trait;
use crate::models::{SearchQuery, SearchResult, SearchError, SourceHealth};
use crate::sources::SearchSource;

pub struct YourSource { /* config, client */ }

#[async_trait]
impl SearchSource for YourSource {
    fn name(&self) -> &str { "your_source" }
    fn description(&self) -> &str { "Description" }
    async fn health_check(&self) -> SourceHealth { /* ... */ }
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> { /* ... */ }
}
```

3. Add `pub mod your_source;` to `src/sources/mod.rs`
4. Add config types to `src/config.rs`
5. Wire it in `src/main.rs` (same pattern as existing sources)
6. Add tests in `tests/test_your_source.rs` using wiremock
7. Add fixtures in `fixtures/your_source/`

See `src/sources/slack.rs` for a complete example.

## Running Tests

```bash
cargo test                           # All tests
cargo test test_jira                 # One test file
cargo test get_detail_issue -- -v    # One test by name
cargo test -- --nocapture            # Show println output
```

All HTTP tests use [wiremock](https://crates.io/crates/wiremock) — no real API calls, no credentials needed.

## Code Style

- Follow existing patterns in the codebase
- `cargo clippy` should produce no new warnings
- No unnecessary abstractions — if it's used once, inline it
- Error messages should tell the user what to fix, not just what failed

## Pull Requests

1. Fork the repo and create a feature branch
2. Write tests first (TDD preferred)
3. Keep PRs focused — one feature or fix per PR
4. Run `cargo test` and `cargo clippy` before submitting
5. Describe what changed and why in the PR description

## Reporting Issues

Open a GitHub issue with:
- What you expected
- What actually happened
- Steps to reproduce
- Output of `unified-search-mcp --verify --config your-config.yaml`
