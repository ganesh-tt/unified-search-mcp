# Changelog

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
