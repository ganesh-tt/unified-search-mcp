# Unified Search MCP v0.3 — GitHub Source, Caching, Confluence Markdown

**Date**: 2026-04-02
**Status**: Approved
**Scope**: Three enhancements to unified-search-mcp

## Goals

1. **GitHub source** — Search PRs, issues, and code across configured orgs/repos via the `gh` CLI. `get_detail` support for full PR/issue content with review comments, diff stats, and CI status.
2. **Response caching** — In-memory LRU cache with configurable TTL to avoid redundant API calls during investigations. Bypass parameter for forced refresh.
3. **Confluence body→Markdown** — Full-fidelity HTML-to-Markdown conversion for Confluence storage format, replacing the current tag-stripping approach in `get_detail_page`.

---

## 1. GitHub Source

### 1.1 Architecture

Uses the `gh` CLI as a subprocess (same pattern as `local_text` uses `rg`). No GitHub token in config — piggybacks on the user's existing `gh auth login`. REST API via reqwest is a future option.

### 1.2 Config

```yaml
sources:
  github:
    enabled: true
    orgs: ["tookitaki"]                                        # required: GitHub org(s)
    repos: ["product-amls", "product-dss", "gladiator-2.0"]   # optional: restrict to repos (empty = all in org)
    weight: 1.0
    max_results: 10
```

### 1.3 Search

Two parallel subprocess calls per search query:

**Issues/PRs**: `gh api search/issues` with query `{text} org:{org}` (optionally `repo:{org}/{repo}` if repos configured). Returns PR/issue title, number, state, body snippet, URL, updated timestamp.

**Code**: `gh api search/code` with query `{text} org:{org}`. Returns file path, repo, matched line snippet, URL.

Both parsed from JSON, merged into `Vec<SearchResult>` with:
- `source`: `"github"`
- `title`: `"org/repo#123: PR title"` for issues/PRs, `"org/repo: path/to/file.rs"` for code
- `snippet`: body excerpt (issues/PRs) or matched code lines (code search)
- `url`: GitHub URL
- `relevance`: normalized from API `score` field (same approach as Slack)
- `metadata`: `{"type": "pull_request"|"issue"|"code", "repo": "org/repo", "state": "open"|"closed"}`

**Rate limiting**: GitHub search API has 30 requests/minute for authenticated users. If `gh` returns a rate limit error, return `SearchError::RateLimited` with retry-after.

### 1.4 Health Check

Run `gh auth status --hostname github.com` as subprocess. Parse exit code: 0 = Healthy, non-zero = Unavailable with stderr as message.

### 1.5 get_detail for GitHub

**PR detail** (`gh api repos/{owner}/{repo}/pulls/{number}`):
- Fetch PR metadata: title, state, author, created, updated, body, additions/deletions, changed files count
- Fetch reviews: `gh api repos/{owner}/{repo}/pulls/{number}/reviews`
- Fetch review comments: `gh api repos/{owner}/{repo}/pulls/{number}/comments`
- Fetch CI status: `gh api repos/{owner}/{repo}/commits/{head_sha}/check-runs`

**Issue detail** (`gh api repos/{owner}/{repo}/issues/{number}`):
- Fetch issue metadata: title, state, author, labels, assignees, body
- Fetch comments: `gh api repos/{owner}/{repo}/issues/{number}/comments`

**Markdown output for PR**:
```markdown
# org/repo#123: PR Title

| Field | Value |
|---|---|
| Status | Open |
| Author | username |
| Branch | feature-branch → main |
| Created | 2026-04-01 |
| Updated | 2026-04-02 |
| Changes | +150 -30 across 5 files |

## Description

Full PR body...

## Reviews (N)

### @reviewer1 — APPROVED — 2026-04-02
Looks good, ship it.

### @reviewer2 — CHANGES_REQUESTED — 2026-04-01
Need to handle the edge case in line 42.

## Review Comments (N)

### @reviewer2 on src/main.rs:42 — 2026-04-01
This could panic if the input is empty.

## CI Status

- [x] build (passed)
- [ ] lint (failed)
- [x] test (passed)
```

**Markdown output for Issue**:
```markdown
# org/repo#456: Issue Title

| Field | Value |
|---|---|
| Status | Open |
| Author | username |
| Labels | bug, backend |
| Assignees | user1, user2 |
| Created | 2026-04-01 |

## Description

Full issue body...

## Comments (N)

### @commenter — 2026-04-01
Comment text...
```

### 1.6 Auto-Detection (resolve.rs additions)

| Priority | Pattern | Result |
|---|---|---|
| New-1 | `https://github.com/{owner}/{repo}/pull/{number}` | `GitHubPR { owner, repo, number }` |
| New-2 | `https://github.com/{owner}/{repo}/issues/{number}` | `GitHubIssue { owner, repo, number }` |
| New-3 | `{repo}#{number}` (e.g., `product-amls#123`) — only when `source="github"` is explicit | `GitHubShorthand { repo, number }` |

