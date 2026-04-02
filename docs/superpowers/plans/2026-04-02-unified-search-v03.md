# Unified Search MCP v0.3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add GitHub source (via `gh` CLI), in-memory response caching with TTL, and full-fidelity Confluence body→Markdown conversion.

**Architecture:** GitHub source follows the same `SearchSource` trait pattern as existing sources but uses `tokio::process::Command` to shell out to `gh` instead of reqwest HTTP. Cache wraps the orchestrator's `search()` method with an LRU HashMap. Confluence Markdown is a pure-function module that replaces `strip_html()` in `get_detail_page`.

**Tech Stack:** Rust, tokio, serde_json, regex, `gh` CLI (subprocess)

---

## File Structure

| File | Responsibility | New/Modified |
|---|---|---|
| `src/sources/github.rs` | GitHub search + get_detail via `gh` CLI | **New** |
| `src/sources/confluence_markdown.rs` | Confluence storage format → Markdown converter | **New** |
| `src/cache.rs` | In-memory LRU response cache with TTL | **New** |
| `src/sources/mod.rs` | Add `pub mod github; pub mod confluence_markdown;` | Modified |
| `src/resolve.rs` | Add `GitHub` variant, GitHub URL patterns | Modified |
| `src/config.rs` | Add `GitHubSourceConfig`, `cache_ttl_seconds` | Modified |
| `src/models.rs` | Add `cache_hit: bool` to `UnifiedSearchResponse` | Modified |
| `src/core.rs` | Wire cache into orchestrator | Modified |
| `src/mcp.rs` | Add `no_cache` param to search tools | Modified |
| `src/server.rs` | Add GitHub to `get_detail`, pass `no_cache`, pass `cache_hit` to footer | Modified |
| `src/main.rs` | Wire GitHub source, cache init | Modified |
| `src/sources/confluence.rs` | Use `confluence_markdown::to_markdown()` in `get_detail_page` | Modified |
| `src/lib.rs` | Add `pub mod cache;` | Modified |
| `tests/test_github.rs` | GitHub search, get_detail, health check tests | **New** |
| `tests/test_cache.rs` | Cache hit/miss/expiry/eviction/bypass tests | **New** |
| `tests/test_confluence_markdown.rs` | Conversion rule tests | **New** |
| `tests/test_resolve.rs` | GitHub URL + shorthand detection tests | Modified |

---

## Task 1: Confluence Markdown Converter

Pure function, no dependencies on other tasks. Start here.

**Files:**
- Create: `src/sources/confluence_markdown.rs`
- Create: `tests/test_confluence_markdown.rs`
- Modify: `src/sources/mod.rs`

- [ ] **Step 1: Add module declaration**

In `src/sources/mod.rs`, add `pub mod confluence_markdown;` after the existing module declarations.

- [ ] **Step 2: Write failing tests**

Create `tests/test_confluence_markdown.rs`:

