# Performance Review: unified-search-mcp
**Date**: 2026-04-08
**Reviewer**: Rust Performance Specialist Agent (Claude Opus 4.6)

## Summary

The codebase is well-structured with good separation of concerns. The recent timeout additions (30s operation-level, per-request timeouts on comment enrichment, connect timeouts) address the immediate hanging problem. Several performance and reliability issues remain.

---

## CRITICAL Findings (causes hangs/crashes)

### 1. Blocking sync I/O inside tokio runtime -- `save_full_results` and `MetricsLogger::write_entry`

**File**: `src/server.rs:539-558`, `src/metrics.rs:51-103`

`save_full_results()` calls `std::fs::create_dir_all`, `std::fs::write`, and `std::fs::set_permissions` synchronously on the tokio runtime. If the filesystem is NFS-mounted, or the disk is slow, this blocks the entire tokio worker thread.

Similarly, `MetricsLogger::log()` spawns a tokio task that calls sync `std::fs::*` ops. While the spawn isolates it from the caller, it still blocks a tokio worker thread.

**Fix**: Use `tokio::task::spawn_blocking` for all filesystem operations.
**Status**: FIXED

### 2. Unbounded response body sizes

**File**: `src/sources/confluence.rs` (`.text().await`), `src/sources/jira.rs` (`.json().await`)

No response body size limit. A malicious or buggy Confluence response could return gigabytes, causing OOM.

**Fix**: Add Content-Length check before reading body. Log warning for large responses.
**Status**: FIXED (warning-only, no rejection — user needs full data)

### 3. `std::sync::Mutex` in async context -- cache contention

**File**: `src/core.rs:32`

`std::sync::Mutex` is used inside async code. While technically safe (lock is held briefly, never across await points), it's fragile — any future refactoring that adds an `.await` while holding the lock will silently deadlock.

**Fix**: Replace with `tokio::sync::Mutex` or `parking_lot::Mutex`.
**Status**: NOT FIXED (low risk, needs careful refactor)

---

## HIGH Findings (measurable performance impact)

### 4. Duplicate HTTP clients -- double memory and connection pools

**File**: `src/main.rs:95-171`

Every source is instantiated **twice**: once for orchestrator search, once for `get_detail`. Creates 6 separate HTTP clients when only 3 are needed.

**Fix**: Share a single `reqwest::Client` instance per source.
**Status**: NOT FIXED (architectural change, future PR)

### 5. Confluence `enrich_with_comments` runs during search -- adds N HTTP calls

**File**: `src/sources/confluence.rs:468`

Every Confluence search makes up to 20 additional HTTP requests to fetch comments. Worst case: 4 batches x 10s = 40s added latency, exceeding orchestrator's 10s timeout.

**Fix**: Skip enrichment during search — `get_detail` already fetches comments.
**Status**: FIXED

### 6. Regex compiled on every `get_detail_pr` and `get_detail_issue` call

**File**: `src/sources/github.rs:134, 281`

`Regex::new()` compiled from scratch on every call.

**Fix**: Use `static VALID_GH_NAME_RE: LazyLock<Regex>` at module level.
**Status**: FIXED

### 7. `serde_json::Value` cloning for large JSON responses

**File**: `src/sources/jira.rs`, `src/sources/github.rs` (multiple locations)

`.cloned()` on JSON arrays deep-clones entire comment/link trees. For issues with 100+ comments, this is 100KB+ of unnecessary allocation.

**Fix**: Borrow via references instead of clone.
**Status**: FIXED

### 8. GitHub shorthand fallback: sequential PR then Issue attempt

**File**: `src/server.rs:467-510`

`repo#N` shorthand tries `get_detail_pr` first, then `get_detail_issue` on failure. Wastes 10-30s if item is an issue.

**Fix**: Parallel detection with `tokio::select!`.
**Status**: FIXED

---

## MEDIUM Findings (suboptimal but works)

### 9. O(n^2) deduplication in orchestrator

**File**: `src/core.rs:188-209`

