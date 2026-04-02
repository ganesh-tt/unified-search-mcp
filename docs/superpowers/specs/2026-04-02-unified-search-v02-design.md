# Unified Search MCP v0.2 — Rich Content, Detail Tool, Metrics

**Date**: 2026-04-02
**Status**: Approved
**Scope**: Three enhancements to unified-search-mcp

## Goals

1. **Comments always in search results** — JIRA and Confluence search results include comments by default, eliminating the primary reason Claude falls back to individual MCPs for context.
2. **`get_detail` tool** — A new MCP tool for deep-dive lookups by identifier (JIRA key, Confluence URL, Slack permalink). Returns full content including comments, child pages, linked issues, thread replies.
3. **Metrics system** — MCP-side telemetry (call counts, latency, errors per source) plus a CLI adoption analyzer that compares unified-search usage against individual MCP tool bypasses.

## Performance Constraint

Unified search must beat the sequential alternative: Claude making 3-4 individual `jira_get` + `conf_get` calls. The bar is **total wall-clock for one unified_search call < sum of sequential individual MCP calls**, not faster than a single `jira_get`.

Strategy: parallel fan-out for comment enrichment within the existing per-source timeout.

---

## 1. Comments Always in Search Results

### 1.1 JIRA Comment Enrichment

**Current state**: `jira.rs` requests `fields=summary,description,comment,status,updated,assignee` but the `comment` field content is never extracted from the response.

**Step 1**: Check if the JIRA search API response already includes comments inline in the `fields.comment` object. The v3 search API returns comments when `comment` is in the fields list — each issue's `fields.comment.comments` is an array of comment objects.

**Step 2**: If comments are present in the search response (likely — the field is already requested), extract them directly. No extra API calls needed.

**Step 3**: If comments are NOT included or are truncated (JIRA sometimes limits to the most recent 20), fan-out parallel `GET /rest/api/3/issue/{key}/comment?maxResults=50&orderBy=-created` calls for each result using `tokio::spawn`, bounded by the existing per-source timeout.

**Enrichment output**:
- `metadata["comment_count"]` — total comment count as string
- Append to `snippet`: the latest 3 comments formatted as:
  ```
  ---
  Comments (N total):
  [Author, 2026-04-01]: First 150 chars of comment body...
  [Author, 2026-03-28]: First 150 chars of comment body...
  ```
- Comment body extracted from ADF format using existing `extract_adf_text()` helper

**Failure handling**: If comment fetch fails for a specific issue, that result still returns with its original snippet — no degradation to core search.

### 1.2 Confluence Comment Enrichment

**Current state**: `confluence.rs` uses CQL search API returning only title + excerpt. No comments fetched.

**After search returns pages**, fan-out parallel requests:
```
GET /wiki/rest/api/content/{id}/child/comment?expand=body.storage,version&limit=25
```

For each page result:
- Extract comment author (`version.by.displayName`), timestamp (`version.when`), body (`body.storage.value` run through existing `strip_html()`)
- Add `metadata["comment_count"]`
- Append latest 3 comments to snippet (same format as JIRA)

**Extracting page ID**: The CQL search response includes `content.id` — already deserialized in `ConfluenceContent.id` (currently `#[allow(dead_code)]`). Remove the dead_code annotation and use it.

**Failure handling**: Same as JIRA — comment fetch failure doesn't block the result.

### 1.3 Performance Budget

| Operation | Expected latency |
|---|---|
| JIRA search (existing) | ~200ms |
| Comment extraction from search response (no extra call) | ~0ms |
| JIRA comment fan-out (if needed, 10 parallel) | ~250ms additional |
| Confluence search (existing) | ~400ms |
| Confluence comment fan-out (10 parallel) | ~300ms additional |

**Worst case** for enriched search: ~700ms per source, running in parallel across sources. Total wall-clock stays under 1s. This beats 3-4 sequential individual MCP calls (3x300ms = 900ms+).

---

## 2. New `get_detail` Tool

### 2.1 MCP Tool Registration