```rust
use unified_search_mcp::sources::confluence_markdown::to_markdown;

#[test]
fn converts_headings() {
    assert_eq!(to_markdown("<h1>Title</h1>"), "# Title\n");
    assert_eq!(to_markdown("<h2>Sub</h2>"), "## Sub\n");
    assert_eq!(to_markdown("<h3>Deep</h3>"), "### Deep\n");
}

#[test]
fn converts_paragraphs() {
    assert_eq!(to_markdown("<p>Hello world</p>"), "Hello world\n\n");
    assert_eq!(
        to_markdown("<p>First</p><p>Second</p>"),
        "First\n\nSecond\n\n"
    );
}

#[test]
fn converts_bold_italic() {
    assert_eq!(to_markdown("<strong>bold</strong>"), "**bold**");
    assert_eq!(to_markdown("<b>bold</b>"), "**bold**");
    assert_eq!(to_markdown("<em>italic</em>"), "*italic*");
    assert_eq!(to_markdown("<i>italic</i>"), "*italic*");
}

#[test]
fn converts_links() {
    assert_eq!(
        to_markdown(r#"<a href="https://example.com">click</a>"#),
        "[click](https://example.com)"
    );
}

#[test]
fn converts_unordered_list() {
    let html = "<ul><li>one</li><li>two</li><li>three</li></ul>";
    let expected = "- one\n- two\n- three\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_ordered_list() {
    let html = "<ol><li>first</li><li>second</li></ol>";
    let expected = "1. first\n2. second\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_nested_list() {
    let html = "<ul><li>top<ul><li>nested</li></ul></li></ul>";
    let expected = "- top\n  - nested\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_inline_code() {
    assert_eq!(to_markdown("<code>let x = 1;</code>"), "`let x = 1;`");
}

#[test]
fn converts_code_block() {
    let html = "<pre>fn main() {\n    println!(\"hello\");\n}</pre>";
    let expected = "```\nfn main() {\n    println!(\"hello\");\n}\n```\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_confluence_code_macro() {
    let html = r#"<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">rust</ac:parameter><ac:plain-text-body><![CDATA[fn main() {}]]></ac:plain-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(result.contains("```rust"), "Should have language hint");
    assert!(result.contains("fn main() {}"), "Should have code content");
}

#[test]
fn converts_table() {
    let html = "<table><tr><th>Name</th><th>Age</th></tr><tr><td>Alice</td><td>30</td></tr></table>";
    let result = to_markdown(html);
    assert!(result.contains("| Name | Age |"), "Should have header row");
    assert!(result.contains("|---|---|"), "Should have separator");
    assert!(result.contains("| Alice | 30 |"), "Should have data row");
}

#[test]
fn converts_image() {
    assert_eq!(
        to_markdown(r#"<img src="https://example.com/img.png" alt="photo">"#),
        "![photo](https://example.com/img.png)"
    );
}

#[test]
fn converts_info_macro() {
    let html = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Important note</p></ac:rich-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(result.contains("> **Info:**"), "Should have info blockquote");
    assert!(result.contains("Important note"), "Should have content");
}

#[test]
fn converts_warning_macro() {
    let html = r#"<ac:structured-macro ac:name="warning"><ac:rich-text-body><p>Be careful</p></ac:rich-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(result.contains("> **Warning:**"), "Should have warning blockquote");
}

#[test]
fn converts_hr() {
    assert_eq!(to_markdown("<hr>"), "---\n\n");
    assert_eq!(to_markdown("<hr/>"), "---\n\n");
}

#[test]
fn converts_br() {
    assert_eq!(to_markdown("line1<br>line2"), "line1\nline2");
    assert_eq!(to_markdown("line1<br/>line2"), "line1\nline2");
}

#[test]
fn strips_unknown_tags() {
    assert_eq!(to_markdown("<div>content</div>"), "content");
    assert_eq!(to_markdown("<span class=\"x\">text</span>"), "text");
}

#[test]
fn handles_empty_input() {
    assert_eq!(to_markdown(""), "");
}

#[test]
fn converts_strikethrough() {
    assert_eq!(to_markdown("<del>removed</del>"), "~~removed~~");
    assert_eq!(to_markdown("<s>struck</s>"), "~~struck~~");
}

#[test]
fn converts_expand_macro() {
    let html = r#"<ac:structured-macro ac:name="expand"><ac:parameter ac:name="title">Details</ac:parameter><ac:rich-text-body><p>Hidden content</p></ac:rich-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(result.contains("<details>"), "Should use details tag");
    assert!(result.contains("Details"), "Should have summary title");
    assert!(result.contains("Hidden content"), "Should have body");
}

#[test]
fn plain_text_passthrough() {
    assert_eq!(to_markdown("just plain text"), "just plain text");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test test_confluence_markdown -- --nocapture 2>&1 | head -10`
Expected: compilation error — module doesn't exist

- [ ] **Step 4: Implement `src/sources/confluence_markdown.rs`**

Implement a recursive tag-walker that tokenizes HTML into open/close/self-closing tags and text nodes, then converts each according to the conversion table in the spec. Key implementation notes:

- Tokenizer: iterate through the string looking for `<` to find tags, everything else is text
- Track state: list depth (for indentation), list type stack (ul/ol), inside-table flag, ordered list counters
- For `<ac:structured-macro>`: check `ac:name` attribute to determine macro type (code, info, warning, note, tip, expand)
- For `<ac:parameter>`: extract parameter values (e.g., language for code macro)
- For `<ac:plain-text-body>`: extract CDATA content for code blocks
- For `<ac:rich-text-body>`: recursively convert inner HTML
- Tables: buffer rows, emit header separator after first `<tr>` that contains `<th>`
- Unknown tags: skip the tag, keep processing inner content

The function signature is `pub fn to_markdown(html: &str) -> String`.

This is the most complex single module. Take your time, handle edge cases, and make sure all 21 tests pass.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test test_confluence_markdown -- --nocapture 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/sources/confluence_markdown.rs src/sources/mod.rs tests/test_confluence_markdown.rs
git commit -m "feat: add full-fidelity Confluence storage format to Markdown converter"
```

---

## Task 2: Integrate Confluence Markdown into get_detail_page

**Files:**
- Modify: `src/sources/confluence.rs`

- [ ] **Step 1: Write failing test**

Add to `tests/test_confluence.rs`:

```rust
#[tokio::test]
async fn get_detail_page_preserves_markdown_structure() {
    let server = MockServer::start().await;

    // Create a fixture with rich HTML content
    let body = r#"{
        "id": "99999",
        "type": "page",
        "title": "Rich Content Page",
        "space": {"key": "TEST", "name": "Test Space"},
        "body": {
            "storage": {
                "value": "<h2>Overview</h2><p>This page has <strong>bold</strong> and <em>italic</em> text.</p><ul><li>Item one</li><li>Item two</li></ul><table><tr><th>Col A</th><th>Col B</th></tr><tr><td>1</td><td>2</td></tr></table>",
                "representation": "storage"
            }
        },
        "version": {"by": {"displayName": "Test User"}, "when": "2026-04-01T10:00:00.000Z", "number": 1},
        "children": {"page": {"results": [], "size": 0}, "comment": {"results": [], "size": 0}},
        "metadata": {"labels": {"results": []}},
        "_links": {"webui": "/spaces/TEST/pages/99999", "base": "https://test.atlassian.net/wiki"}
    }"#;

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/99999"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let result = source.get_detail_page("99999").await.unwrap();

    // Should have Markdown headings, not stripped text
    assert!(result.contains("## Overview"), "Should convert h2 to ##");
    assert!(result.contains("**bold**"), "Should convert strong to bold");
    assert!(result.contains("*italic*"), "Should convert em to italic");
    assert!(result.contains("- Item one"), "Should convert ul/li to list");
    assert!(result.contains("| Col A | Col B |"), "Should convert table");

    // Should NOT have raw HTML
    assert!(!result.contains("<h2>"), "Should not have raw HTML tags");
    assert!(!result.contains("<strong>"), "Should not have raw HTML tags");
    assert!(!result.contains("<table>"), "Should not have raw HTML tags");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test get_detail_page_preserves_markdown_structure -- --nocapture 2>&1 | head -10`
Expected: FAIL — current `strip_html` removes all formatting

- [ ] **Step 3: Replace `strip_html` with `to_markdown` in `get_detail_page`**

In `src/sources/confluence.rs`, in the `get_detail_page` method, find where `body_html` is converted to text. Change:
```rust
let body_text = self.strip_html(body_html);
```
to:
```rust
let body_text = super::confluence_markdown::to_markdown(body_html);
```

Also do the same for comment bodies in `get_detail_page` — replace `self.strip_html(c_body_html)` with `super::confluence_markdown::to_markdown(c_body_html)`.

- [ ] **Step 4: Run tests**

Run: `cargo test test_confluence -- --nocapture 2>&1 | tail -10`
Expected: all Confluence tests pass

- [ ] **Step 5: Commit**

```bash
git add src/sources/confluence.rs tests/test_confluence.rs
git commit -m "feat: use Markdown converter in Confluence get_detail_page"
```

---

## Task 3: Response Cache Module

**Files:**
- Create: `src/cache.rs`
- Create: `tests/test_cache.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add `pub mod cache;` to `src/lib.rs`**

- [ ] **Step 2: Write failing tests**

Create `tests/test_cache.rs`:

```rust
use std::time::Duration;
use unified_search_mcp::cache::ResponseCache;
use unified_search_mcp::models::{UnifiedSearchResponse, PerSourceStats};

fn make_response(n_results: usize) -> UnifiedSearchResponse {
    UnifiedSearchResponse {
        results: (0..n_results)
            .map(|i| unified_search_mcp::models::SearchResult {
                source: "test".to_string(),
                title: format!("Result {}", i),
                snippet: format!("Snippet {}", i),
                url: Some(format!("https://example.com/{}", i)),
                timestamp: None,
                relevance: 1.0 - (i as f32 * 0.1),
                metadata: std::collections::HashMap::new(),
            })
            .collect(),
        warnings: vec![],
        total_sources_queried: 1,
        query_time_ms: 100,
        per_source_stats: vec![],
        cache_hit: false,
    }
}

#[test]
fn cache_miss_then_hit() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(300));

    // Miss
    assert!(cache.get("test query", &["slack"]).is_none());

    // Store
    let response = make_response(3);
    cache.put("test query", &["slack"], response.clone());

    // Hit
    let cached = cache.get("test query", &["slack"]);
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().results.len(), 3);
}

