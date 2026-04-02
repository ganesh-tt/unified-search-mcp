# Unified Search MCP v0.2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enrich search results with comments, add a `get_detail` deep-dive tool, and build a metrics/adoption tracking system.

**Architecture:** Three loosely-coupled enhancements sharing model changes. Comment enrichment happens inside existing source `search()` methods via parallel sub-requests. `get_detail` adds a new `resolve.rs` module that auto-detects identifiers and delegates to per-source detail fetchers. Metrics appends JSONL in a fire-and-forget `tokio::spawn`, with a `--stats` CLI mode reading both own logs and Claude Code conversation history.

**Tech Stack:** Rust, rmcp, tokio, reqwest, serde_json, wiremock (tests), chrono, regex

---

## File Structure

| File | Responsibility | New/Modified |
|---|---|---|
| `src/models.rs` | Add `PerSourceStats`, `DetailResult`, extend `UnifiedSearchResponse` with per-source metrics | Modified |
| `src/sources/jira.rs` | Extract comments from search response, add `get_detail_issue()` public method | Modified |
| `src/sources/confluence.rs` | Parallel comment fetch after search, add `get_detail_page()` public method | Modified |
| `src/sources/slack.rs` | Permalink parsing, add `get_detail_thread()` public method | Modified |
| `src/resolve.rs` | Identifier auto-detection + `get_detail` orchestration | **New** |
| `src/metrics.rs` | `MetricsLogger` struct, JSONL append, file rotation | **New** |
| `src/stats.rs` | `--stats` CLI mode, reads metrics.jsonl + Claude Code logs | **New** |
| `src/core.rs` | Return per-source timing in `UnifiedSearchResponse` | Modified |
| `src/server.rs` | Add `handle_get_detail`, wire metrics, enhance footer | Modified |
| `src/mcp.rs` | Register `get_detail` tool, pass `MetricsLogger` | Modified |
| `src/main.rs` | Add `--stats` flag, initialize `MetricsLogger` | Modified |
| `src/config.rs` | Add optional `metrics_path` field | Modified |
| `src/lib.rs` | Add `mod resolve; mod metrics; mod stats;` | Modified |
| `tests/test_jira.rs` | Tests for comment extraction in search, `get_detail_issue()` | Modified |
| `tests/test_confluence.rs` | Tests for comment enrichment, `get_detail_page()` | Modified |
| `tests/test_slack.rs` | Tests for permalink parsing, `get_detail_thread()` | Modified |
| `tests/test_resolve.rs` | Tests for identifier auto-detection | **New** |
| `tests/test_metrics.rs` | Tests for JSONL logging, rotation | **New** |
| `tests/test_server.rs` | Tests for `handle_get_detail`, enhanced footer | Modified |
| `fixtures/jira/issue_with_comments.json` | JIRA issue detail fixture | **New** |
| `fixtures/jira/issue_comments.json` | JIRA comments list fixture | **New** |
| `fixtures/confluence/page_detail.json` | Confluence page with body + children | **New** |
| `fixtures/confluence/page_comments.json` | Confluence comment list fixture | **New** |
| `fixtures/slack/conversation_history.json` | Slack conversations.history fixture | **New** |
| `fixtures/slack/conversation_replies.json` | Slack conversations.replies fixture | **New** |
| `fixtures/slack/conversation_info.json` | Slack conversations.info fixture | **New** |

---

## Task 1: Extend Models for Per-Source Stats and Detail Results

**Files:**
- Modify: `src/models.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/test_models.rs` additions — test that `PerSourceStats` serializes to JSON and `UnifiedSearchResponse` has the new field:

```rust
// Add to tests/test_models.rs

#[test]
fn per_source_stats_serializes() {
    use unified_search_mcp::models::PerSourceStats;

    let stats = PerSourceStats {
        source: "jira".to_string(),
        latency_ms: 180,
        result_count: 8,
        comment_count: 24,
        error: None,
    };

    let json = serde_json::to_value(&stats).unwrap();
    assert_eq!(json["source"], "jira");
    assert_eq!(json["latency_ms"], 180);
    assert_eq!(json["result_count"], 8);
    assert_eq!(json["comment_count"], 24);
    assert!(json["error"].is_null());
}

#[test]
fn unified_response_includes_per_source_stats() {
    use unified_search_mcp::models::{UnifiedSearchResponse, PerSourceStats};

    let response = UnifiedSearchResponse {
        results: vec![],
        warnings: vec![],
        total_sources_queried: 2,
        query_time_ms: 500,
        per_source_stats: vec![
            PerSourceStats {
                source: "slack".to_string(),
                latency_ms: 300,
                result_count: 5,
                comment_count: 10,
                error: None,
            },
        ],
    };

    assert_eq!(response.per_source_stats.len(), 1);
    assert_eq!(response.per_source_stats[0].source, "slack");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test per_source_stats_serializes unified_response_includes_per_source_stats -- --nocapture 2>&1 | head -30`
Expected: compilation error — `PerSourceStats` doesn't exist, `per_source_stats` field missing

- [ ] **Step 3: Add `PerSourceStats` struct and extend `UnifiedSearchResponse`**

In `src/models.rs`, add after `UnifiedSearchResponse`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerSourceStats {
    pub source: String,
    pub latency_ms: u64,
    pub result_count: usize,
    pub comment_count: usize,
    pub error: Option<String>,
}
```

And add the new field to `UnifiedSearchResponse`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResponse {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    pub total_sources_queried: usize,
    pub query_time_ms: u64,
    pub per_source_stats: Vec<PerSourceStats>,
}
```

- [ ] **Step 4: Fix compilation errors**

The new field needs to be populated wherever `UnifiedSearchResponse` is constructed. Update `src/core.rs` line ~168:

```rust
UnifiedSearchResponse {
    results: deduped,
    warnings,
    total_sources_queried,
    query_time_ms,
    per_source_stats: vec![], // populated in next task
}
```

Update `tests/test_core.rs` — every assertion that constructs/checks `UnifiedSearchResponse` needs the new field. The `MockSource`-based tests don't construct it directly (the orchestrator does), so only the orchestrator construction in `core.rs` needs the change.

Update `tests/test_server.rs` — same, the `UnifiedSearchServer` uses the orchestrator which now returns the new field.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/core.rs tests/test_models.rs
git commit -m "feat: add PerSourceStats model and extend UnifiedSearchResponse"
```

---

## Task 2: JIRA Comment Extraction from Search Response

The JIRA search API already receives `fields=...,comment,...` but the comment data is never extracted. This task extracts comments from the existing response — no extra API calls.

**Files:**
- Modify: `src/sources/jira.rs`
- Modify: `tests/test_jira.rs`
- Create: `fixtures/jira/search_with_comments.json`

- [ ] **Step 1: Create fixture with comments in search response**

Create `fixtures/jira/search_with_comments.json`:

```json
{
  "startAt": 0,
  "maxResults": 25,
  "total": 1,
  "issues": [
    {
      "key": "FIN-100",
      "fields": {
        "summary": "Fix OOM in broadcast",
        "description": {
          "type": "doc",
          "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Broadcast was unbounded"}]}]
        },
        "status": {"name": "In Progress"},
        "assignee": {"displayName": "Alice"},
        "updated": "2026-03-15T10:00:00.000+0000",
        "comment": {
          "comments": [
            {
              "id": "10001",
              "author": {"displayName": "Bob"},
              "body": {
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "I've reproduced this on staging. The broadcast queue grows without bound when the consumer is slower than the producer."}]}]
              },
              "created": "2026-03-14T09:00:00.000+0000",
              "updated": "2026-03-14T09:00:00.000+0000"
            },
            {
              "id": "10002",
              "author": {"displayName": "Alice"},
              "body": {
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Fix deployed. Added backpressure with a bounded channel of 1000 elements."}]}]
              },
              "created": "2026-03-15T08:00:00.000+0000",
              "updated": "2026-03-15T08:00:00.000+0000"
            },
            {
              "id": "10003",
              "author": {"displayName": "Charlie"},
              "body": {
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Verified on staging. OOM no longer occurs under load test."}]}]
              },
              "created": "2026-03-15T14:00:00.000+0000",
              "updated": "2026-03-15T14:00:00.000+0000"
            }
          ],
          "maxResults": 3,
          "total": 3,
          "startAt": 0
        }
      }
    }
  ]
}
```

- [ ] **Step 2: Write the failing test**

Add to `tests/test_jira.rs`:

```rust
#[tokio::test]
async fn search_extracts_comments_from_response() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/jira/search_with_comments.json");

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source.search(&make_query("broadcast")).await.unwrap();

    assert_eq!(results.len(), 1);

    // Check comment_count in metadata
    assert_eq!(
        results[0].metadata.get("comment_count"),
        Some(&"3".to_string()),
        "Expected comment_count=3 in metadata"
    );

    // Snippet should contain comments section
    assert!(
        results[0].snippet.contains("Comments (3"),
        "Snippet should contain 'Comments (3', got:\n{}",
        results[0].snippet
    );

    // Should contain most recent comments (latest first)
    assert!(
        results[0].snippet.contains("Charlie"),
        "Snippet should contain latest commenter 'Charlie'"
    );
    assert!(
        results[0].snippet.contains("Verified on staging"),
        "Snippet should contain latest comment text"
    );
}

