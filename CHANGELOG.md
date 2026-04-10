# Changelog

## v0.3.1 (2026-04-10)

### Fixed
- CI failures: `cargo fmt --check` and `cargo clippy -- -D warnings` now pass
- Bumped MSRV from 1.75 to 1.80 (required by `LazyLock` and other 1.80+ features)
- Fixed 49 clippy warnings: `map_or` → `is_some_and`, dead code annotations, redundant conditions, identical branches

### Added
- One-click install script (`install.sh`) — installs Rust, builds binary, creates config
- Updated README: all 7 MCP tools documented (including 3 deep-enrichment tools), GitHub source in examples, accurate test/binary metrics

## v0.3.0 (2026-04-03)

### Added
- **GitHub source** -- search PRs, issues, and code via `gh` CLI subprocess, scoped to configured orgs/repos
- **GitHub get_detail** -- full PR details (reviews, line comments, diff stats) and issue details (comments, labels) via `get_detail` tool
- **GitHub auto-detection** -- GitHub PR/issue URLs auto-detected by `get_detail`; `repo#N` shorthand with explicit `source="github"`
- **Response caching** -- in-memory LRU cache (max 100 entries, default 5min TTL), `no_cache` parameter for bypass
- **Confluence Markdown** -- `get_detail` for Confluence pages now returns full Markdown (headings, bold, lists, tables, code blocks, Confluence macros) instead of stripped plain text
- **`cache_ttl_seconds`** config option (default 300, set 0 to disable)
- **Cache hit indicator** in response footer (`**Cache**: HIT`)

### Changed
- `SearchOrchestrator::new()` now accepts `cache_ttl_seconds` parameter
- `unified_search` and `search_source` tools accept optional `no_cache` parameter
- `UnifiedSearchResponse` includes `cache_hit: bool` field
- `get_detail` Confluence output preserves document structure (headings, tables, lists)

## v0.2.0 (2026-04-02)

### Added
- **JIRA comment extraction** — search results now include comments from the JIRA API response (no extra calls)
- **Confluence comment enrichment** — parallel comment fetch for each search result page
- **`get_detail` MCP tool** — deep-dive lookups for JIRA tickets, Confluence pages, and Slack threads
  - Auto-detects JIRA keys (`FIN-1234`), Atlassian URLs, and Slack permalinks
  - Optional `source` parameter to force interpretation
  - Returns full Markdown with comments, linked issues, subtasks, child pages, thread replies
- **Identifier auto-detection** (`src/resolve.rs`) — parses URLs and patterns to route to the correct source
- **JSONL metrics logger** — every tool call logged to `~/.unified-search/metrics.jsonl`
- **Per-source stats** — response footer shows latency, result count, and comment count per source
- **`--stats` CLI mode** — adoption report comparing unified-search usage vs individual MCP bypasses
- **Configurable `metrics_path`** in `config.yaml`
- **Updated MCP instructions** — Claude now knows about `get_detail` and comments in search results

### Changed
- `UnifiedSearchResponse` now includes `per_source_stats: Vec<PerSourceStats>`
- Response footer enhanced with per-source breakdown
- `unified_search` tool description updated to mention comments and replace individual MCPs

## v0.1.0 (2026-03-17)

### Added
- Initial release
- 4 MCP tools: `unified_search`, `search_source`, `list_sources`, `index_local`
- 4 source adapters: Slack, Confluence, JIRA, local text (ripgrep + grep-regex fallback)
- Parallel fan-out search with configurable timeouts
- Cross-source ranking with weighted relevance scoring
- URL and snippet-prefix deduplication
- YAML config with `${ENV_VAR}` interpolation
- `--verify` preflight check
- MIT license