#[test]
fn cache_key_normalized() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(300));
    let response = make_response(1);

    // Store with one order
    cache.put("Test Query", &["slack", "jira"], response.clone());

    // Hit with different case and source order
    let cached = cache.get("test query", &["jira", "slack"]);
    assert!(cached.is_some(), "Cache key should be case-insensitive and source-order-independent");
}

#[test]
fn cache_ttl_expiry() {
    let mut cache = ResponseCache::new(100, Duration::from_millis(50));
    let response = make_response(1);

    cache.put("query", &["slack"], response);

    // Immediate hit
    assert!(cache.get("query", &["slack"]).is_some());

    // Wait for expiry
    std::thread::sleep(Duration::from_millis(60));

    // Should be expired
    assert!(cache.get("query", &["slack"]).is_none());
}

#[test]
fn cache_eviction_at_max() {
    let mut cache = ResponseCache::new(3, Duration::from_secs(300));

    for i in 0..3 {
        cache.put(&format!("query{}", i), &["slack"], make_response(1));
    }

    // All 3 should be present
    assert!(cache.get("query0", &["slack"]).is_some());
    assert!(cache.get("query1", &["slack"]).is_some());
    assert!(cache.get("query2", &["slack"]).is_some());

    // Adding a 4th should evict the least recently accessed
    // query0 was accessed most recently (by the get above), so query1 or query2 gets evicted
    // Actually: query0 was last accessed, then query1, then query2.
    // The LRU (least recently used) is whichever was accessed longest ago.
    // After the 3 puts: order is query0(oldest), query1, query2(newest)
    // After the 3 gets: order is query0(newest), query1, query2 — wait, gets update access time
    // Let's simplify: just verify count stays at max
    cache.put("query3", &["slack"], make_response(1));

    // One of the originals should be evicted
    let present: usize = (0..=3)
        .filter(|i| cache.get(&format!("query{}", i), &["slack"]).is_some())
        .count();
    assert_eq!(present, 3, "Should have exactly 3 entries after eviction");
}