#[tokio::test]
async fn search_handles_empty_comments() {
    let server = MockServer::start().await;

    // Existing helper already creates issues with empty comment arrays
    let issues = vec![jira_issue(
        "FIN-200",
        "No comments here",
        Some(adf_text("Description text")),
        "Open",
        None,
        "2026-03-15T10:00:00.000+0000",
    )];

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(issues)))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source.search(&make_query("test")).await.unwrap();

    assert_eq!(results.len(), 1);

    // comment_count should be "0" or absent
    let count = results[0].metadata.get("comment_count").map(|s| s.as_str()).unwrap_or("0");
    assert_eq!(count, "0");

    // Snippet should NOT contain "Comments" section
    assert!(
        !results[0].snippet.contains("Comments ("),
        "Snippet should not have comments section when there are none"
    );
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test search_extracts_comments search_handles_empty_comments -- --nocapture 2>&1 | head -20`
Expected: `search_extracts_comments` fails (no `comment_count` in metadata, snippet doesn't contain comments)

- [ ] **Step 4: Implement comment extraction in `jira.rs`**

In `src/sources/jira.rs`, inside the `for (i, issue) in issues.iter().enumerate()` loop (after the existing metadata block around line 284), add comment extraction:

```rust
            // Extract comments from the search response
            let comments = fields_obj
                .get("comment")
                .and_then(|c| c.get("comments"))
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            let comment_count = fields_obj
                .get("comment")
                .and_then(|c| c.get("total"))
                .and_then(|t| t.as_u64())
                .unwrap_or(comments.len() as u64);

            metadata.insert("comment_count".to_string(), comment_count.to_string());

            // Append latest 3 comments to snippet (most recent first)
            if !comments.is_empty() {
                let mut comment_texts: Vec<(String, String, String)> = Vec::new(); // (author, date, body)
                for comment in comments.iter().rev().take(3) {
                    let author = comment
                        .get("author")
                        .and_then(|a| a.get("displayName"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown");
                    let created = comment
                        .get("created")
                        .and_then(|c| c.as_str())
                        .and_then(|s| {
                            DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
                                .ok()
                                .map(|dt| dt.format("%Y-%m-%d").to_string())
                        })
                        .unwrap_or_default();
                    let body_raw = comment
                        .get("body")
                        .map(|b| Self::extract_adf_text(b))
                        .unwrap_or_default();
                    let body = Self::truncate_description(&body_raw, 150);
                    comment_texts.push((author.to_string(), created, body));
                }

                snippet.push_str(&format!("\n---\nComments ({} total):", comment_count));
                for (author, date, body) in &comment_texts {
                    snippet.push_str(&format!("\n[{}, {}]: {}", author, date, body));
                }
            }
```

This goes right before `results.push(SearchResult { ... })`. The `snippet` variable needs to be made mutable — change `let snippet = ...` to `let mut snippet = ...` on line ~252.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test test_jira -- --nocapture 2>&1 | tail -10`
Expected: all JIRA tests pass including the two new ones

- [ ] **Step 6: Commit**

```bash
git add src/sources/jira.rs tests/test_jira.rs fixtures/jira/search_with_comments.json
git commit -m "feat: extract JIRA comments from search response into snippets"
```

---

## Task 3: Confluence Comment Enrichment via Parallel Fetch

Unlike JIRA, Confluence search API does not return comments. We need parallel sub-requests.

**Files:**
- Modify: `src/sources/confluence.rs`
- Modify: `tests/test_confluence.rs`
- Create: `fixtures/confluence/page_comments.json`

- [ ] **Step 1: Create the comment fixture**

Create `fixtures/confluence/page_comments.json`:

```json
{
  "results": [
    {
      "id": "c1",
      "type": "comment",
      "title": "Re: Broadcast Threshold Design",
      "body": {
        "storage": {
          "value": "<p>We should set the threshold at 500 messages per second based on the load test results.</p>",
          "representation": "storage"
        }
      },
      "version": {
        "by": {
          "type": "known",
          "displayName": "Bob Smith"
        },
        "when": "2026-03-12T14:30:00.000Z",
        "number": 1
      }
    },
    {
      "id": "c2",
      "type": "comment",
      "title": "Re: Broadcast Threshold Design",
      "body": {
        "storage": {
          "value": "<p>Agreed. I've also added a <b>circuit breaker</b> to prevent cascade failures.</p>",
          "representation": "storage"
        }
      },
      "version": {
        "by": {
          "type": "known",
          "displayName": "Alice Chen"
        },
        "when": "2026-03-13T10:00:00.000Z",
        "number": 1
      }
    }
  ],
  "start": 0,
  "limit": 25,
  "size": 2
}
```

- [ ] **Step 2: Write the failing test**

Add to `tests/test_confluence.rs`:

```rust
/// Search results should include comments fetched via parallel sub-requests.
/// The wiremock server must serve both the search endpoint and the per-page
/// comment endpoint.
#[tokio::test]
async fn search_enriches_with_comments() {
    let server = MockServer::start().await;

    let search_body = include_str!("../fixtures/confluence/search_success.json");
    let comments_body = include_str!("../fixtures/confluence/page_comments.json");

    // Mount search endpoint
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(search_body, "application/json"))
        .mount(&server)
        .await;

    // Mount comment endpoint for all page IDs (wildcard matching via regex)
    // The fixture search_success.json has page IDs: 12345, 67890, 11111
    for page_id in &["12345", "67890", "11111"] {
        Mock::given(method("GET"))
            .and(path(format!("/wiki/rest/api/content/{}/child/comment", page_id)))
            .respond_with(ResponseTemplate::new(200).set_body_raw(comments_body, "application/json"))
            .mount(&server)
            .await;
    }

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query("broadcast threshold")).await.unwrap();

    assert_eq!(results.len(), 3);

    // Each result should have comment_count in metadata
    for result in &results {
        assert_eq!(
            result.metadata.get("comment_count"),
            Some(&"2".to_string()),
            "Expected comment_count=2 for page '{}'",
            result.title
        );
    }

    // First result's snippet should contain comment text (HTML stripped)
    assert!(
        results[0].snippet.contains("Comments (2"),
        "Snippet should contain 'Comments (2', got:\n{}",
        results[0].snippet
    );
    assert!(
        results[0].snippet.contains("Bob Smith") || results[0].snippet.contains("Alice Chen"),
        "Snippet should contain a comment author"
    );
    // HTML tags should be stripped from comment body
    assert!(
        !results[0].snippet.contains("<p>") && !results[0].snippet.contains("<b>"),
        "Comment body should have HTML stripped"
    );
}

/// When the comment endpoint fails (404/timeout), the search result still
/// returns — just without comments. No error propagated.
#[tokio::test]
async fn search_comment_failure_degrades_gracefully() {
    let server = MockServer::start().await;

    let search_body = include_str!("../fixtures/confluence/search_success.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(search_body, "application/json"))
        .mount(&server)
        .await;

    // Don't mount any comment endpoints — they'll 404

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query("broadcast")).await.unwrap();

    // Should still return search results despite comment fetch failures
    assert_eq!(results.len(), 3, "Search results should be returned even when comments fail");

    // comment_count should be "0" or absent (graceful degradation)
    for result in &results {
        let count = result.metadata.get("comment_count").map(|s| s.as_str()).unwrap_or("0");
        assert_eq!(count, "0", "comment_count should be 0 when fetch fails for '{}'", result.title);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test search_enriches_with_comments search_comment_failure_degrades_gracefully -- --nocapture 2>&1 | head -20`
Expected: FAIL — no `comment_count` in metadata

- [ ] **Step 4: Verify `search_success.json` fixture has page IDs**

Read `fixtures/confluence/search_success.json` and verify that the page IDs (`content.id`) match `12345`, `67890`, `11111`. If they differ, update the test's page ID list. The fixture was likely created during the initial build — check and adjust.

- [ ] **Step 5: Implement comment enrichment in `confluence.rs`**

In `src/sources/confluence.rs`, after the `search()` method builds `results` (around line ~342), add parallel comment fetch before returning:

```rust
        // Parallel comment enrichment
        let results = self.enrich_with_comments(results).await;

        Ok(results)
```

Add the enrichment method to the `impl ConfluenceSource` block:

```rust
    /// Fetch comments for each search result in parallel and append to snippets.
    async fn enrich_with_comments(&self, mut results: Vec<SearchResult>) -> Vec<SearchResult> {
        // Collect page IDs from results. Page ID is extracted from content.id
        // which we stored... but we need it. Let's store it in metadata during result construction.
        // We need to refactor slightly: store page_id in metadata during result construction above.

        let mut handles = Vec::new();

        for (i, result) in results.iter().enumerate() {
            let page_id = match result.metadata.get("page_id") {
                Some(id) if !id.is_empty() => id.clone(),
                _ => continue,
            };

            let client = self.client.clone();
            let base_url = self.config.base_url.clone();
            let auth = self.auth_header();
            let html_re = self.html_tag_re.clone();

            let handle = tokio::spawn(async move {
                let url = format!(
                    "{}/wiki/rest/api/content/{}/child/comment",
                    base_url, page_id
                );
                let resp = client
                    .get(&url)
                    .header("Authorization", auth)
                    .query(&[
                        ("expand", "body.storage,version"),
                        ("limit", "25"),
                    ])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            let comments = body
                                .get("results")
                                .and_then(|r| r.as_array())
                                .cloned()
                                .unwrap_or_default();
                            Some((i, comments))
                        } else {
                            Some((i, vec![]))
                        }
                    }
                    _ => Some((i, vec![])),
                }
            });
            handles.push(handle);
        }

        // Collect comment results
        for handle in handles {
            if let Ok(Some((idx, comments))) = handle.await {
                let comment_count = comments.len();
                results[idx]
                    .metadata
                    .insert("comment_count".to_string(), comment_count.to_string());

                if !comments.is_empty() {
                    let html_re = &self.html_tag_re;
                    let mut comment_lines = format!("\n---\nComments ({} total):", comment_count);

                    for comment in comments.iter().rev().take(3) {
                        let author = comment
                            .pointer("/version/by/displayName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown");
                        let when = comment
                            .pointer("/version/when")
                            .and_then(|v| v.as_str())
                            .and_then(|s| {
                                chrono::DateTime::parse_from_rfc3339(s)
                                    .ok()
                                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                            })
                            .unwrap_or_default();
                        let body_html = comment
                            .pointer("/body/storage/value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let body_text = html_re.replace_all(body_html, "").to_string();
                        let body_truncated = if body_text.len() > 150 {
                            format!("{}...", &body_text[..150])
                        } else {
                            body_text
                        };

                        comment_lines
                            .push_str(&format!("\n[{}, {}]: {}", author, when, body_truncated));
                    }
                    results[idx].snippet.push_str(&comment_lines);
                }
            }
        }

        // Set comment_count=0 for results that didn't get comments
        for result in &mut results {
            if !result.metadata.contains_key("comment_count") {
                result.metadata.insert("comment_count".to_string(), "0".to_string());
            }
        }

        results
    }
```

Also, during result construction in the `.map(|(i, r)| { ... })` closure, store the page ID in metadata:

```rust
                // Store page_id for comment enrichment
                if let Some(ref content) = r.content {
                    if let Some(ref id) = content.id {
                        metadata.insert("page_id".to_string(), id.clone());
                    }
                }
```

Remove the `#[allow(dead_code)]` from `ConfluenceContent.id`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test test_confluence -- --nocapture 2>&1 | tail -10`
Expected: all Confluence tests pass

- [ ] **Step 7: Commit**

```bash
git add src/sources/confluence.rs tests/test_confluence.rs fixtures/confluence/page_comments.json
git commit -m "feat: enrich Confluence search results with parallel comment fetch"
```

---

## Task 4: Identifier Auto-Detection (`resolve.rs`)

**Files:**
- Create: `src/resolve.rs`
- Create: `tests/test_resolve.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `tests/test_resolve.rs`:

```rust
use unified_search_mcp::resolve::{detect_source, SourceType, ParsedIdentifier};

#[test]
fn detects_jira_key() {
    let (source_type, parsed) = detect_source("FIN-1234").expect("Should detect JIRA key");
    assert!(matches!(source_type, SourceType::Jira));
    assert!(matches!(parsed, ParsedIdentifier::JiraKey(ref k) if k == "FIN-1234"));
}

#[test]
fn detects_jira_url() {
    let (source_type, parsed) =
        detect_source("https://tookitaki.atlassian.net/browse/FIN-1234")
            .expect("Should detect JIRA URL");
    assert!(matches!(source_type, SourceType::Jira));
    match parsed {
        ParsedIdentifier::JiraUrl { key, .. } => assert_eq!(key, "FIN-1234"),
        other => panic!("Expected JiraUrl, got {:?}", other),
    }
}

#[test]
fn detects_confluence_url() {
    let (source_type, parsed) =
        detect_source("https://tookitaki.atlassian.net/wiki/spaces/PROD/pages/123456/Page+Title")
            .expect("Should detect Confluence URL");
    assert!(matches!(source_type, SourceType::Confluence));
    match parsed {
        ParsedIdentifier::ConfluencePageId(id) => assert_eq!(id, "123456"),
        other => panic!("Expected ConfluencePageId, got {:?}", other),
    }
}

#[test]
fn detects_slack_permalink() {
    let (source_type, parsed) =
        detect_source("https://tookitaki.slack.com/archives/C06ABC123/p1712000000123456")
            .expect("Should detect Slack permalink");
    assert!(matches!(source_type, SourceType::Slack));
    match parsed {
        ParsedIdentifier::SlackPermalink { channel, ts } => {
            assert_eq!(channel, "C06ABC123");
            assert_eq!(ts, "1712000000.123456");
        }
        other => panic!("Expected SlackPermalink, got {:?}", other),
    }
}

#[test]
fn returns_none_for_unrecognized() {
    assert!(detect_source("just some random text").is_none());
    assert!(detect_source("").is_none());
    assert!(detect_source("https://google.com").is_none());
}

#[test]
fn jira_key_various_formats() {
    // Multi-letter project
    assert!(detect_source("PLAT-42").is_some());
    // Single letter project — should NOT match (JIRA requires 2+ chars)
    assert!(detect_source("A-1").is_none());
    // Lowercase — should NOT match
    assert!(detect_source("fin-1234").is_none());
    // Mixed case in numbers
    assert!(detect_source("FIN-0").is_some());
}

#[test]
fn slack_permalink_ts_parsing() {
    // The p-prefix timestamp has no dot. It needs to be converted:
    // p1712000000123456 -> 1712000000.123456 (dot before last 6 digits)
    let (_, parsed) =
        detect_source("https://foo.slack.com/archives/C123/p1712000000123456").unwrap();
    match parsed {
        ParsedIdentifier::SlackPermalink { ts, .. } => {
            assert_eq!(ts, "1712000000.123456");
        }
        other => panic!("Expected SlackPermalink, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_resolve -- --nocapture 2>&1 | head -10`
Expected: compilation error — module `resolve` doesn't exist

- [ ] **Step 3: Add `mod resolve;` to `lib.rs`**

In `src/lib.rs`, add:

```rust
pub mod resolve;
```

- [ ] **Step 4: Implement `src/resolve.rs`**

```rust
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub enum SourceType {
    Jira,
    Confluence,
    Slack,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedIdentifier {
    JiraKey(String),
    JiraUrl { base_url: String, key: String },
    ConfluencePageId(String),
    ConfluenceTitle { title: String, space: Option<String> },
    SlackPermalink { channel: String, ts: String },
}

/// Detect the source type and parse the identifier.
///
/// Returns `None` if the identifier doesn't match any known pattern.
pub fn detect_source(identifier: &str) -> Option<(SourceType, ParsedIdentifier)> {
    let id = identifier.trim();
    if id.is_empty() {
        return None;
    }

    // Priority 1: Atlassian JIRA URL
    // https://*.atlassian.net/browse/KEY-123
    let jira_url_re = Regex::new(
        r"^https?://([^/]+\.atlassian\.net)/browse/([A-Z][A-Z0-9]+-\d+)$"
    ).ok()?;
    if let Some(caps) = jira_url_re.captures(id) {
        let base_url = format!("https://{}", &caps[1]);
        let key = caps[2].to_string();
        return Some((SourceType::Jira, ParsedIdentifier::JiraUrl { base_url, key }));
    }

    // Priority 2: Confluence URL
    // https://*.atlassian.net/wiki/spaces/SPACE/pages/ID/...
    let confluence_url_re = Regex::new(
        r"^https?://[^/]+\.atlassian\.net/wiki/spaces/[^/]+/pages/(\d+)"
    ).ok()?;
    if let Some(caps) = confluence_url_re.captures(id) {
        let page_id = caps[1].to_string();
        return Some((SourceType::Confluence, ParsedIdentifier::ConfluencePageId(page_id)));
    }

    // Priority 3: Slack archive URL
    // https://*.slack.com/archives/CHANNEL/pTIMESTAMP
    let slack_url_re = Regex::new(
        r"^https?://[^/]+\.slack\.com/archives/([A-Z0-9]+)/p(\d+)$"
    ).ok()?;
    if let Some(caps) = slack_url_re.captures(id) {
        let channel = caps[1].to_string();
        let raw_ts = &caps[2];
        // Insert dot before last 6 digits: p1712000000123456 -> 1712000000.123456
        let ts = if raw_ts.len() > 6 {
            let (secs, micros) = raw_ts.split_at(raw_ts.len() - 6);
            format!("{}.{}", secs, micros)
        } else {
            raw_ts.to_string()
        };
        return Some((SourceType::Slack, ParsedIdentifier::SlackPermalink { channel, ts }));
    }

    // Priority 4: JIRA key pattern (no URL)
    // Requires 2+ uppercase letters, dash, 1+ digits
    let jira_key_re = Regex::new(r"^[A-Z][A-Z0-9]+-\d+$").ok()?;
    if jira_key_re.is_match(id) {
        return Some((SourceType::Jira, ParsedIdentifier::JiraKey(id.to_string())));
    }

    None
}

/// Force-interpret an identifier as a specific source type.
/// Used when the caller provides an explicit `source` parameter.
pub fn force_source(identifier: &str, source: &str) -> Option<(SourceType, ParsedIdentifier)> {
    let id = identifier.trim();
    match source {
        "jira" => {
            // Try auto-detect first (might be a URL), fall back to treating as key
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::Jira))
                .or_else(|| Some((SourceType::Jira, ParsedIdentifier::JiraKey(id.to_string()))))
        }
        "confluence" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::Confluence))
                .or_else(|| Some((
                    SourceType::Confluence,
                    ParsedIdentifier::ConfluenceTitle {
                        title: id.to_string(),
                        space: None,
                    },
                )))
        }
        "slack" => {
            detect_source(id)
                .filter(|(st, _)| matches!(st, SourceType::Slack))
                .or_else(|| None) // Slack requires a parseable URL
        }
        _ => None,
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test test_resolve -- --nocapture 2>&1 | tail -10`
Expected: all resolve tests pass

- [ ] **Step 6: Commit**

```bash
git add src/resolve.rs src/lib.rs tests/test_resolve.rs
git commit -m "feat: add identifier auto-detection for get_detail tool"
```

---

## Task 5: JIRA `get_detail_issue()` Method

**Files:**
- Modify: `src/sources/jira.rs`
- Modify: `tests/test_jira.rs`
- Create: `fixtures/jira/issue_detail.json`

- [ ] **Step 1: Create the issue detail fixture**

Create `fixtures/jira/issue_detail.json`:

```json
{
  "key": "FIN-1234",
  "fields": {
    "summary": "Fix broadcast threshold OOM",
    "description": {
      "type": "doc",
      "content": [{"type": "paragraph", "content": [{"type": "text", "text": "The broadcast queue grows unbounded when consumer is slower than producer. Need to add backpressure."}]}]
    },
    "issuetype": {"name": "Story"},
    "priority": {"name": "High"},
    "status": {"name": "In Progress"},
    "assignee": {"displayName": "Alice Chen"},
    "reporter": {"displayName": "Bob Smith"},
    "labels": ["backend", "performance"],
    "fixVersions": [{"name": "v6.3.4"}],
    "created": "2026-03-10T08:00:00.000+0000",
    "updated": "2026-04-01T14:30:00.000+0000",
    "issuelinks": [
      {
        "type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"},
        "outwardIssue": {
          "key": "FIN-1235",
          "fields": {"summary": "Deploy broadcast fix", "status": {"name": "Open"}}
        }
      },
      {
        "type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"},
        "inwardIssue": {
          "key": "FIN-1200",
          "fields": {"summary": "Kafka consumer optimization", "status": {"name": "Done"}}
        }
      }
    ],
    "subtasks": [
      {"key": "FIN-1234-1", "fields": {"summary": "Add bounded channel", "status": {"name": "Done"}}},
      {"key": "FIN-1234-2", "fields": {"summary": "Add metrics for queue depth", "status": {"name": "In Progress"}}}
    ],
    "comment": {
      "comments": [
        {
          "author": {"displayName": "Bob Smith"},
          "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Reproduced on staging."}]}]},
          "created": "2026-03-11T09:00:00.000+0000"
        },
        {
          "author": {"displayName": "Alice Chen"},
          "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Fix deployed with bounded channel of 1000 elements."}]}]},
          "created": "2026-03-15T08:00:00.000+0000"
        }
      ],
      "total": 2
    }
  }
}
```

- [ ] **Step 2: Write the failing test**

Add to `tests/test_jira.rs`:

```rust
#[tokio::test]
async fn get_detail_issue_returns_full_markdown() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/jira/issue_detail.json");

    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/FIN-1234"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let result = source.get_detail_issue("FIN-1234").await.unwrap();

    // Should contain the title
    assert!(result.contains("FIN-1234: Fix broadcast threshold OOM"), "Missing title");

    // Should contain description
    assert!(result.contains("broadcast queue grows unbounded"), "Missing description");

    // Should contain field metadata
    assert!(result.contains("In Progress"), "Missing status");
    assert!(result.contains("Alice Chen"), "Missing assignee");
    assert!(result.contains("v6.3.4"), "Missing fix version");
    assert!(result.contains("High"), "Missing priority");

    // Should contain linked issues
    assert!(result.contains("FIN-1235"), "Missing linked issue");
    assert!(result.contains("blocks"), "Missing link type");

    // Should contain subtasks
    assert!(result.contains("FIN-1234-1"), "Missing subtask 1");
    assert!(result.contains("FIN-1234-2"), "Missing subtask 2");

    // Should contain comments
    assert!(result.contains("Bob Smith"), "Missing comment author");
    assert!(result.contains("Reproduced on staging"), "Missing comment body");
}

#[tokio::test]
async fn get_detail_issue_404_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/NOPE-999"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let result = source.get_detail_issue("NOPE-999").await;

    assert!(result.is_err(), "Should return error for 404");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("404"), "Error should mention not found");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test get_detail_issue -- --nocapture 2>&1 | head -10`
Expected: compilation error — `get_detail_issue` doesn't exist on `JiraSource`

- [ ] **Step 4: Implement `get_detail_issue` on `JiraSource`**

Add to `impl JiraSource` block in `src/sources/jira.rs`:

```rust
    /// Fetch full details for a single JIRA issue by key.
    /// Returns a Markdown string with all fields, linked issues, subtasks, and comments.
    pub async fn get_detail_issue(&self, key: &str) -> Result<String, SearchError> {
        let url = format!("{}/rest/api/3/issue/{}", self.config.base_url, key);
        let fields = "summary,description,status,assignee,reporter,labels,fixVersions,\
                       issuelinks,subtasks,comment,priority,issuetype,created,updated";

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .query(&[("fields", fields)])
            .send()
            .await
            .map_err(|e| SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("JIRA issue {} not found", key),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SearchError::Auth {
                source_name: "jira".to_string(),
                message: "Authentication failed".to_string(),
            });
        }
        if !status.is_success() {
            return Err(SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("HTTP {}", status),
            });
        }

        let body: serde_json::Value = response.json().await.map_err(|e| SearchError::Source {
            source_name: "jira".to_string(),
            message: format!("Failed to parse JSON: {}", e),
        })?;

        let issue_key = body.get("key").and_then(|v| v.as_str()).unwrap_or(key);
        let fields_obj = body.get("fields").cloned().unwrap_or(serde_json::Value::Null);

        let summary = fields_obj.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let description = fields_obj
            .get("description")
            .filter(|d| !d.is_null())
            .map(|d| Self::extract_adf_text(d))
            .unwrap_or_default();

        let issue_type = fields_obj.pointer("/issuetype/name").and_then(|v| v.as_str()).unwrap_or("-");
        let priority = fields_obj.pointer("/priority/name").and_then(|v| v.as_str()).unwrap_or("-");
        let status_name = fields_obj.pointer("/status/name").and_then(|v| v.as_str()).unwrap_or("-");
        let assignee = fields_obj.pointer("/assignee/displayName").and_then(|v| v.as_str()).unwrap_or("Unassigned");
        let reporter = fields_obj.pointer("/reporter/displayName").and_then(|v| v.as_str()).unwrap_or("-");

        let labels = fields_obj
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| "-".to_string());

        let fix_versions = fields_obj
            .get("fixVersions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.get("name").and_then(|n| n.as_str())).collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| "-".to_string());

        let created = fields_obj.get("created").and_then(|v| v.as_str()).unwrap_or("-");
        let updated = fields_obj.get("updated").and_then(|v| v.as_str()).unwrap_or("-");

        // Build markdown
        let mut md = format!("# {}: {}\n\n", issue_key, summary);

        md.push_str("| Field | Value |\n|---|---|\n");
        md.push_str(&format!("| Status | {} |\n", status_name));
        md.push_str(&format!("| Type | {} |\n", issue_type));
        md.push_str(&format!("| Priority | {} |\n", priority));
        md.push_str(&format!("| Assignee | {} |\n", assignee));
        md.push_str(&format!("| Reporter | {} |\n", reporter));
        md.push_str(&format!("| Labels | {} |\n", labels));
        md.push_str(&format!("| Fix Versions | {} |\n", fix_versions));
        md.push_str(&format!("| Created | {} |\n", created));
        md.push_str(&format!("| Updated | {} |\n", updated));

        // Description
        if !description.is_empty() {
            md.push_str(&format!("\n## Description\n\n{}\n", description));
        }

        // Linked issues
        if let Some(links) = fields_obj.get("issuelinks").and_then(|v| v.as_array()) {
            if !links.is_empty() {
                md.push_str("\n## Linked Issues\n\n");
                for link in links {
                    let link_type = link.get("type").and_then(|t| t.get("outward")).and_then(|v| v.as_str());
                    let inward_type = link.get("type").and_then(|t| t.get("inward")).and_then(|v| v.as_str());

                    if let Some(outward) = link.get("outwardIssue") {
                        let lk = outward.get("key").and_then(|v| v.as_str()).unwrap_or("?");
                        let ls = outward.pointer("/fields/summary").and_then(|v| v.as_str()).unwrap_or("");
                        let lst = outward.pointer("/fields/status/name").and_then(|v| v.as_str()).unwrap_or("");
                        md.push_str(&format!("- **{}** {}: {} ({})\n", link_type.unwrap_or("relates to"), lk, ls, lst));
                    }
                    if let Some(inward) = link.get("inwardIssue") {
                        let lk = inward.get("key").and_then(|v| v.as_str()).unwrap_or("?");
                        let ls = inward.pointer("/fields/summary").and_then(|v| v.as_str()).unwrap_or("");
                        let lst = inward.pointer("/fields/status/name").and_then(|v| v.as_str()).unwrap_or("");
                        md.push_str(&format!("- **{}** {}: {} ({})\n", inward_type.unwrap_or("relates to"), lk, ls, lst));
                    }
                }
            }
        }

        // Subtasks
        if let Some(subtasks) = fields_obj.get("subtasks").and_then(|v| v.as_array()) {
            if !subtasks.is_empty() {
                md.push_str("\n## Subtasks\n\n");
                for st in subtasks {
                    let sk = st.get("key").and_then(|v| v.as_str()).unwrap_or("?");
                    let ss = st.pointer("/fields/summary").and_then(|v| v.as_str()).unwrap_or("");
                    let sst = st.pointer("/fields/status/name").and_then(|v| v.as_str()).unwrap_or("");
                    let checkbox = if sst == "Done" { "x" } else { " " };
                    md.push_str(&format!("- [{}] {}: {} ({})\n", checkbox, sk, ss, sst));
                }
            }
        }

        // Comments
        let comments = fields_obj
            .pointer("/comment/comments")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if !comments.is_empty() {
            md.push_str(&format!("\n## Comments ({})\n", comments.len()));
            for comment in &comments {
                let author = comment.pointer("/author/displayName").and_then(|v| v.as_str()).unwrap_or("Unknown");
                let created_at = comment.get("created").and_then(|v| v.as_str()).unwrap_or("");
                let body_text = comment.get("body").map(|b| Self::extract_adf_text(b)).unwrap_or_default();
                md.push_str(&format!("\n### {} -- {}\n{}\n", author, created_at, body_text));
            }
        }

        Ok(md)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test get_detail_issue -- --nocapture 2>&1 | tail -10`
Expected: both tests pass

- [ ] **Step 6: Commit**

```bash
git add src/sources/jira.rs tests/test_jira.rs fixtures/jira/issue_detail.json
git commit -m "feat: add get_detail_issue method for full JIRA ticket retrieval"
```

---

## Task 6: Confluence `get_detail_page()` Method

**Files:**
- Modify: `src/sources/confluence.rs`
- Modify: `tests/test_confluence.rs`
- Create: `fixtures/confluence/page_detail.json`

- [ ] **Step 1: Create the page detail fixture**

Create `fixtures/confluence/page_detail.json`:

```json
{
  "id": "123456",
  "type": "page",
  "title": "Broadcast Threshold Design",
  "space": {
    "key": "PROD",
    "name": "Production"
  },
  "body": {
    "storage": {
      "value": "<h2>Overview</h2><p>The broadcast threshold controls how many messages per second are sent to downstream consumers. We settled on <b>500 msg/s</b> based on load testing.</p><h2>Decision</h2><p>Use a bounded channel with backpressure.</p>",
      "representation": "storage"
    }
  },
  "version": {
    "by": {
      "type": "known",
      "displayName": "Alice Chen"
    },
    "when": "2026-04-01T10:00:00.000Z",
    "number": 5
  },
  "children": {
    "page": {
      "results": [
        {"id": "111", "type": "page", "title": "Load Test Results", "_links": {"webui": "/spaces/PROD/pages/111/Load+Test+Results"}},
        {"id": "222", "type": "page", "title": "Configuration Guide", "_links": {"webui": "/spaces/PROD/pages/222/Configuration+Guide"}}
      ],
      "size": 2
    },
    "comment": {
      "results": [
        {
          "id": "c1",
          "body": {"storage": {"value": "<p>Looks good to me. Approved.</p>"}},
          "version": {"by": {"displayName": "Bob Smith"}, "when": "2026-03-15T14:00:00.000Z"}
        },
        {
          "id": "c2",
          "body": {"storage": {"value": "<p>Should we document the <b>fallback behavior</b> when the channel is full?</p>"}},
          "version": {"by": {"displayName": "Charlie Lee"}, "when": "2026-03-20T09:30:00.000Z"}
        }
      ],
      "size": 2
    }
  },
  "metadata": {
    "labels": {
      "results": [
        {"name": "architecture"},
        {"name": "decisions"}
      ]
    }
  },
  "_links": {
    "webui": "/spaces/PROD/pages/123456/Broadcast+Threshold+Design",
    "base": "https://tookitaki.atlassian.net/wiki"
  }
}
```

- [ ] **Step 2: Write the failing test**

Add to `tests/test_confluence.rs`:

```rust
#[tokio::test]
async fn get_detail_page_returns_full_markdown() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/page_detail.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123456"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let result = source.get_detail_page("123456").await.unwrap();

    assert!(result.contains("Broadcast Threshold Design"), "Missing title");
    assert!(result.contains("PROD"), "Missing space");
    assert!(result.contains("500 msg/s"), "Missing body content (HTML stripped)");

    // Child pages
    assert!(result.contains("Load Test Results"), "Missing child page");
    assert!(result.contains("Configuration Guide"), "Missing child page");

    // Comments
    assert!(result.contains("Bob Smith"), "Missing comment author");
    assert!(result.contains("Looks good to me"), "Missing comment text");
    assert!(result.contains("Charlie Lee"), "Missing second comment author");

    // Labels
    assert!(result.contains("architecture"), "Missing label");

    // HTML should be stripped
    assert!(!result.contains("<h2>"), "HTML tags should be stripped");
    assert!(!result.contains("<p>"), "HTML tags should be stripped");
    assert!(!result.contains("<b>"), "HTML tags should be stripped");
}

#[tokio::test]
async fn get_detail_page_404_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/999999"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let result = source.get_detail_page("999999").await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("404"));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test get_detail_page -- --nocapture 2>&1 | head -10`
Expected: compilation error — `get_detail_page` doesn't exist

- [ ] **Step 4: Implement `get_detail_page` on `ConfluenceSource`**

Add to `impl ConfluenceSource` in `src/sources/confluence.rs`:

```rust
    /// Fetch full details for a Confluence page by ID.
    /// Returns a Markdown string with body, child pages, comments, and labels.
    pub async fn get_detail_page(&self, page_id: &str) -> Result<String, SearchError> {
        let url = format!("{}/wiki/rest/api/content/{}", self.config.base_url, page_id);
        let expand = "body.storage,version,children.page,children.comment.body.storage,metadata.labels,space";

        let response = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .query(&[("expand", expand)])
            .send()
            .await
            .map_err(|e| SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("Confluence page {} not found", page_id),
            });
        }
        if status.as_u16() == 401 {
            return Err(SearchError::Auth {
                source_name: "confluence".to_string(),
                message: "Authentication failed".to_string(),
            });
        }
        if !status.is_success() {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("HTTP {}", status.as_u16()),
            });
        }

        let body: serde_json::Value = response.json().await.map_err(|e| SearchError::Source {
            source_name: "confluence".to_string(),
            message: format!("Failed to parse JSON: {}", e),
        })?;

        let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let space_key = body.pointer("/space/key").and_then(|v| v.as_str()).unwrap_or("-");
        let author = body.pointer("/version/by/displayName").and_then(|v| v.as_str()).unwrap_or("-");
        let last_updated = body.pointer("/version/when").and_then(|v| v.as_str()).unwrap_or("-");

        // Labels
        let labels = body
            .pointer("/metadata/labels/results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "-".to_string());

        // Body
        let body_html = body
            .pointer("/body/storage/value")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let body_text = self.strip_html(body_html);

        // Build markdown
        let mut md = format!("# {}\n\n", title);

        md.push_str("| Field | Value |\n|---|---|\n");
        md.push_str(&format!("| Space | {} |\n", space_key));
        md.push_str(&format!("| Author | {} |\n", author));
        md.push_str(&format!("| Last Updated | {} |\n", last_updated));
        md.push_str(&format!("| Labels | {} |\n", labels));

        // Content
        if !body_text.is_empty() {
            md.push_str(&format!("\n## Content\n\n{}\n", body_text));
        }

        // Child pages
        if let Some(children) = body.pointer("/children/page/results").and_then(|v| v.as_array()) {
            if !children.is_empty() {
                md.push_str("\n## Child Pages\n\n");
                for child in children {
                    let child_title = child.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    let child_link = child.pointer("/_links/webui").and_then(|v| v.as_str()).unwrap_or("");
                    md.push_str(&format!("- [{}]({})\n", child_title, child_link));
                }
            }
        }

        // Comments
        if let Some(comments) = body.pointer("/children/comment/results").and_then(|v| v.as_array()) {
            if !comments.is_empty() {
                md.push_str(&format!("\n## Comments ({})\n", comments.len()));
                for comment in comments {
                    let c_author = comment.pointer("/version/by/displayName").and_then(|v| v.as_str()).unwrap_or("Unknown");
                    let c_when = comment.pointer("/version/when").and_then(|v| v.as_str()).unwrap_or("");
                    let c_body_html = comment.pointer("/body/storage/value").and_then(|v| v.as_str()).unwrap_or("");
                    let c_body = self.strip_html(c_body_html);
                    md.push_str(&format!("\n### {} -- {}\n{}\n", c_author, c_when, c_body));
                }
            }
        }

        Ok(md)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test get_detail_page -- --nocapture 2>&1 | tail -10`
Expected: both tests pass

- [ ] **Step 6: Commit**

```bash
git add src/sources/confluence.rs tests/test_confluence.rs fixtures/confluence/page_detail.json
git commit -m "feat: add get_detail_page method for full Confluence page retrieval"
```

---

## Task 7: Slack `get_detail_thread()` Method

**Files:**
- Modify: `src/sources/slack.rs`
- Modify: `tests/test_slack.rs`
- Create: `fixtures/slack/conversation_history.json`
- Create: `fixtures/slack/conversation_replies.json`
- Create: `fixtures/slack/conversation_info.json`

- [ ] **Step 1: Create fixtures**

Create `fixtures/slack/conversation_history.json`:

```json
{
  "ok": true,
  "messages": [
    {
      "type": "message",
      "user": "U123",
      "text": "We need to decide on the broadcast threshold. Current thinking is 500 msg/s.",
      "ts": "1712000000.123456"
    }
  ],
  "has_more": false
}
```

Create `fixtures/slack/conversation_replies.json`:

```json
{
  "ok": true,
  "messages": [
    {
      "type": "message",
      "user": "U123",
      "text": "We need to decide on the broadcast threshold. Current thinking is 500 msg/s.",
      "ts": "1712000000.123456"
    },
    {
      "type": "message",
      "user": "U456",
      "text": "That sounds reasonable. Load tests show we can handle up to 800 msg/s before OOM.",
      "ts": "1712000100.000000"
    },
    {
      "type": "message",
      "user": "U789",
      "text": "Let's go with 500 msg/s with a circuit breaker at 750.",
      "ts": "1712000200.000000"
    }
  ],
  "has_more": false
}
```

Create `fixtures/slack/conversation_info.json`:

```json
{
  "ok": true,
  "channel": {
    "id": "C06ABC123",
    "name": "engineering",
    "is_channel": true,
    "topic": {"value": "Engineering discussions"},
    "purpose": {"value": "General engineering chat"}
  }
}
```

- [ ] **Step 2: Write the failing test**

Add to `tests/test_slack.rs`:

```rust
#[tokio::test]
async fn get_detail_thread_returns_full_markdown() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/conversations.history"))
        .and(query_param("channel", "C06ABC123"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("../fixtures/slack/conversation_history.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/conversations.replies"))
        .and(query_param("channel", "C06ABC123"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("../fixtures/slack/conversation_replies.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/conversations.info"))
        .and(query_param("channel", "C06ABC123"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("../fixtures/slack/conversation_info.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let result = source
        .get_detail_thread("C06ABC123", "1712000000.123456")
        .await
        .unwrap();

    // Should contain channel name
    assert!(result.contains("engineering"), "Missing channel name");

    // Should contain the original message
    assert!(
        result.contains("broadcast threshold"),
        "Missing original message"
    );

    // Should contain replies
    assert!(
        result.contains("800 msg/s before OOM"),
        "Missing reply content"
    );
    assert!(
        result.contains("circuit breaker at 750"),
        "Missing second reply"
    );
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test get_detail_thread -- --nocapture 2>&1 | head -10`
Expected: compilation error — method doesn't exist

- [ ] **Step 4: Implement `get_detail_thread` on `SlackSource`**

Add to `impl SlackSource` in `src/sources/slack.rs`:

```rust
    /// Fetch a Slack message and its full thread by channel ID and timestamp.
    /// Returns a Markdown string with the message, replies, and channel info.
    pub async fn get_detail_thread(
        &self,
        channel: &str,
        ts: &str,
    ) -> Result<String, SearchError> {
        // Fetch channel info, message, and replies in parallel
        let channel_owned = channel.to_string();
        let ts_owned = ts.to_string();

        let client = self.client.clone();
        let token = self.config.user_token.clone();
        let base = self.config.base_url.clone();

        // 1. Fetch channel info
        let info_url = format!("{}/api/conversations.info", base);
        let info_resp = client
            .get(&info_url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[("channel", channel)])
            .send()
            .await
            .map_err(SearchError::Http)?;

        let info_body: serde_json::Value = info_resp.json().await.map_err(|e| SearchError::Source {
            source_name: "slack".to_string(),
            message: format!("Failed to parse conversations.info: {}", e),
        })?;

        let channel_name = info_body
            .pointer("/channel/name")
            .and_then(|v| v.as_str())
            .unwrap_or(&channel_owned);

        // 2. Fetch replies (includes the parent message)
        let replies_url = format!("{}/api/conversations.replies", base);
        let replies_resp = client
            .get(&replies_url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[
                ("channel", channel),
                ("ts", ts),
                ("limit", "200"),
            ])
            .send()
            .await
            .map_err(SearchError::Http)?;

        let replies_body: serde_json::Value =
            replies_resp.json().await.map_err(|e| SearchError::Source {
                source_name: "slack".to_string(),
                message: format!("Failed to parse conversations.replies: {}", e),
            })?;

        if !replies_body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let error = replies_body.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
            return Err(SearchError::Source {
                source_name: "slack".to_string(),
                message: format!("conversations.replies failed: {}", error),
            });
        }

        let messages = replies_body
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Build markdown
        let mut md = format!("# Slack Thread in #{}\n\n", channel_name);

        if let Some(first) = messages.first() {
            let user = first.get("user").and_then(|v| v.as_str()).unwrap_or("unknown");
            let text = first.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let msg_ts = first.get("ts").and_then(|v| v.as_str()).unwrap_or("");
            let dt = parse_slack_ts(msg_ts)
                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_default();

            md.push_str(&format!("**Started by**: {} -- {}\n\n", user, dt));
            md.push_str(&format!("## Original Message\n\n{}\n", text));
        }

        let replies: Vec<_> = messages.iter().skip(1).collect();
        if !replies.is_empty() {
            md.push_str(&format!("\n## Thread Replies ({})\n", replies.len()));

            let mut participants = std::collections::HashSet::new();
            if let Some(first) = messages.first() {
                if let Some(u) = first.get("user").and_then(|v| v.as_str()) {
                    participants.insert(u.to_string());
                }
            }

            for reply in &replies {
                let user = reply.get("user").and_then(|v| v.as_str()).unwrap_or("unknown");
                let text = reply.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let reply_ts = reply.get("ts").and_then(|v| v.as_str()).unwrap_or("");
                let dt = parse_slack_ts(reply_ts)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_default();

                participants.insert(user.to_string());
                md.push_str(&format!("\n### {} -- {}\n{}\n", user, dt, text));
            }

            // Participants
            let mut sorted: Vec<_> = participants.into_iter().collect();
            sorted.sort();
            md.push_str(&format!("\n## Participants\n\n{}\n", sorted.join(", ")));
        }

        Ok(md)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test get_detail_thread -- --nocapture 2>&1 | tail -10`
Expected: test passes

- [ ] **Step 6: Commit**

```bash
git add src/sources/slack.rs tests/test_slack.rs fixtures/slack/conversation_history.json fixtures/slack/conversation_replies.json fixtures/slack/conversation_info.json
git commit -m "feat: add get_detail_thread method for full Slack thread retrieval"
```

---

## Task 8: Wire `get_detail` MCP Tool

**Files:**
- Modify: `src/server.rs`
- Modify: `src/mcp.rs`
- Modify: `tests/test_server.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/test_server.rs`:

```rust
/// handle_get_detail with a JIRA key should return markdown containing the issue.
/// We need a mock source that also implements get_detail. Since we're at the server
/// layer, we test via the UnifiedSearchServer which delegates to resolve.rs.
/// For now, test that handle_get_detail exists and returns a non-empty error for
/// an unknown identifier (no sources available).
#[tokio::test]
async fn get_detail_returns_error_for_unknown_identifier() {
    let server = build_server(vec![]);

    let output = server
        .handle_get_detail("random text".to_string(), None, None)
        .await;

    // Should return an error message about unrecognized identifier
    let lower = output.to_lowercase();
    assert!(
        lower.contains("could not detect") || lower.contains("not recognized") || lower.contains("error"),
        "Expected error for unrecognized identifier, got:\n{output}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test get_detail_returns_error -- --nocapture 2>&1 | head -10`
Expected: compilation error — `handle_get_detail` doesn't exist

- [ ] **Step 3: Add `handle_get_detail` to `UnifiedSearchServer`**

The server needs access to individual source configs to call `get_detail_issue`, `get_detail_page`, `get_detail_thread`. Refactor `UnifiedSearchServer` to hold source configs alongside the orchestrator.

In `src/server.rs`, add:

```rust
use crate::resolve::{detect_source, force_source, SourceType, ParsedIdentifier};
use crate::sources::jira::JiraSource;
use crate::sources::confluence::ConfluenceSource;
use crate::sources::slack::SlackSource;
```

Add optional source references to `UnifiedSearchServer`:

```rust
pub struct UnifiedSearchServer {
    orchestrator: SearchOrchestrator,
    jira_source: Option<JiraSource>,
    confluence_source: Option<ConfluenceSource>,
    slack_source: Option<SlackSource>,
}
```

Update `new()`:

```rust
    pub fn new(
        orchestrator: SearchOrchestrator,
        jira_source: Option<JiraSource>,
        confluence_source: Option<ConfluenceSource>,
        slack_source: Option<SlackSource>,
    ) -> Self {
        Self { orchestrator, jira_source, confluence_source, slack_source }
    }
```

Add the handler:

```rust
    pub async fn handle_get_detail(
        &self,
        identifier: String,
        source: Option<String>,
        max_comments: Option<usize>,
    ) -> String {
        let detection = if let Some(ref src) = source {
            force_source(&identifier, src)
        } else {
            detect_source(&identifier)
        };

        let (source_type, parsed) = match detection {
            Some(pair) => pair,
            None => {
                return format!(
                    "Error: Could not detect source type for '{}'. \
                     Provide a `source` parameter ('jira', 'confluence', 'slack').",
                    identifier
                );
            }
        };

        match source_type {
            SourceType::Jira => {
                let key = match parsed {
                    ParsedIdentifier::JiraKey(k) => k,
                    ParsedIdentifier::JiraUrl { key, .. } => key,
                    _ => return "Error: unexpected parsed identifier for JIRA".to_string(),
                };
                match &self.jira_source {
                    Some(src) => match src.get_detail_issue(&key).await {
                        Ok(md) => md,
                        Err(e) => format!("Error: {}", e),
                    },
                    None => "Error: JIRA source not configured".to_string(),
                }
            }
            SourceType::Confluence => {
                let page_id = match parsed {
                    ParsedIdentifier::ConfluencePageId(id) => id,
                    ParsedIdentifier::ConfluenceTitle { title, space } => {
                        // TODO in a future task: title resolution via API
                        return format!(
                            "Error: Confluence title lookup not yet implemented. \
                             Use a page URL or ID instead. (title='{}', space={:?})",
                            title, space
                        );
                    }
                    _ => return "Error: unexpected parsed identifier for Confluence".to_string(),
                };
                match &self.confluence_source {
                    Some(src) => match src.get_detail_page(&page_id).await {
                        Ok(md) => md,
                        Err(e) => format!("Error: {}", e),
                    },
                    None => "Error: Confluence source not configured".to_string(),
                }
            }
            SourceType::Slack => {
                let (channel, ts) = match parsed {
                    ParsedIdentifier::SlackPermalink { channel, ts } => (channel, ts),
                    _ => return "Error: unexpected parsed identifier for Slack".to_string(),
                };
                match &self.slack_source {
                    Some(src) => match src.get_detail_thread(&channel, &ts).await {
                        Ok(md) => md,
                        Err(e) => format!("Error: {}", e),
                    },
                    None => "Error: Slack source not configured".to_string(),
                }
            }
        }
    }
```

Update all call sites: `UnifiedSearchServer::new()` in `main.rs` and `tests/test_server.rs` now needs the extra params. In tests, pass `None, None, None`. In `main.rs`, clone the sources before passing to both the orchestrator and the server.

- [ ] **Step 4: Register `get_detail` tool in `mcp.rs`**

Add the params struct:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetDetailParams {
    /// The identifier: JIRA key (FIN-1234), URL, or Confluence page title
    pub identifier: String,
    /// Optional: force source type ('jira', 'confluence', 'slack'). If omitted, auto-detected.
    #[serde(default)]
    pub source: Option<String>,
    /// Optional: max comments to return (default: all)
    #[serde(default)]
    pub max_comments: Option<usize>,
}
```

Add the tool method to `#[tool_router] impl McpServer`:

```rust
    #[tool(description = "Fetch full details for a specific JIRA ticket, Confluence page, or Slack thread. Accepts a JIRA key (FIN-1234), a JIRA/Confluence/Slack URL, or a Confluence page title. Returns full content: description, all comments, linked issues, subtasks, child pages, or thread replies depending on source. Use this when you need complete context about a specific item found via unified_search.")]
    async fn get_detail(
        &self,
        Parameters(params): Parameters<GetDetailParams>,
    ) -> String {
        self.server
            .handle_get_detail(params.identifier, params.source, params.max_comments)
            .await
    }
```

- [ ] **Step 5: Update `main.rs` to pass sources to `UnifiedSearchServer`**

Clone sources before moving them into the orchestrator. After building sources in `main.rs`:

```rust
    // Clone sources for get_detail before moving into orchestrator
    let jira_detail = app_config.sources.jira.as_ref()
        .filter(|c| c.enabled)
        .map(|c| JiraSource::new(c.config.clone()));
    let confluence_detail = app_config.sources.confluence.as_ref()
        .filter(|c| c.enabled)
        .map(|c| ConfluenceSource::new(c.config.clone()));
    let slack_detail = app_config.sources.slack.as_ref()
        .filter(|c| c.enabled)
        .map(|c| SlackSource::new(c.config.clone()));
    
    // ... existing orchestrator construction ...

    let server = UnifiedSearchServer::new(orchestrator, jira_detail, confluence_detail, slack_detail);
```

- [ ] **Step 6: Update test helpers**

In `tests/test_server.rs`, update `build_server`:

```rust
fn build_server(sources: Vec<Box<dyn SearchSource>>) -> UnifiedSearchServer {
    let orchestrator = SearchOrchestrator::new(sources, default_config());
    UnifiedSearchServer::new(orchestrator, None, None, None)
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/server.rs src/mcp.rs src/main.rs tests/test_server.rs
git commit -m "feat: wire get_detail MCP tool with auto-detection and source delegation"
```

---

## Task 9: Metrics Logger (`metrics.rs`)

**Files:**
- Create: `src/metrics.rs`
- Create: `tests/test_metrics.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `tests/test_metrics.rs`:

```rust
use std::path::PathBuf;
use tempfile::TempDir;
use unified_search_mcp::metrics::{MetricsLogger, MetricsEntry};

#[tokio::test]
async fn logs_entry_to_jsonl() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metrics.jsonl");

    let logger = MetricsLogger::new(path.clone());

    let entry = MetricsEntry::Search {
        tool: "unified_search".to_string(),
        query: "broadcast threshold".to_string(),
        sources_queried: vec!["slack".to_string(), "jira".to_string()],
        total_results: 10,
        deduped_results: 8,
        total_ms: 450,
    };

    logger.log(entry).await;

    // Give the background task time to write
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "Expected 1 log line");

    let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["tool"], "unified_search");
    assert_eq!(parsed["query"], "broadcast threshold");
    assert_eq!(parsed["total_results"], 10);
    assert!(parsed["ts"].is_string(), "Should have timestamp");
}