The shorthand `repo#N` is ambiguous (could be a Markdown heading), so it only triggers with explicit `source="github"` via `force_source`.

GitHub URLs are unambiguous and auto-detect without explicit source.

Add to `ParsedIdentifier` enum:
```rust
GitHubPR { owner: String, repo: String, number: u64 },
GitHubIssue { owner: String, repo: String, number: u64 },
GitHubShorthand { repo: String, number: u64 },
```

Add `GitHub` variant to `SourceType` enum.

### 1.7 Error Handling

| Scenario | Behavior |
|---|---|
| `gh` not installed | Health check: Unavailable with "gh CLI not found in PATH" |
| `gh auth` expired | Health check: Unavailable with "not authenticated" |
| Rate limited (403) | `SearchError::RateLimited` with retry hint |
| Repo not found | `SearchError::Source` with "repository not found" |
| `gh` timeout (>10s) | Kill subprocess, return `SearchError::Source` with "timeout" |

---

## 2. Response Caching

### 2.1 Cache Structure (`src/cache.rs`)

```rust
pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
}

struct CacheEntry {
    response: UnifiedSearchResponse,
    created_at: Instant,
    last_accessed: Instant,
}
```

### 2.2 Cache Key

Normalized key = `lowercase(query_text) + "|" + sorted(source_names).join(",")`.

Examples:
- `unified_search("Broadcast threshold")` → `"broadcast threshold|confluence,github,jira,local_text,slack"`
- `search_source("slack", "deploy")` → `"deploy|slack"`

### 2.3 TTL and Eviction

- **TTL**: configurable via `cache_ttl_seconds` in `config.yaml` (default 300 = 5min). Set to 0 to disable caching entirely.
- **Max entries**: 100 (hardcoded, not worth configuring).
- **Eviction**: when cache is full, remove the entry with the oldest `last_accessed` timestamp.
- **Expiry**: checked on read. If entry exists but `created_at + ttl < now`, treat as cache miss and remove the entry.

### 2.4 Config

```yaml
server:
  cache_ttl_seconds: 300   # default 5min, 0 = disabled
```

### 2.5 Bypass Parameter

Add `no_cache: Option<bool>` (default false) to `UnifiedSearchParams` and `SearchSourceParams` in `mcp.rs`. When `true`:
- Skip cache lookup
- Execute query normally
- Store result in cache (overwriting any existing entry for that key)

### 2.6 What Is Cached

| Tool | Cached? |
|---|---|
| `unified_search` | Yes |
| `search_source` | Yes |
| `get_detail` | No — always fresh |
| `list_sources` | No |

### 2.7 Integration Point

Cache lives in `SearchOrchestrator` as `Option<ResponseCache>`:
- `None` when `cache_ttl_seconds = 0`
- Check cache before fan-out in `search()` method
- Write to cache after dedup/truncation
- Thread-safe: wrap in `Arc<Mutex<ResponseCache>>` since the orchestrator is shared across tool calls

### 2.8 Metrics

Cache hits/misses are tracked in the response:
- Add `cache_hit: bool` field to `UnifiedSearchResponse`
- Metrics logger records cache hits in the JSONL entry
- `--stats` report shows cache hit rate

---

## 3. Confluence Body→Markdown

### 3.1 Module: `src/sources/confluence_markdown.rs`

Single public function:
```rust
pub fn to_markdown(html: &str) -> String
```

### 3.2 Conversion Table