#[test]
fn cache_disabled_with_zero_ttl() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(0));
    let response = make_response(1);

    cache.put("query", &["slack"], response);

    // Zero TTL means everything expires immediately
    assert!(cache.get("query", &["slack"]).is_none());
}

#[test]
fn different_sources_different_keys() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(300));

    cache.put("query", &["slack"], make_response(1));
    cache.put("query", &["jira"], make_response(2));

    let slack_result = cache.get("query", &["slack"]).unwrap();
    let jira_result = cache.get("query", &["jira"]).unwrap();

    assert_eq!(slack_result.results.len(), 1);
    assert_eq!(jira_result.results.len(), 2);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test test_cache -- --nocapture 2>&1 | head -10`
Expected: compilation error

- [ ] **Step 4: Implement `src/cache.rs`**

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::models::UnifiedSearchResponse;

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

impl ResponseCache {
    pub fn new(max_entries: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            ttl,
        }
    }

    /// Build a normalized cache key from query text and source list.
    fn make_key(query: &str, sources: &[&str]) -> String {
        let normalized_query = query.trim().to_lowercase();
        let mut sorted_sources: Vec<&str> = sources.to_vec();
        sorted_sources.sort();
        format!("{}|{}", normalized_query, sorted_sources.join(","))
    }

    /// Look up a cached response. Returns None on miss or expiry.
    pub fn get(&mut self, query: &str, sources: &[&str]) -> Option<UnifiedSearchResponse> {
        let key = Self::make_key(query, sources);

        // Check if entry exists and is not expired
        let expired = self.entries.get(&key).map_or(false, |entry| {
            self.ttl.as_secs() == 0 || entry.created_at.elapsed() > self.ttl
        });

        if expired {
            self.entries.remove(&key);
            return None;
        }

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_accessed = Instant::now();
            let mut response = entry.response.clone();
            response.cache_hit = true;
            Some(response)
        } else {
            None
        }
    }

    /// Store a response in the cache.
    pub fn put(&mut self, query: &str, sources: &[&str], response: UnifiedSearchResponse) {
        let key = Self::make_key(query, sources);

        // Evict if at capacity
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            self.evict_oldest();
        }

        self.entries.insert(key, CacheEntry {
            response,
            created_at: Instant::now(),
            last_accessed: Instant::now(),
        });
    }

    fn evict_oldest(&mut self) {
        if let Some(oldest_key) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest_key);
        }
    }
}
```