#[tokio::test]
async fn logs_multiple_entries() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metrics.jsonl");

    let logger = MetricsLogger::new(path.clone());

    for i in 0..5 {
        let entry = MetricsEntry::Search {
            tool: "unified_search".to_string(),
            query: format!("query {}", i),
            sources_queried: vec!["slack".to_string()],
            total_results: i,
            deduped_results: i,
            total_ms: 100 + i as u64,
        };
        logger.log(entry).await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 5, "Expected 5 log lines");
}

#[test]
fn detail_entry_serializes() {
    let entry = MetricsEntry::Detail {
        tool: "get_detail".to_string(),
        identifier: "FIN-1234".to_string(),
        detected_source: "jira".to_string(),
        explicit_source: None,
        latency_ms: 350,
        comments_returned: 15,
        error: None,
    };

    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["tool"], "get_detail");
    assert_eq!(json["identifier"], "FIN-1234");
    assert_eq!(json["latency_ms"], 350);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_metrics -- --nocapture 2>&1 | head -10`
Expected: compilation error — module doesn't exist

- [ ] **Step 3: Add `mod metrics;` to `lib.rs`**

```rust
pub mod metrics;
```

- [ ] **Step 4: Implement `src/metrics.rs`**

```rust
use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;

/// A single metrics entry. Tagged enum serialized to JSON.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MetricsEntry {
    Search {
        tool: String,
        query: String,
        sources_queried: Vec<String>,
        total_results: usize,
        deduped_results: usize,
        total_ms: u64,
    },
    Detail {
        tool: String,
        identifier: String,
        detected_source: String,
        explicit_source: Option<String>,
        latency_ms: u64,
        comments_returned: usize,
        error: Option<String>,
    },
}