```rust
#[tool(description = "Fetch full details for a specific JIRA ticket, Confluence page, or Slack thread. \
    Accepts a JIRA key (FIN-1234), a JIRA/Confluence/Slack URL, or a Confluence page title. \
    Returns full content: description, all comments (with replies), linked issues, subtasks, \
    child pages, or thread replies depending on source. Use this when you need complete context \
    about a specific item found via unified_search.")]
async fn get_detail(
    &self,
    Parameters(params): Parameters<GetDetailParams>,
) -> String
```

**Parameters**:
```rust
struct GetDetailParams {
    /// The identifier: a JIRA key (FIN-1234), URL, or Confluence page title
    identifier: String,
    /// Optional: force source type ('jira', 'confluence', 'slack'). If omitted, auto-detected.
    source: Option<String>,
    /// Optional: max comments to return (default: all)
    max_comments: Option<usize>,
}
```

### 2.2 Auto-Detection Logic (`src/resolve.rs`)

New module with a `detect_source(identifier: &str) -> Option<(SourceType, ParsedIdentifier)>` function.

```rust
enum SourceType { Jira, Confluence, Slack }

enum ParsedIdentifier {
    JiraKey(String),                        // "FIN-1234"
    JiraUrl { base_url: String, key: String }, // parsed from URL
    ConfluencePageId(String),               // extracted from URL
    ConfluenceTitle { title: String, space: Option<String> },
    SlackPermalink { channel: String, ts: String }, // parsed from archive URL
}
```

**Detection rules (applied in order)**:

| Priority | Pattern | Result |
|---|---|---|
| 1 | `https://*.atlassian.net/browse/{KEY}` | `JiraUrl` |
| 2 | `https://*.atlassian.net/wiki/spaces/*/pages/{id}/*` | `ConfluencePageId` |
| 3 | `https://*.slack.com/archives/{channel}/p{ts}` | `SlackPermalink` |
| 4 | `^[A-Z][A-Z0-9]+-\d+$` (regex) | `JiraKey` |
| 5 | Anything else + `source=confluence` | `ConfluenceTitle` |

If `source` parameter is provided, skip auto-detection and interpret the identifier directly for that source type.

### 2.3 What Each Source Fetches

#### JIRA `get_detail`

**API call**:
```
GET /rest/api/3/issue/{key}?fields=summary,description,status,assignee,reporter,labels,
    fixVersions,issuelinks,subtasks,comment,priority,issuetype,created,updated
    &expand=renderedFields
```

**Response structure** (Markdown output):
```markdown
# FIN-1234: Ticket summary here

| Field | Value |
|---|---|
| Status | In Progress |
| Type | Story |
| Priority | High |
| Assignee | John Doe |
| Reporter | Jane Smith |
| Labels | backend, auth |
| Fix Versions | v6.3.4 |
| Created | 2026-03-15 |
| Updated | 2026-04-01 |

## Description

Full description text converted from ADF...

## Linked Issues

- **blocks** FIN-1235: Other ticket summary (In Progress)
- **is blocked by** FIN-1200: Dependency ticket (Done)

## Subtasks

- [x] FIN-1234-1: Subtask one (Done)
- [ ] FIN-1234-2: Subtask two (In Progress)

## Comments (N)

### Author Name — 2026-04-01 14:30 UTC
Full comment body here...

### Another Author — 2026-03-28 10:15 UTC
Full comment body here...
```

#### Confluence `get_detail`

**API calls**:
```
GET /wiki/rest/api/content/{id}?expand=body.storage,version,children.page,
    children.comment.body.storage,metadata.labels,space
```

If identifier is a title (not ID), first resolve via:
```
GET /wiki/rest/api/content?title={title}&spaceKey={space}&expand=...
```

**Response structure** (Markdown output):
```markdown
# Page Title

| Field | Value |
|---|---|
| Space | PROD |
| Author | John Doe |
| Last Updated | 2026-04-01 |
| Labels | architecture, decisions |

## Content

Full page body converted from HTML (strip tags)...

## Child Pages

- [Child Page One](/wiki/spaces/PROD/pages/111) — Last updated 2026-03-20
- [Child Page Two](/wiki/spaces/PROD/pages/222) — Last updated 2026-03-15

## Comments (N)

### Author Name — 2026-04-01 14:30 UTC
Full comment body...

### Another Author — 2026-03-28 10:15 UTC
Full comment body...
```