**Important**: This requires adding `cache_hit: bool` to `UnifiedSearchResponse` in `src/models.rs`. Add it with `#[serde(default)]` so existing code compiles:

```rust
pub struct UnifiedSearchResponse {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    pub total_sources_queried: usize,
    pub query_time_ms: u64,
    pub per_source_stats: Vec<PerSourceStats>,
    #[serde(default)]
    pub cache_hit: bool,
}
```

Fix all existing `UnifiedSearchResponse` construction sites to add `cache_hit: false` (in `core.rs` and any test files).

- [ ] **Step 5: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/cache.rs src/lib.rs src/models.rs src/core.rs tests/test_cache.rs tests/test_models.rs
git commit -m "feat: add in-memory LRU response cache with TTL"
```

---

## Task 4: Wire Cache into Orchestrator

**Files:**
- Modify: `src/core.rs`
- Modify: `src/config.rs`
- Modify: `src/main.rs`
- Modify: `src/mcp.rs`
- Modify: `src/server.rs`
- Modify: `tests/test_core.rs`

- [ ] **Step 1: Add `cache_ttl_seconds` to config**

In `src/config.rs`, add to `ServerConfig`:
```rust
pub cache_ttl_seconds: u64,
```

Default: `300` (5 minutes). Add to `Default` impl and `RawServerConfig`.

- [ ] **Step 2: Add `no_cache` to MCP params**

In `src/mcp.rs`, add to both `UnifiedSearchParams` and `SearchSourceParams`:
```rust
    /// Optional: bypass cache and force fresh results (default false)
    #[serde(default)]
    pub no_cache: Option<bool>,
```

Pass `no_cache` through to the server handlers.

- [ ] **Step 3: Add cache to SearchOrchestrator**

In `src/core.rs`, add cache field:
```rust
use std::sync::Mutex;
use crate::cache::ResponseCache;