Each result iterates all kept results for dedup. With 100 results = 10,000 comparisons, each allocating Strings via `normalize_snippet_prefix`.

**Fix**: Use HashSet-based dedup.
**Status**: NOT FIXED (low practical impact at typical result counts)

### 10. `confluence_markdown::Walker` clones every token

**File**: `src/sources/confluence_markdown.rs:270`

Tokens can contain large strings (entire paragraphs). For 500+ tokens, doubles memory usage.

**Fix**: VecDeque with `pop_front()`.
**Status**: NOT FIXED

### 11. `decode_entities` creates 7 intermediate Strings

**File**: `src/sources/confluence_markdown.rs:198-206`

Each `.replace()` creates a new String even if no substitution occurs.

**Fix**: Early return if no `&` in input.
**Status**: FIXED

### 12. `tokenize` converts input to `Vec<char>` upfront

**File**: `src/sources/confluence_markdown.rs:28`

For 100KB HTML = ~400KB (4 bytes per char).

**Fix**: Use `as_bytes()` for ASCII checks.
**Status**: NOT FIXED

### 13. `search_with_fallback` runs sync file I/O on async runtime

**File**: `src/sources/local_text.rs:285-434`

`walkdir::WalkDir` and `grep_searcher::Searcher` are synchronous. Blocks tokio runtime if rg unavailable.

**Fix**: Wrap in `spawn_blocking`.
**Status**: NOT FIXED

### 14. Slack `get_detail_thread` makes sequential API calls

**File**: `src/sources/slack.rs:84-127`

`conversations.info` and `conversations.replies` are independent but run sequentially.

**Fix**: Use `tokio::join!` to parallelize.
**Status**: FIXED

### 15. Cache key computation allocates unnecessary Strings

**File**: `src/cache.rs:23-28`

Key computed twice per search (get + put).

**Fix**: Compute once, pass through.
**Status**: NOT FIXED

---

## LOW Findings (micro-optimizations)

| # | Finding | Status |
|---|---------|--------|
| 16 | Repeated `.to_string()` for static source names | NOT FIXED |
| 17 | `HashMap::new()` for small metadata maps (2-4 entries) | NOT FIXED |
| 18 | `normalize_snippet_prefix` called per dedup comparison | NOT FIXED |
| 19 | `parse_slack_ts` uses f64 (precision loss) | NOT FIXED |

---

## What Was Done Well

- Lazy-compiled regexes in resolve.rs and jira.rs
- Bounded concurrency on comment enrichment (buffered(5) with per-request timeout)
- 30s operation timeout on `handle_get_detail`
- `connect_timeout(5s)` on all HTTP clients
- Parallel fan-out via `tokio::spawn` in orchestrator
- Parallel GitHub API calls in `get_detail_pr` using `tokio::join!`
- Redacted Debug impls on config structs

## Fix Summary

| Finding | Severity | Status |
|---------|----------|--------|
| 1. Blocking I/O | CRITICAL | FIXED |
| 2. Unbounded response | CRITICAL | FIXED (warn only) |
| 3. Mutex in async | CRITICAL | DEFERRED |
| 4. Duplicate clients | HIGH | DEFERRED |
| 5. Comment enrichment | HIGH | FIXED |
| 6. Regex per call | HIGH | FIXED |
| 7. JSON cloning | HIGH | FIXED |
| 8. Shorthand fallback | HIGH | FIXED |
| 11. decode_entities | MEDIUM | FIXED |
| 14. Slack sequential | MEDIUM | FIXED |

## Additional Fixes (same session)

| Fix | Description |
|-----|-------------|
| Confluence URL resolver | Added support for /spaces/ (no /wiki/), /rest/api/content/, /api/v2/pages/ patterns |
| Deep search tools | 3 new enriched tools: search_confluence_comments, search_jira_comments, search_slack_threads |
| 45s enriched search timeout | Safety net on all deep search tools |
| tracing subscriber init | Logs to stderr with RUST_LOG env filter support |
| Metrics logger await | spawn_blocking + await ensures sequential writes, no race conditions |