/// Append-only JSONL metrics logger.
///
/// Each call to `log()` spawns a background task that appends one JSON line.
/// File rotation occurs when the file exceeds 10MB.
#[derive(Clone)]
pub struct MetricsLogger {
    path: PathBuf,
}

impl MetricsLogger {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Log a metrics entry. This is fire-and-forget — writes happen in a
    /// background tokio task and never block the caller.
    pub async fn log(&self, entry: MetricsEntry) {
        let path = self.path.clone();

        tokio::spawn(async move {
            if let Err(e) = write_entry(&path, &entry) {
                eprintln!("metrics: failed to write: {}", e);
            }
        });
    }
}

/// Serialize and append one JSON line to the metrics file.
fn write_entry(path: &PathBuf, entry: &MetricsEntry) -> std::io::Result<()> {
    use std::io::Write;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Check file size for rotation
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() > 10 * 1024 * 1024 {
            // Rotate: rename current to .1
            let backup = path.with_extension("jsonl.1");
            let _ = std::fs::rename(path, backup);
        }
    }

    // Build the JSON line with timestamp
    let mut json_value = serde_json::to_value(entry).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = json_value.as_object_mut() {
        obj.insert("ts".to_string(), serde_json::Value::String(Utc::now().to_rfc3339()));
    }

    let line = serde_json::to_string(&json_value).unwrap_or_default();

    // Append
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    writeln!(file, "{}", line)?;
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test test_metrics -- --nocapture 2>&1 | tail -10`
Expected: all 3 tests pass

- [ ] **Step 6: Commit**

```bash
git add src/metrics.rs src/lib.rs tests/test_metrics.rs
git commit -m "feat: add JSONL metrics logger with fire-and-forget writes"
```

---

## Task 10: Wire Metrics into Server Handlers

**Files:**
- Modify: `src/server.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `MetricsLogger` to `UnifiedSearchServer`**