pub struct SearchOrchestrator {
    sources: Vec<Arc<dyn SearchSource>>,
    config: OrchestratorConfig,
    cache: Option<Mutex<ResponseCache>>,
}
```

Update `new()` to accept `cache_ttl_seconds: u64`:
```rust
pub fn new(sources: Vec<Box<dyn SearchSource>>, config: OrchestratorConfig, cache_ttl_seconds: u64) -> Self {
    let sources = sources.into_iter().map(|s| Arc::from(s)).collect();
    let cache = if cache_ttl_seconds > 0 {
        Some(Mutex::new(ResponseCache::new(100, Duration::from_secs(cache_ttl_seconds))))
    } else {
        None
    };
    Self { sources, config, cache }
}
```

Update `search()` to accept `no_cache: bool` and check cache before fan-out:
```rust
pub async fn search(&self, query: &SearchQuery, no_cache: bool) -> UnifiedSearchResponse {
    // Check cache first (unless bypassed)
    if !no_cache {
        if let Some(ref cache_mutex) = self.cache {
            let source_names: Vec<&str> = /* collect active source names */;
            if let Ok(mut cache) = cache_mutex.lock() {
                if let Some(cached) = cache.get(&query.text, &source_names) {
                    return cached;
                }
            }
        }
    }

    // ... existing fan-out logic ...

    // Store in cache before returning
    if let Some(ref cache_mutex) = self.cache {
        let source_names: Vec<&str> = /* same list */;
        if let Ok(mut cache) = cache_mutex.lock() {
            cache.put(&query.text, &source_names, response.clone());
        }
    }

    response
}
```

- [ ] **Step 4: Update all callers of `SearchOrchestrator::new()` and `search()`**

In `main.rs`: pass `app_config.server.cache_ttl_seconds` to orchestrator constructor.
In `server.rs`: pass `no_cache` through `handle_unified_search` and `handle_search_source`.
In `tests/test_core.rs` and `tests/test_server.rs`: pass `0` for cache_ttl (disabled in tests) and `false` for no_cache.
In `tests/test_integration.rs`: same.

- [ ] **Step 5: Add cache_hit to response footer**

In `src/server.rs`, in `handle_unified_search`, add to the footer:
```rust
if response.cache_hit {
    let _ = write!(md, " | **Cache**: HIT");
}
```

- [ ] **Step 6: Write test for cache integration**

Add to `tests/test_core.rs`:

```rust
#[tokio::test]
async fn cache_returns_cached_results() {
    let source = MockSource::new("slack", vec![make_result("slack", "msg1", 0.9)]);

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source)],
        default_config(),
        300, // 5min TTL
    );

    // First call — cache miss
    let response1 = orchestrator.search(&query("test"), false).await;
    assert_eq!(response1.results.len(), 1);
    assert!(!response1.cache_hit);

    // Second call — cache hit
    let response2 = orchestrator.search(&query("test"), false).await;
    assert_eq!(response2.results.len(), 1);
    assert!(response2.cache_hit);
}

#[tokio::test]
async fn cache_bypass_with_no_cache() {
    let source = MockSource::new("slack", vec![make_result("slack", "msg1", 0.9)]);

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source)],
        default_config(),
        300,
    );

    // First call
    let _ = orchestrator.search(&query("test"), false).await;

    // Second call with no_cache — should NOT be a cache hit
    let response = orchestrator.search(&query("test"), true).await;
    assert!(!response.cache_hit);
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/core.rs src/config.rs src/mcp.rs src/server.rs src/main.rs tests/test_core.rs tests/test_server.rs tests/test_integration.rs
git commit -m "feat: wire response cache into orchestrator with no_cache bypass"
```

---

## Task 5: GitHub Source Config and Types

**Files:**
- Modify: `src/config.rs`
- Create: `src/sources/github.rs` (config + struct only, no impl yet)
- Modify: `src/sources/mod.rs`

- [ ] **Step 1: Add GitHub config types to `src/config.rs`**

Add after existing source configs:

```rust
pub struct GitHubSourceConfig {
    pub enabled: bool,
    pub weight: f32,
    pub config: GitHubConfig,
}
```

Add to `SourcesConfig`:
```rust
pub github: Option<GitHubSourceConfig>,
```

Add raw config types and wire in `build_sources`.

The `GitHubConfig` struct (in `src/sources/github.rs`):
```rust
#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub orgs: Vec<String>,
    pub repos: Vec<String>,
    pub max_results: usize,
}
```

- [ ] **Step 2: Create `src/sources/github.rs` with struct and config**

Create the file with config, struct, and empty `SearchSource` impl (returning empty results). Add `pub mod github;` to `src/sources/mod.rs`.

- [ ] **Step 3: Run `cargo test` to verify compilation**

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/sources/github.rs src/sources/mod.rs
git commit -m "feat: add GitHub source config and skeleton"
```

---

## Task 6: GitHub Search Implementation

**Files:**
- Modify: `src/sources/github.rs`
- Create: `tests/test_github.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/test_github.rs` with tests that mock the `gh` CLI using a fake script. The approach:

1. Create a temp directory with a fake `gh` script that returns fixture JSON
2. Set `PATH` to that directory + original PATH
3. Create `GitHubSource` with the modified PATH
4. Call `search()` and assert results