| HTML / Confluence Tag | Markdown Output |
|---|---|
| `<h1>`–`<h6>` | `#`–`######` with blank lines |
| `<p>` | Paragraph with blank line separator |
| `<strong>`, `<b>` | `**bold**` |
| `<em>`, `<i>` | `*italic*` |
| `<u>` | `<u>underline</u>` (no Markdown equivalent, pass through) |
| `<s>`, `<del>` | `~~strikethrough~~` |
| `<a href="url">text</a>` | `[text](url)` |
| `<ul>` / `<li>` | `- item` (nested with 2-space indent per level) |
| `<ol>` / `<li>` | `1. item` (nested with 3-space indent per level) |
| `<code>` (inline) | `` `code` `` |
| `<pre>` | Fenced code block (``` ``` ```) |
| `<ac:structured-macro ac:name="code">` | Fenced code block with language from `ac:parameter[@ac:name="language"]` |
| `<table>` / `<tr>` / `<th>` / `<td>` | Markdown table with `\|` separators and `\|---\|` header row |
| `<img src="url" alt="text">` | `![text](url)` |
| `<ac:image>` with `<ri:url ri:value="...">` | `![](url)` |
| `<ac:structured-macro ac:name="info">` | `> **Info:** content` |
| `<ac:structured-macro ac:name="warning">` | `> **Warning:** content` |
| `<ac:structured-macro ac:name="note">` | `> **Note:** content` |
| `<ac:structured-macro ac:name="tip">` | `> **Tip:** content` |
| `<ac:structured-macro ac:name="expand">` | `<details><summary>title</summary>\n\ncontent\n\n</details>` |
| `<ac:link>` with `<ri:page ri:content-title="...">` | `[page title]` (no URL available from storage format) |
| `<ac:emoticon ac:name="...">` | Strip (not useful in Markdown) |
| `<br>` / `<br/>` | Newline |
| `<hr>` | `---` |
| Unknown tags | Strip tag, keep inner text content |

### 3.3 Implementation Approach

**Lightweight XML/HTML parser**: Since Confluence storage format is well-structured XHTML (not arbitrary browser HTML), use a simple tag-by-tag state machine:

1. Tokenize input into: `OpenTag(name, attrs)`, `CloseTag(name)`, `SelfClosingTag(name, attrs)`, `Text(content)`
2. Walk tokens maintaining a stack of open tags
3. On each token, check the tag name against the conversion table
4. Track nesting depth for lists (to compute indentation)
5. Track whether we're inside a table (to buffer rows and emit the header separator after the first row)

**No external HTML parsing crate** — the input is controlled (Confluence storage format), and a full HTML parser (like `scraper` or `html5ever`) would add significant dependencies for a narrow use case.

### 3.4 Integration

- `ConfluenceSource::get_detail_page()` calls `confluence_markdown::to_markdown(body_html)` instead of `self.strip_html(body_html)` for the `## Content` section
- Comment bodies in `get_detail_page()` also use `to_markdown()` for richer output
- Search result snippets continue using `strip_html()` — snippets should be plain text for the Markdown table
- Comment enrichment in search results (Task 3 from v0.2) continues using `strip_html()` for the same reason

### 3.5 Edge Cases

- **Empty input**: return empty string
- **Nested tables**: Markdown doesn't support nested tables. Inner table rendered as plain text with ` | ` separators.
- **Multi-paragraph table cells**: Join with `<br>` in the Markdown cell
- **Images with relative URLs**: Prefix with Confluence base URL from config
- **Malformed HTML**: Best-effort — unmatched close tags are ignored, unclosed tags are auto-closed at end

---

## 4. Files Changed

| File | Change | New/Modified |
|---|---|---|
| `src/sources/github.rs` | GitHub source — search via `gh` CLI, get_detail for PRs/issues | **New** |
| `src/sources/confluence_markdown.rs` | Confluence storage format → Markdown converter | **New** |
| `src/cache.rs` | LRU response cache with TTL | **New** |
| `src/sources/mod.rs` | Add `pub mod github; pub mod confluence_markdown;` | Modified |
| `src/resolve.rs` | Add `GitHub` to `SourceType`, GitHub URL + shorthand patterns | Modified |
| `src/config.rs` | Add `GitHubSourceConfig`, `cache_ttl_seconds` | Modified |
| `src/core.rs` | Wire cache into orchestrator (check before fan-out, write after dedup) | Modified |
| `src/mcp.rs` | Add `no_cache` param to search tool params | Modified |
| `src/server.rs` | Pass `no_cache` to orchestrator, add GitHub to `get_detail` dispatch | Modified |
| `src/main.rs` | Wire GitHub source, cache initialization | Modified |
| `src/sources/confluence.rs` | Use `confluence_markdown::to_markdown()` in `get_detail_page` | Modified |
| `src/models.rs` | Add `cache_hit: bool` to `UnifiedSearchResponse` | Modified |
| `src/lib.rs` | Add `pub mod cache;` | Modified |
| `tests/test_github.rs` | Tests for GitHub search, get_detail, health check, error handling | **New** |
| `tests/test_cache.rs` | Tests for LRU eviction, TTL expiry, bypass, cache key normalization | **New** |
| `tests/test_confluence_markdown.rs` | Tests for each conversion rule, edge cases | **New** |
| `tests/test_resolve.rs` | Add tests for GitHub URL and shorthand detection | Modified |
| `tests/test_server.rs` | Add tests for GitHub get_detail dispatch, no_cache parameter | Modified |
| `tests/test_core.rs` | Add tests for cache integration in orchestrator | Modified |

## 5. Out of Scope

- GitHub REST API via reqwest (future — when `gh` CLI dependency is unacceptable)
- Per-source TTLs (not worth the config complexity)
- Disk-persistent cache (in-memory is fine for MCP server lifecycle)
- Query history, export, cross-reference detection (Spec B)
- Confluence `<ac:task-list>` rendering (rare, low value)

## 6. Testing Strategy

- **GitHub source**: mock `gh` CLI by setting `PATH` to a directory with a fake `gh` script that returns fixture JSON. Test search, get_detail (PR + issue), health check, rate limiting, timeout.
- **Cache**: unit tests with deterministic timestamps. Test hit, miss, expiry, eviction, bypass, key normalization, disabled (ttl=0).
- **Confluence Markdown**: pure function tests with HTML input → expected Markdown output. One test per conversion rule. Edge case tests for nested lists, nested tables, malformed HTML, empty input.