In `src/server.rs`, add the metrics import and field:

```rust
use crate::metrics::{MetricsLogger, MetricsEntry};
```

Add to `UnifiedSearchServer`:

```rust
pub struct UnifiedSearchServer {
    orchestrator: SearchOrchestrator,
    jira_source: Option<JiraSource>,
    confluence_source: Option<ConfluenceSource>,
    slack_source: Option<SlackSource>,
    metrics: Option<MetricsLogger>,
}
```

Update `new()` to accept `metrics: Option<MetricsLogger>`.

- [ ] **Step 2: Emit metrics in `handle_unified_search`**

After building the response, add:

```rust
        // Emit metrics
        if let Some(ref metrics) = self.metrics {
            let sources_queried: Vec<String> = response.per_source_stats
                .iter()
                .map(|s| s.source.clone())
                .collect();
            metrics.log(MetricsEntry::Search {
                tool: "unified_search".to_string(),
                query: search_query.text.clone(),
                sources_queried,
                total_results: response.results.len(),
                deduped_results: response.results.len(),
                total_ms: response.query_time_ms,
            }).await;
        }
```

- [ ] **Step 3: Emit metrics in `handle_get_detail`**

At the end of `handle_get_detail`, before returning the result string:

```rust
        // Emit metrics (at the end, wrap the match result)
        if let Some(ref metrics) = self.metrics {
            // Build entry based on what was resolved
            let entry = MetricsEntry::Detail {
                tool: "get_detail".to_string(),
                identifier: identifier.clone(),
                detected_source: format!("{:?}", source_type),
                explicit_source: source.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                comments_returned: 0, // simplified for now
                error: None,
            };
            metrics.log(entry).await;
        }
```