Tests needed:
- `search_returns_issues_and_prs` — fake `gh` returns JSON with 2 issues/PRs
- `search_returns_code_results` — fake `gh` returns code search JSON
- `health_check_when_authenticated` — fake `gh auth status` returns 0
- `health_check_when_not_authenticated` — fake `gh auth status` returns 1
- `empty_results` — fake `gh` returns empty results

Since mocking CLI subprocesses is complex, an alternative approach: make the `GitHubSource` accept an optional `gh_path: String` config field (default "gh") so tests can point to a script.

- [ ] **Step 2: Implement search in `src/sources/github.rs`**

Use `tokio::process::Command` to run:
- `gh api search/issues -q '.items[]' --jq '...'` with query params
- `gh api search/code -q '.items[]' --jq '...'` with query params

Parse JSON output into `Vec<SearchResult>`.

Handle rate limiting: check stderr for "rate limit" and return `SearchError::RateLimited`.
Handle timeout: use `tokio::time::timeout` around the subprocess.

- [ ] **Step 3: Implement health_check**

Run `gh auth status --hostname github.com`, check exit code.

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Commit**

```bash
git add src/sources/github.rs tests/test_github.rs
git commit -m "feat: implement GitHub search via gh CLI subprocess"
```

---

## Task 7: GitHub get_detail

**Files:**
- Modify: `src/sources/github.rs`
- Modify: `tests/test_github.rs`

- [ ] **Step 1: Write failing test**

Add test `get_detail_pr_returns_full_markdown` using a fake `gh` script that returns PR JSON + reviews JSON + comments JSON.

- [ ] **Step 2: Implement `get_detail_pr` and `get_detail_issue`**

`get_detail_pr(owner, repo, number)`:
- `gh api repos/{owner}/{repo}/pulls/{number}` — metadata
- `gh api repos/{owner}/{repo}/pulls/{number}/reviews` — reviews
- `gh api repos/{owner}/{repo}/pulls/{number}/comments` — line comments
- `gh api repos/{owner}/{repo}/commits/{head_sha}/check-runs` — CI status
- Build Markdown output per spec

`get_detail_issue(owner, repo, number)`:
- `gh api repos/{owner}/{repo}/issues/{number}` — metadata
- `gh api repos/{owner}/{repo}/issues/{number}/comments` — comments
- Build Markdown output per spec

- [ ] **Step 3: Run tests**

- [ ] **Step 4: Commit**

```bash
git add src/sources/github.rs tests/test_github.rs
git commit -m "feat: add get_detail for GitHub PRs and issues"
```

---

## Task 8: GitHub Auto-Detection in resolve.rs

**Files:**
- Modify: `src/resolve.rs`
- Modify: `tests/test_resolve.rs`

- [ ] **Step 1: Write failing tests**

Add to `tests/test_resolve.rs`:

```rust
#[test]
fn detects_github_pr_url() {
    let (st, parsed) = detect_source("https://github.com/tookitaki/product-amls/pull/123").unwrap();
    assert!(matches!(st, SourceType::GitHub));
    match parsed {
        ParsedIdentifier::GitHubPR { owner, repo, number } => {
            assert_eq!(owner, "tookitaki");
            assert_eq!(repo, "product-amls");
            assert_eq!(number, 123);
        }
        other => panic!("Expected GitHubPR, got {:?}", other),
    }
}

#[test]
fn detects_github_issue_url() {
    let (st, parsed) = detect_source("https://github.com/tookitaki/product-amls/issues/456").unwrap();
    assert!(matches!(st, SourceType::GitHub));
    match parsed {
        ParsedIdentifier::GitHubIssue { owner, repo, number } => {
            assert_eq!(owner, "tookitaki");
            assert_eq!(repo, "product-amls");
            assert_eq!(number, 456);
        }
        other => panic!("Expected GitHubIssue, got {:?}", other),
    }
}

#[test]
fn github_shorthand_only_with_force() {
    // Bare "repo#123" should NOT auto-detect (ambiguous)
    assert!(detect_source("product-amls#123").is_none());

    // But with force_source it should work
    let result = force_source("product-amls#123", "github");
    assert!(result.is_some());
    let (st, parsed) = result.unwrap();
    assert!(matches!(st, SourceType::GitHub));
    match parsed {
        ParsedIdentifier::GitHubShorthand { repo, number } => {
            assert_eq!(repo, "product-amls");
            assert_eq!(number, 123);
        }
        other => panic!("Expected GitHubShorthand, got {:?}", other),
    }
}
```