#### Slack `get_detail`

**Permalink parsing**:
```
https://tookitaki.slack.com/archives/C06ABC123/p1712000000123456
                                      ^^^^^^^^  ^^^^^^^^^^^^^^^^
                                      channel   ts (insert dot before last 6 digits)
```

`p1712000000123456` → `1712000000.123456`

**API calls**:
```
GET conversations.history?channel={channel}&latest={ts}&oldest={ts}&inclusive=true&limit=1
GET conversations.replies?channel={channel}&ts={ts}&limit=200
GET conversations.info?channel={channel}
```

**Response structure** (Markdown output):
```markdown
# Slack Thread in #channel-name

**Started by**: @username — 2026-04-01 14:30 UTC

## Original Message

Full message text here...

## Thread Replies (N)

### @author1 — 2026-04-01 14:35 UTC
Reply text...

### @author2 — 2026-04-01 14:40 UTC
Reply text...

## Participants

@user1, @user2, @user3
```

### 2.4 Error Handling

| Scenario | Behavior |
|---|---|
| Identifier not recognized + no `source` | Return error: "Could not detect source type. Provide a `source` parameter ('jira', 'confluence', 'slack')." |
| JIRA key not found (404) | Return error: "JIRA issue {key} not found." |
| Confluence page ID not found | Return error: "Confluence page {id} not found." |
| Slack channel not accessible | Return error: "Cannot access Slack channel {channel}. Bot may not be a member." |
| Auth failure | Delegate to existing `SearchError::Auth` handling |
| Timeout | Per-source timeout applies (configurable, default 10s) |

---

## 3. Metrics System

### 3.1 MCP-Side Telemetry (`src/metrics.rs`)

**Data model**: One JSON line per tool call, appended to `~/.unified-search/metrics.jsonl`:

```json
{
  "ts": "2026-04-02T14:30:00.123Z",
  "tool": "unified_search",
  "query": "broadcast threshold decision",
  "sources_queried": ["slack", "jira", "confluence", "local_text"],
  "per_source": {
    "slack": { "latency_ms": 320, "results": 5, "comments_fetched": 12, "error": null },
    "jira": { "latency_ms": 180, "results": 8, "comments_fetched": 24, "error": null },
    "confluence": { "latency_ms": 450, "results": 3, "comments_fetched": 6, "error": null },
    "local_text": { "latency_ms": 45, "results": 3, "comments_fetched": 0, "error": null }
  },
  "total_results": 19,
  "deduped_results": 16,
  "total_ms": 460
}
```

For `get_detail` calls:
```json
{
  "ts": "2026-04-02T14:31:00.456Z",
  "tool": "get_detail",
  "identifier": "FIN-1234",
  "detected_source": "jira",
  "explicit_source": null,
  "latency_ms": 350,
  "comments_returned": 15,
  "linked_issues": 3,
  "error": null
}
```

**Implementation**:
- `MetricsLogger` struct with `log(&self, entry: MetricsEntry)` method
- Writes are fire-and-forget via `tokio::spawn` — zero impact on response latency
- File rotation: when file exceeds 10MB, rename to `metrics.jsonl.1` (keep only 1 backup), start fresh
- Graceful failure: if write fails (permissions, disk full), log to stderr and continue

### 3.2 Adoption Analysis CLI (`--stats`)

Invoked as: `unified-search-mcp --stats [--days N]` (default: 7 days)

**Data sources**:
1. `~/.unified-search/metrics.jsonl` — own call history (authoritative)
2. `~/.claude/projects/*/conversations/*.jsonl` — Claude Code conversation logs

**Claude Code conversation log parsing**:
- Scan for tool_use blocks where tool name matches: `jira_get`, `jira_post`, `jira_put`, `jira_delete`, `conf_get`, `mcp__claude_ai_Slack__*`
- These represent "bypasses" — cases where Claude chose an individual MCP over unified-search
- Filter by timestamp within the `--days` window
- Exclude write operations (`jira_post`, `jira_put`, `jira_delete`) from bypass count — those are not search-like and unified-search doesn't do writes