Add `let start = std::time::Instant::now();` at the top of `handle_get_detail`.

- [ ] **Step 4: Enhance response footer with per-source stats**

In `handle_unified_search`, replace the existing footer:

```rust
        // Footer: per-source breakdown
        if !response.per_source_stats.is_empty() {
            let parts: Vec<String> = response.per_source_stats.iter().map(|s| {
                let err = s.error.as_deref().map(|e| format!(", err: {}", e)).unwrap_or_default();
                format!("{} ({}ms, {} results, {} comments{})",
                    s.source, s.latency_ms, s.result_count, s.comment_count, err)
            }).collect();
            let _ = write!(md, "**Sources**: {} | **Total**: {}ms", parts.join(" | "), response.query_time_ms);
        } else {
            let _ = write!(md, "**Sources queried**: {} | **Time**: {}ms",
                response.total_sources_queried, response.query_time_ms);
        }
```

- [ ] **Step 5: Update `main.rs` to create `MetricsLogger`**

```rust
    let metrics_path = shellexpand::tilde("~/.unified-search/metrics.jsonl").to_string();
    let metrics = MetricsLogger::new(std::path::PathBuf::from(metrics_path));
    
    let server = UnifiedSearchServer::new(
        orchestrator, jira_detail, confluence_detail, slack_detail, Some(metrics),
    );
```