- [ ] **Step 2: Add GitHub variants to enums in `src/resolve.rs`**

Add `GitHub` to `SourceType`. Add `GitHubPR`, `GitHubIssue`, `GitHubShorthand` to `ParsedIdentifier`.

Add detection rules (before the JIRA key pattern, since GitHub URLs are more specific):
- `https://github.com/{owner}/{repo}/pull/{number}` → `GitHubPR`
- `https://github.com/{owner}/{repo}/issues/{number}` → `GitHubIssue`

Add `"github"` case to `force_source`:
- Try auto-detect first
- Fall back to parsing `{repo}#{number}` shorthand

- [ ] **Step 3: Run tests**

- [ ] **Step 4: Commit**

```bash
git add src/resolve.rs tests/test_resolve.rs
git commit -m "feat: add GitHub URL and shorthand detection to resolve.rs"
```

---

## Task 9: Wire GitHub into Server and Main

**Files:**
- Modify: `src/server.rs`
- Modify: `src/main.rs`
- Modify: `src/mcp.rs` (update instructions)
- Modify: `tests/test_server.rs`

- [ ] **Step 1: Add GitHub source to `UnifiedSearchServer`**

Add `github_source: Option<GitHubSource>` field. Update `new()`. Add GitHub cases to `handle_get_detail` dispatch.

- [ ] **Step 2: Wire in `main.rs`**

Clone GitHub config, create `GitHubSource`, pass to orchestrator sources list and server.

- [ ] **Step 3: Update MCP instructions**

Add "github" to the list of searchable sources. Mention `get_detail` now supports GitHub PRs and issues.

- [ ] **Step 4: Update verify.rs**

Add GitHub health check to preflight verification.

- [ ] **Step 5: Add test**

Add to `tests/test_server.rs`:
```rust
#[tokio::test]
async fn get_detail_github_url_without_source_returns_not_configured() {
    let server = build_server(vec![]);
    let output = server
        .handle_get_detail("https://github.com/org/repo/pull/1".to_string(), None, None)
        .await;
    assert!(output.to_lowercase().contains("not configured") || output.to_lowercase().contains("error"));
}
```

- [ ] **Step 6: Run all tests**

Run: `cargo test 2>&1 | tail -5`

- [ ] **Step 7: Commit**

```bash
git add src/server.rs src/main.rs src/mcp.rs src/verify.rs tests/test_server.rs
git commit -m "feat: wire GitHub source into server, main, and MCP instructions"
```

---

## Task 10: Update config.example.yaml, README, CHANGELOG

**Files:**
- Modify: `config.example.yaml`
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Update config.example.yaml**

Add GitHub source and cache_ttl_seconds:

```yaml
server:
  cache_ttl_seconds: 300    # response cache TTL (0 = disabled)

sources:
  github:
    enabled: true
    orgs: ["your-org"]
    repos: []                # empty = all repos in org
    weight: 1.0
    max_results: 10
```

- [ ] **Step 2: Update README**

Add GitHub to the credentials table, tools table, and source list. Mention caching and `no_cache` parameter. Update MCP tools table to show 6 tools.

- [ ] **Step 3: Update CHANGELOG**

Add v0.3.0 section.

- [ ] **Step 4: Run tests, build release**

```bash
cargo test
cargo build --release
```

- [ ] **Step 5: Commit**

```bash
git add config.example.yaml README.md CHANGELOG.md
git commit -m "docs: update config, README, and CHANGELOG for v0.3"
```

---

## Summary

| Task | Description | Commits |
|---|---|---|
| 1 | Confluence Markdown converter | 1 |
| 2 | Integrate Markdown into get_detail_page | 1 |
| 3 | Response cache module | 1 |
| 4 | Wire cache into orchestrator | 1 |
| 5 | GitHub source config + skeleton | 1 |
| 6 | GitHub search implementation | 1 |
| 7 | GitHub get_detail (PR + issue) | 1 |
| 8 | GitHub auto-detection in resolve.rs | 1 |
| 9 | Wire GitHub into server + main | 1 |
| 10 | Docs update | 1 |
| **Total** | | **10 commits** |