**Report output**:
```
=== Unified Search Adoption Report (last 7 days) ===

Tool Calls:
  unified_search:  45 calls  (avg 420ms, p50 380ms, p95 890ms)
  search_source:   12 calls  (avg 280ms, p50 250ms, p95 650ms)
  get_detail:       8 calls  (avg 350ms, p50 310ms, p95 700ms)

Bypasses (Claude used individual MCPs for search/read):
  jira_get:         6 calls
  conf_get:         3 calls
  Slack MCP:        1 call

Adoption Rate: 88% (65 unified / 74 total search-like operations)

Per-Source Performance:
  slack:       98% success | avg 310ms | p95 620ms | 5.2 results/query
  jira:       100% success | avg 190ms | p95 380ms | 7.8 results/query
  confluence:  95% success | avg 440ms | p95 810ms | 3.1 results/query
  local_text: 100% success | avg  45ms | p95  90ms | 2.4 results/query

Comment Enrichment:
  Total comments fetched: 412
  Avg comments per JIRA result: 3.1
  Avg comments per Confluence result: 2.0

Top Queries (by frequency):
  1. "broadcast threshold" (7 calls)
  2. "FIN-10384" (5 calls)
  3. "auth middleware" (4 calls)
```

**Claude Code log format assumption**: Claude Code stores conversations as JSONL with tool_use entries containing `name` and `input` fields. If the log format changes or is inaccessible, the stats command gracefully reports "Claude Code logs not found — showing MCP metrics only" and omits the bypass/adoption section.

### 3.3 Metrics in Response Footer

Enhance the existing response footer to include per-source breakdown:

**Current**:
```
**Sources queried**: 4 | **Time**: 460ms
```

**New**:
```
**Sources**: slack (320ms, 5 results, 12 comments) | jira (180ms, 8 results, 24 comments) | confluence (450ms, 3 results, 6 comments) | local (45ms, 3 results) | **Total**: 460ms
```

---

## 4. Files Changed

| File | Change | New/Modified |
|---|---|---|
| `src/sources/jira.rs` | Extract comments from search response, parallel comment enrichment, `get_detail_jira()` method | Modified |
| `src/sources/confluence.rs` | Parallel comment fetch after search, `get_detail_confluence()` method | Modified |
| `src/sources/slack.rs` | Permalink parsing, `conversations.history`/`replies` calls, `get_detail_slack()` method | Modified |
| `src/resolve.rs` | URL/identifier auto-detection, `get_detail` orchestration | **New** |
| `src/metrics.rs` | JSONL append logger, file rotation, `MetricsLogger` struct | **New** |
| `src/stats.rs` | Adoption analysis CLI — reads metrics.jsonl + Claude Code logs, produces report | **New** |
| `src/models.rs` | Add `DetailResult` enum, `PerSourceMetrics` struct, extend `SearchResult` metadata | Modified |
| `src/core.rs` | Pass per-source timing/result-counts back in `UnifiedSearchResponse` | Modified |
| `src/mcp.rs` | Register `get_detail` tool, wire `MetricsLogger` into all tool handlers | Modified |
| `src/server.rs` | Add `handle_get_detail`, emit metrics after each handler, enhanced response footer | Modified |
| `src/main.rs` | Add `--stats` flag routing, initialize `MetricsLogger` | Modified |
| `src/config.rs` | Add optional `metrics_path` config field (default `~/.unified-search/metrics.jsonl`) | Modified |
| `src/lib.rs` | Add `mod resolve; mod metrics; mod stats;` declarations | Modified |

## 5. Out of Scope

- Vector/semantic search (Phase 2 stub unchanged)
- Write operations (no JIRA/Confluence create/update)
- Full HTML-to-Markdown conversion for Confluence (uses HTML tag stripping — good enough for v0.2)
- Response caching
- Slack user ID → display name resolution (uses whatever the API returns)

## 6. Testing Strategy

- Extend existing `wiremock` tests for JIRA/Confluence to include comment responses
- Add `wiremock` tests for `get_detail` per source type
- Add unit tests for `resolve.rs` auto-detection with edge cases (malformed URLs, ambiguous identifiers)
- Add unit tests for `metrics.rs` (JSONL serialization, rotation logic)
- Add integration test for `--stats` with fixture data