- [ ] **Step 6: Update tests to pass `None` for metrics**

In `tests/test_server.rs`, update `build_server`:

```rust
fn build_server(sources: Vec<Box<dyn SearchSource>>) -> UnifiedSearchServer {
    let orchestrator = SearchOrchestrator::new(sources, default_config());
    UnifiedSearchServer::new(orchestrator, None, None, None, None)
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/server.rs src/main.rs tests/test_server.rs
git commit -m "feat: wire metrics logging into all MCP tool handlers"
```

---

## Task 11: Per-Source Stats in Orchestrator

**Files:**
- Modify: `src/core.rs`
- Modify: `tests/test_core.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/test_core.rs`:

```rust
#[tokio::test]
async fn per_source_stats_populated() {
    let source_a = MockSource::new(
        "slack",
        vec![make_result("slack", "msg1", 0.9)],
    );
    let source_b = MockSource::new(
        "jira",
        vec![
            make_result("jira", "j1", 0.8),
            make_result("jira", "j2", 0.6),
        ],
    );

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source_a), boxed(source_b)],
        default_config(),
    );

    let response = orchestrator.search(&query("test")).await;

    assert_eq!(
        response.per_source_stats.len(), 2,
        "Expected 2 per-source stats entries"
    );

    let slack_stats = response.per_source_stats.iter().find(|s| s.source == "slack").unwrap();
    assert_eq!(slack_stats.result_count, 1);
    assert!(slack_stats.latency_ms > 0 || slack_stats.latency_ms == 0); // just exists
    assert!(slack_stats.error.is_none());

    let jira_stats = response.per_source_stats.iter().find(|s| s.source == "jira").unwrap();
    assert_eq!(jira_stats.result_count, 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test per_source_stats_populated -- --nocapture 2>&1 | head -10`
Expected: FAIL — `per_source_stats` is empty (populated with `vec![]` in task 1)

- [ ] **Step 3: Populate per-source stats in orchestrator**

In `src/core.rs`, modify the fan-out to track per-source timing:

Replace the handle collection loop (Step 3, around line 78) to collect stats:

```rust
        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut per_source_stats: Vec<crate::models::PerSourceStats> = Vec::new();

        for handle in handles {
            match handle.await {
                Ok((source_name, timeout_result)) => match timeout_result {
                    Ok(search_result) => match search_result {
                        Ok(results) => {
                            let count = results.len();
                            let comment_count: usize = results.iter()
                                .filter_map(|r| r.metadata.get("comment_count"))
                                .filter_map(|c| c.parse::<usize>().ok())
                                .sum();
                            per_source_stats.push(crate::models::PerSourceStats {
                                source: source_name,
                                latency_ms: 0, // TODO: track actual per-source latency
                                result_count: count,
                                comment_count,
                                error: None,
                            });
                            all_results.extend(results);
                        }
                        Err(e) => {
                            let msg = format!("{}", e);
                            per_source_stats.push(crate::models::PerSourceStats {
                                source: source_name.clone(),
                                latency_ms: 0,
                                result_count: 0,
                                comment_count: 0,
                                error: Some(msg.clone()),
                            });
                            warnings.push(format!("Source '{}' failed: {}", source_name, msg));
                        }
                    },
                    Err(_) => {
                        per_source_stats.push(crate::models::PerSourceStats {
                            source: source_name.clone(),
                            latency_ms: self.config.timeout_seconds * 1000,
                            result_count: 0,
                            comment_count: 0,
                            error: Some("timeout".to_string()),
                        });
                        warnings.push(format!(
                            "Source '{}' timed out after {}s",
                            source_name, self.config.timeout_seconds
                        ));
                    }
                },
                Err(_join_error) => {
                    warnings.push("Source task panicked or crashed".to_string());
                }
            }
        }
```

And use it in the response construction:

```rust
        UnifiedSearchResponse {
            results: deduped,
            warnings,
            total_sources_queried,
            query_time_ms,
            per_source_stats,
        }
```

To track actual per-source latency, wrap the search call inside the `tokio::spawn` with timing:

```rust
            let handle = tokio::spawn(async move {
                let name = source.name().to_string();
                let source_start = std::time::Instant::now();
                let result = tokio::time::timeout(timeout_dur, source.search(&q)).await;
                let latency_ms = source_start.elapsed().as_millis() as u64;
                (name, result, latency_ms)
            });
```

Update the collection loop to destructure `(source_name, timeout_result, latency)` and use `latency` in `PerSourceStats`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_core -- --nocapture 2>&1 | tail -10`
Expected: all core tests pass

- [ ] **Step 5: Commit**

```bash
git add src/core.rs tests/test_core.rs
git commit -m "feat: populate per-source stats (latency, result count, comments) in orchestrator"
```

---

## Task 12: Stats CLI (`--stats`)

**Files:**
- Create: `src/stats.rs`
- Modify: `src/main.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add `mod stats;` to `lib.rs`**

```rust
pub mod stats;
```

- [ ] **Step 2: Implement `src/stats.rs`**

```rust
use std::path::PathBuf;

use chrono::{DateTime, Utc, Duration};

/// Run the stats report, reading metrics from the given path.
/// Optionally scans Claude Code conversation logs for bypass detection.
pub fn run_stats(metrics_path: &str, days: u64) {
    let cutoff = Utc::now() - Duration::days(days as i64);
    let path = PathBuf::from(shellexpand::tilde(metrics_path).to_string());

    println!("=== Unified Search Adoption Report (last {} days) ===\n", days);

    // Read own metrics
    let entries = read_metrics(&path, &cutoff);
    if entries.is_empty() {
        println!("No metrics found in {}", path.display());
        println!("Metrics are recorded automatically when the MCP server handles tool calls.");
        return;
    }

    // Categorize
    let mut search_calls: Vec<&serde_json::Value> = Vec::new();
    let mut detail_calls: Vec<&serde_json::Value> = Vec::new();

    for entry in &entries {
        match entry.get("tool").and_then(|v| v.as_str()) {
            Some("unified_search") | Some("search_source") => search_calls.push(entry),
            Some("get_detail") => detail_calls.push(entry),
            _ => {}
        }
    }

    // Report tool calls
    println!("Tool Calls:");
    report_tool_stats("  unified_search", &search_calls, "unified_search");
    report_tool_stats("  search_source", &search_calls, "search_source");
    report_tool_stats("  get_detail", &detail_calls, "get_detail");

    // Bypasses from Claude Code logs
    println!("\nBypasses (Claude used individual MCPs for search/read):");
    let bypass_counts = scan_claude_code_logs(&cutoff);
    if bypass_counts.is_empty() {
        println!("  (Claude Code logs not found or no bypasses detected)");
    } else {
        for (tool, count) in &bypass_counts {
            println!("  {}: {} calls", tool, count);
        }
    }

    // Adoption rate
    let total_unified = search_calls.len() + detail_calls.len();
    let total_bypasses: usize = bypass_counts.values().sum();
    let total = total_unified + total_bypasses;
    if total > 0 {
        let rate = (total_unified as f64 / total as f64) * 100.0;
        println!(
            "\nAdoption Rate: {:.0}% ({} unified / {} total search-like operations)",
            rate, total_unified, total
        );
    }

    println!();
}

fn read_metrics(path: &PathBuf, cutoff: &DateTime<Utc>) -> Vec<serde_json::Value> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    content
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| {
            entry
                .get("ts")
                .and_then(|ts| ts.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc) >= *cutoff)
                .unwrap_or(false)
        })
        .collect()
}

fn report_tool_stats(label: &str, entries: &[&serde_json::Value], tool_name: &str) {
    let matching: Vec<&&serde_json::Value> = entries
        .iter()
        .filter(|e| e.get("tool").and_then(|v| v.as_str()) == Some(tool_name))
        .collect();

    if matching.is_empty() {
        println!("{}:  0 calls", label);
        return;
    }

    let latencies: Vec<u64> = matching
        .iter()
        .filter_map(|e| e.get("total_ms").or(e.get("latency_ms")))
        .filter_map(|v| v.as_u64())
        .collect();

    if latencies.is_empty() {
        println!("{}:  {} calls", label, matching.len());
        return;
    }

    let mut sorted = latencies.clone();
    sorted.sort();
    let avg = sorted.iter().sum::<u64>() / sorted.len() as u64;
    let p50 = sorted[sorted.len() / 2];
    let p95_idx = (sorted.len() as f64 * 0.95).ceil() as usize - 1;
    let p95 = sorted[p95_idx.min(sorted.len() - 1)];

    println!(
        "{}:  {} calls  (avg {}ms, p50 {}ms, p95 {}ms)",
        label,
        matching.len(),
        avg,
        p50,
        p95
    );
}

fn scan_claude_code_logs(cutoff: &DateTime<Utc>) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();

    let base = shellexpand::tilde("~/.claude/projects").to_string();
    let base_path = PathBuf::from(&base);
    if !base_path.exists() {
        return counts;
    }

    // Walk conversation directories looking for JSONL files
    let bypass_tools = [
        "jira_get",
        "mcp__jira__jira_get",
        "conf_get",
        "mcp__claude_ai_Slack__authenticate",
    ];

    if let Ok(entries) = glob_conversation_files(&base_path) {
        for file_path in entries {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                for line in content.lines() {
                    if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                        // Check timestamp
                        let in_range = entry
                            .get("timestamp")
                            .or(entry.get("ts"))
                            .and_then(|ts| ts.as_str())
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&Utc) >= *cutoff)
                            .unwrap_or(true); // include if no timestamp

                        if !in_range {
                            continue;
                        }

                        // Check for tool_use with bypass tool names
                        if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
                            for bypass in &bypass_tools {
                                if name.contains(bypass) {
                                    *counts.entry(name.to_string()).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    counts
}

fn glob_conversation_files(base: &PathBuf) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Look for conversations subdirectory
                let convos = path.join("conversations");
                if convos.exists() {
                    if let Ok(conv_entries) = std::fs::read_dir(&convos) {
                        for conv in conv_entries.flatten() {
                            let conv_path = conv.path();
                            if conv_path.extension().map_or(false, |e| e == "jsonl") {
                                files.push(conv_path);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(files)
}
```

- [ ] **Step 3: Wire `--stats` in `main.rs`**

Add at the top of `main()`, after `verify` check:

```rust
    let stats = args.iter().any(|a| a == "--stats");
    let stats_days = args
        .iter()
        .position(|a| a == "--days")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(7);

    if stats {
        let metrics_path = "~/.unified-search/metrics.jsonl";
        unified_search_mcp::stats::run_stats(metrics_path, stats_days);
        return;
    }
```

- [ ] **Step 4: Run the full test suite**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/stats.rs src/lib.rs src/main.rs
git commit -m "feat: add --stats CLI mode for adoption and performance reporting"
```

---

## Task 13: Update Config for Metrics Path

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add `metrics_path` to `ServerConfig`**

```rust
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub max_results: usize,
    pub timeout_seconds: u64,
    pub log_level: String,
    pub metrics_path: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "unified-search".to_string(),
            max_results: 20,
            timeout_seconds: 10,
            log_level: "info".to_string(),
            metrics_path: "~/.unified-search/metrics.jsonl".to_string(),
        }
    }
}
```

Add to `RawServerConfig`:

```rust
    metrics_path: Option<String>,
```

Wire it in the `load()` function:

```rust
            metrics_path: s.metrics_path.unwrap_or_else(|| "~/.unified-search/metrics.jsonl".to_string()),
```

- [ ] **Step 2: Use config path in `main.rs`**

Replace the hardcoded path:

```rust
    let metrics_path = shellexpand::tilde(&app_config.server.metrics_path).to_string();
```

- [ ] **Step 3: Run all tests**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: add configurable metrics_path to server config"
```

---

## Task 14: Update MCP Instructions and Tool Descriptions

**Files:**
- Modify: `src/mcp.rs`

- [ ] **Step 1: Update server instructions to mention `get_detail`**

In `src/mcp.rs`, update the `with_instructions` string:

```rust
            .with_instructions(
                "PRIORITY: This tool takes priority over individual Confluence, JIRA, or Slack \
                 MCP tools for ANY search or lookup operation. Always use unified_search FIRST \
                 before falling back to individual conf_get, jira_get, or other source-specific \
                 tools. unified_search queries Slack messages, Confluence pages, JIRA tickets, \
                 and local code/docs in parallel — use it for questions like 'what did we decide \
                 about X?', 'where is Y documented?', 'who worked on Z?', 'find discussions \
                 about X', or any cross-system lookup. Use search_source only when you know the \
                 answer is in one specific system. Use get_detail to fetch full content for a \
                 specific JIRA ticket (FIN-1234), Confluence page, or Slack thread URL — this \
                 replaces jira_get, conf_get, and direct Slack MCP calls for single-item lookups.",
            )
```

- [ ] **Step 2: Update `unified_search` tool description to mention comments**

```rust
    #[tool(description = "Search across Slack messages, Confluence pages, JIRA tickets, and local code/docs in parallel. Returns a ranked Markdown table with results from all sources, including comments. Use for decisions, discussions, documentation, or code related to a topic. Also use instead of jira_get or conf_get for searching — this tool searches all systems at once. Example queries: 'broadcast threshold decision', 'auth middleware migration', 'FIN-10384 context'.")]
```

- [ ] **Step 3: Run all tests to make sure nothing broke**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/mcp.rs
git commit -m "feat: update MCP instructions and tool descriptions for v0.2 capabilities"
```

---

## Task 15: Final Integration Test

**Files:**
- Modify: `tests/test_integration.rs`

- [ ] **Step 1: Read existing integration test**

Read `tests/test_integration.rs` to understand the current pattern.

- [ ] **Step 2: Add integration test for get_detail flow**

```rust
// Add test that exercises resolve -> source -> markdown for JIRA get_detail
#[tokio::test]
async fn integration_get_detail_jira() {
    // Test auto-detection + JIRA detail fetch via wiremock
    use unified_search_mcp::resolve::detect_source;

    let (source_type, parsed) = detect_source("FIN-1234").unwrap();
    assert!(matches!(source_type, unified_search_mcp::resolve::SourceType::Jira));

    // The actual HTTP call is tested in test_jira.rs
    // This test verifies the end-to-end detection → delegation path
}
```

- [ ] **Step 3: Run full test suite**

Run: `cargo test 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 4: Build release binary and verify**

Run: `cargo build --release 2>&1 | tail -5`
Expected: clean build

Run: `./target/release/unified-search-mcp --verify --config config.yaml`
Expected: preflight passes (assuming env vars set)

- [ ] **Step 5: Commit**

```bash
git add tests/test_integration.rs
git commit -m "test: add integration test for get_detail auto-detection flow"
```

---

## Summary

| Task | Description | Commits |
|---|---|---|
| 1 | Extend models (PerSourceStats) | 1 |
| 2 | JIRA comment extraction from search | 1 |
| 3 | Confluence comment enrichment (parallel) | 1 |
| 4 | Identifier auto-detection (resolve.rs) | 1 |
| 5 | JIRA get_detail_issue | 1 |
| 6 | Confluence get_detail_page | 1 |
| 7 | Slack get_detail_thread | 1 |
| 8 | Wire get_detail MCP tool | 1 |
| 9 | Metrics logger | 1 |
| 10 | Wire metrics into handlers | 1 |
| 11 | Per-source stats in orchestrator | 1 |
| 12 | Stats CLI (--stats) | 1 |
| 13 | Config for metrics path | 1 |
| 14 | Update MCP instructions | 1 |
| 15 | Final integration test | 1 |
| **Total** | | **15 commits** |
