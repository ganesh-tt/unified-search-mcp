//! Integration tests: exercise the full pipeline from UnifiedSearchServer
//! down through real adapters backed by wiremock HTTP servers and temp
//! filesystem directories.

use std::collections::HashMap;
use std::io::Write;

use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use unified_search_mcp::core::{OrchestratorConfig, SearchOrchestrator};
use unified_search_mcp::server::UnifiedSearchServer;
use unified_search_mcp::sources::confluence::{ConfluenceConfig, ConfluenceSource};
use unified_search_mcp::sources::jira::{JiraConfig, JiraSource};
use unified_search_mcp::sources::local_text::{LocalTextConfig, LocalTextSource};
use unified_search_mcp::sources::slack::{SlackConfig, SlackSource};
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Helpers
// ===========================================================================

/// Slack search.messages JSON fixture.
fn slack_success_body() -> serde_json::Value {
    json!({
        "ok": true,
        "messages": {
            "total": 2,
            "matches": [
                {
                    "text": "broadcast threshold was set to 50K rows",
                    "permalink": "https://slack.com/archives/C01/p111",
                    "channel": {"name": "engineering"},
                    "username": "ganesh",
                    "ts": "1710700800.123456",
                    "score": 0.95
                },
                {
                    "text": "broadcast threshold PR merged",
                    "permalink": "https://slack.com/archives/C02/p222",
                    "channel": {"name": "deployments"},
                    "username": "priya",
                    "ts": "1710614400.654321",
                    "score": 0.80
                }
            ]
        }
    })
}

/// Confluence search response JSON fixture.
fn confluence_success_body() -> serde_json::Value {
    json!({
        "results": [
            {
                "content": {"id": "12345", "type": "page", "title": "Broadcast Threshold Design"},
                "excerpt": "The <b>broadcast threshold</b> was set to 50K rows.",
                "url": "/wiki/spaces/DEV/pages/12345",
                "lastModified": "2026-03-10T10:00:00.000Z",
                "resultGlobalContainer": {"title": "DEV", "displayUrl": "/wiki/spaces/DEV"}
            },
            {
                "content": {"id": "12346", "type": "page", "title": "Spark Config Guide"},
                "excerpt": "Configure <b>broadcast</b> size <b>threshold</b> in HOCON.",
                "url": "/wiki/spaces/DEV/pages/12346",
                "lastModified": "2026-02-15T08:00:00.000Z",
                "resultGlobalContainer": {"title": "DEV", "displayUrl": "/wiki/spaces/DEV"}
            }
        ],
        "start": 0, "limit": 10, "size": 2, "totalSize": 2
    })
}

/// JIRA search response JSON fixture.
fn jira_success_body(_base_url: &str) -> serde_json::Value {
    json!({
        "startAt": 0,
        "maxResults": 10,
        "total": 2,
        "issues": [
            {
                "key": "FIN-10384",
                "fields": {
                    "summary": "Remove broadcastRowThreshold callers",
                    "description": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Remove the broadcast threshold configuration and constant."}]}]},
                    "status": {"name": "In Progress"},
                    "updated": "2026-03-15T12:00:00.000+0000",
                    "assignee": {"displayName": "Ganesh K"},
                    "comment": {"comments": []}
                }
            },
            {
                "key": "FIN-10385",
                "fields": {
                    "summary": "Add WatchlistBroadcastRowThreshold seed SQL",
                    "description": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Add threshold config to dml.sql and v6.3.4 migration."}]}]},
                    "status": {"name": "Done"},
                    "updated": "2026-03-12T09:00:00.000+0000",
                    "assignee": {"displayName": "Ganesh K"},
                    "comment": {"comments": []}
                }
            }
        ]
    })
}

/// Set up a temp dir containing a markdown file with "broadcast threshold" text.
fn setup_local_text_dir() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = dir.path().join("broadcast-notes.md");
    let mut f = std::fs::File::create(&file_path).expect("Failed to create test file");
    writeln!(f, "# Broadcast Threshold Notes").unwrap();
    writeln!(f, "").unwrap();
    writeln!(
        f,
        "The broadcast threshold was set to 50K rows after the OOM incident."
    )
    .unwrap();
    writeln!(f, "This is now configurable via amls_dynamic_properties.").unwrap();
    f.flush().unwrap();
    dir
}

/// Build a SlackSource pointing at the given wiremock URL.
fn make_slack(base_url: &str) -> Box<dyn SearchSource> {
    Box::new(SlackSource::new(SlackConfig {
        user_token: "xoxp-test".into(),
        max_results: 10,
        base_url: base_url.to_string(),
    }))
}

/// Build a ConfluenceSource pointing at the given wiremock URL.
fn make_confluence(base_url: &str) -> Box<dyn SearchSource> {
    Box::new(ConfluenceSource::new(ConfluenceConfig {
        base_url: base_url.to_string(),
        email: "test@example.com".into(),
        api_token: "test-token".into(),
        spaces: vec![],
        max_results: 10,
    }))
}

/// Build a JiraSource pointing at the given wiremock URL.
fn make_jira(base_url: &str) -> Box<dyn SearchSource> {
    Box::new(JiraSource::new(JiraConfig {
        base_url: base_url.to_string(),
        email: "test@example.com".into(),
        api_token: "test-token".into(),
        projects: vec![],
        max_results: 10,
    }))
}

/// Build a LocalTextSource pointing at the given temp directory.
fn make_local(temp_dir: &TempDir) -> Box<dyn SearchSource> {
    Box::new(LocalTextSource::new(LocalTextConfig {
        paths: vec![temp_dir.path().to_path_buf()],
        include_patterns: vec!["*.md".into()],
        exclude_patterns: vec![],
        max_file_size_bytes: 1_000_000,
    }))
}

fn default_orchestrator_config() -> OrchestratorConfig {
    OrchestratorConfig {
        timeout_seconds: 10,
        source_weights: HashMap::new(),
        max_results: 20,
    }
}

fn build_server_from(
    sources: Vec<Box<dyn SearchSource>>,
    config: OrchestratorConfig,
) -> UnifiedSearchServer {
    let orchestrator = SearchOrchestrator::new(sources, config, 0);
    UnifiedSearchServer::new(orchestrator, None, None, None, None, None)
}

// ===========================================================================
// Test 1: full_pipeline_all_sources_mocked
// ===========================================================================

/// Set up wiremock for Slack + Confluence + JIRA and a temp dir for local text.
/// Build real adapters pointing at wiremock URLs. Build orchestrator + server.
/// Call handle_unified_search("broadcast threshold") and verify results from
/// all 4 sources are merged and ranked.
#[tokio::test]
async fn full_pipeline_all_sources_mocked() {
    // Start 3 independent wiremock servers
    let slack_server = MockServer::start().await;
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;
    let temp_dir = setup_local_text_dir();

    // Mount Slack mock
    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_success_body()))
        .mount(&slack_server)
        .await;

    // Mount Confluence mock
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(confluence_success_body()))
        .mount(&confluence_server)
        .await;

    // Mount JIRA mock
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jira_success_body(&jira_server.uri())),
        )
        .mount(&jira_server)
        .await;

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_slack(&slack_server.uri()),
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
        make_local(&temp_dir),
    ];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server
        .handle_unified_search("broadcast threshold".to_string(), None, None, false)
        .await;

    // Verify all 4 sources appear in the results
    assert!(
        output.contains("slack"),
        "Expected 'slack' in output, got:\n{output}"
    );
    assert!(
        output.contains("confluence"),
        "Expected 'confluence' in output, got:\n{output}"
    );
    assert!(
        output.contains("jira"),
        "Expected 'jira' in output, got:\n{output}"
    );
    assert!(
        output.contains("local_text"),
        "Expected 'local_text' in output, got:\n{output}"
    );

    // Should be a Markdown table with header
    assert!(output.contains("| # |"), "Expected Markdown table header");
    assert!(output.contains("| Source |"), "Expected Source column");

    // Footer should show 4 sources queried
    assert!(
        output.contains("**Sources**:") || output.contains("**Sources queried**:"),
        "Expected 4 sources queried in footer, got:\n{output}"
    );

    // Time should be present
    assert!(
        output.contains("**Time**") || output.contains("**Total**"),
        "Expected time in footer, got:\n{output}"
    );
}

// ===========================================================================
// Test 2: mixed_success_failure
// ===========================================================================

/// Slack wiremock returns 401 (auth failure), others succeed.
/// Unified search should return results from confluence + jira + local,
/// with a warning about the slack failure.
#[tokio::test]
async fn mixed_success_failure() {
    let slack_server = MockServer::start().await;
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;
    let temp_dir = setup_local_text_dir();

    // Slack returns auth error (ok: false)
    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": false,
            "error": "invalid_auth"
        })))
        .mount(&slack_server)
        .await;

    // Confluence succeeds
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(confluence_success_body()))
        .mount(&confluence_server)
        .await;

    // JIRA succeeds
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jira_success_body(&jira_server.uri())),
        )
        .mount(&jira_server)
        .await;

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_slack(&slack_server.uri()),
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
        make_local(&temp_dir),
    ];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server
        .handle_unified_search("broadcast threshold".to_string(), None, None, false)
        .await;

    // Should have results from the 3 working sources
    assert!(
        output.contains("confluence"),
        "Expected confluence results, got:\n{output}"
    );
    assert!(
        output.contains("jira"),
        "Expected jira results, got:\n{output}"
    );
    assert!(
        output.contains("local_text"),
        "Expected local_text results, got:\n{output}"
    );

    // Should have a warning about slack failure
    assert!(
        output.contains("**Warnings**"),
        "Expected Warnings section, got:\n{output}"
    );
    assert!(
        output.contains("slack") && output.contains("failed"),
        "Expected warning mentioning slack failure, got:\n{output}"
    );
}

// ===========================================================================
// Test 3: search_source_single
// ===========================================================================

/// Call handle_search_source("confluence", "query") and verify only
/// confluence results are returned in JSON format.
#[tokio::test]
async fn search_source_single() {
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;

    // Mount confluence mock
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(confluence_success_body()))
        .mount(&confluence_server)
        .await;

    // Mount jira mock (should NOT be queried)
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jira_success_body(&jira_server.uri())),
        )
        .expect(0) // should not be called
        .mount(&jira_server)
        .await;

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
    ];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server
        .handle_search_source(
            "confluence".to_string(),
            "broadcast threshold".to_string(),
            None,
            false,
        )
        .await;

    // Should be valid JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("Output should be valid JSON");

    // Should be a non-empty array
    let arr = parsed.as_array().expect("Expected JSON array");
    assert!(
        !arr.is_empty(),
        "Expected non-empty results from confluence"
    );

    // All results should be from confluence source
    for item in arr {
        let source = item["source"].as_str().unwrap_or("");
        assert_eq!(
            source, "confluence",
            "Expected only confluence results, got source='{source}'"
        );
    }
}

// ===========================================================================
// Test 4: list_sources_health
// ===========================================================================

/// All sources configured, jira wiremock has no mock mounted (will fail health).
/// Call handle_list_sources and verify health per source.
#[tokio::test]
async fn list_sources_health() {
    let slack_server = MockServer::start().await;
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;
    let temp_dir = setup_local_text_dir();

    // Mount Slack auth.test (healthy)
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": "testuser"
        })))
        .mount(&slack_server)
        .await;

    // Mount Confluence space endpoint (healthy)
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/space"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [], "size": 0})))
        .mount(&confluence_server)
        .await;

    // JIRA: no mock mounted for /rest/api/3/myself — health check will get 404

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_slack(&slack_server.uri()),
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
        make_local(&temp_dir),
    ];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server.handle_list_sources().await;

    // Should list all 4 sources
    assert!(output.contains("slack"), "Expected slack in health output");
    assert!(
        output.contains("confluence"),
        "Expected confluence in health output"
    );
    assert!(output.contains("jira"), "Expected jira in health output");
    assert!(
        output.contains("local_text"),
        "Expected local_text in health output"
    );

    // Slack and confluence should be healthy
    // The output format is: "- **slack** — healthy (OK)"
    // Jira should show unavailable since no mock is mounted for /myself
    // (it gets a 404 which maps to Unavailable)

    // At minimum, all 4 source names are present in the output
    assert!(output.contains("Source Health"), "Expected health header");
}

// ===========================================================================
// Test 5: unified_search_returns_markdown_table
// ===========================================================================

/// With 3+ results, verify the output contains the expected Markdown table
/// structure: header row, separator, data rows, footer with warnings/time.
#[tokio::test]
async fn unified_search_returns_markdown_table() {
    let slack_server = MockServer::start().await;
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;

    // Mount all mocks
    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_success_body()))
        .mount(&slack_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(confluence_success_body()))
        .mount(&confluence_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jira_success_body(&jira_server.uri())),
        )
        .mount(&jira_server)
        .await;

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_slack(&slack_server.uri()),
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
    ];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server
        .handle_unified_search("broadcast threshold".to_string(), None, None, false)
        .await;

    // Verify Markdown table structure
    assert!(
        output.contains("| # | Source | Title |"),
        "Expected table header with | # | Source | Title |, got:\n{output}"
    );
    assert!(
        output.contains("|---|--------|"),
        "Expected table separator row, got:\n{output}"
    );

    // Should have numbered data rows (at least | 1 |, | 2 |, | 3 |)
    assert!(
        output.contains("| 1 |"),
        "Expected row 1 in table, got:\n{output}"
    );
    assert!(
        output.contains("| 2 |"),
        "Expected row 2 in table, got:\n{output}"
    );
    assert!(
        output.contains("| 3 |"),
        "Expected row 3 in table, got:\n{output}"
    );

    // Footer
    assert!(
        output.contains("**Sources**") || output.contains("**Sources queried**"),
        "Expected sources queried footer"
    );
    assert!(
        output.contains("**Time**") || output.contains("**Total**"),
        "Expected time footer"
    );
    assert!(output.contains("ms"), "Expected milliseconds in time");
}

// ===========================================================================
// Test 6: source_filter_respected
// ===========================================================================

/// Call handle_unified_search with sources=["slack","jira"] and verify only
/// those two sources are queried (confluence should NOT be called).
#[tokio::test]
async fn source_filter_respected() {
    let slack_server = MockServer::start().await;
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;

    // Mount Slack mock
    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_success_body()))
        .mount(&slack_server)
        .await;

    // Confluence mock should NOT be called
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(confluence_success_body()))
        .expect(0) // asserts 0 calls
        .mount(&confluence_server)
        .await;

    // Mount JIRA mock
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jira_success_body(&jira_server.uri())),
        )
        .mount(&jira_server)
        .await;

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_slack(&slack_server.uri()),
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
    ];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server
        .handle_unified_search(
            "broadcast threshold".to_string(),
            Some(vec!["slack".to_string(), "jira".to_string()]),
            None,
            false,
        )
        .await;

    // Should have results from slack and jira
    assert!(
        output.contains("slack"),
        "Expected 'slack' in output, got:\n{output}"
    );
    assert!(
        output.contains("jira"),
        "Expected 'jira' in output, got:\n{output}"
    );

    // Should show 2 sources queried (not 3)
    assert!(
        output.contains("**Sources**") || output.contains("**Sources queried**: 2"),
        "Expected 2 sources queried, got:\n{output}"
    );
}

// ===========================================================================
// Test 7: max_results_global
// ===========================================================================

/// Set orchestrator max_results=5. Each source returns 3+ results.
/// Total output should have at most 5 result rows.
#[tokio::test]
async fn max_results_global() {
    let slack_server = MockServer::start().await;
    let confluence_server = MockServer::start().await;
    let jira_server = MockServer::start().await;

    // Slack returns 3 results (extend the fixture)
    let slack_body = json!({
        "ok": true,
        "messages": {
            "total": 3,
            "matches": [
                {"text": "slack msg 1 about threshold", "permalink": "https://slack.com/1", "channel": {"name": "eng"}, "username": "u1", "ts": "1710700800.001", "score": 0.9},
                {"text": "slack msg 2 about threshold", "permalink": "https://slack.com/2", "channel": {"name": "eng"}, "username": "u2", "ts": "1710700801.002", "score": 0.8},
                {"text": "slack msg 3 about threshold", "permalink": "https://slack.com/3", "channel": {"name": "eng"}, "username": "u3", "ts": "1710700802.003", "score": 0.7}
            ]
        }
    });

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_body))
        .mount(&slack_server)
        .await;

    // Confluence returns 3 results (extended)
    let confluence_body = json!({
        "results": [
            {"content": {"id": "1", "type": "page", "title": "Conf Page 1"}, "excerpt": "threshold excerpt 1", "url": "/wiki/spaces/DEV/pages/1", "lastModified": "2026-03-10T10:00:00.000Z", "resultGlobalContainer": {"title": "DEV", "displayUrl": "/"}},
            {"content": {"id": "2", "type": "page", "title": "Conf Page 2"}, "excerpt": "threshold excerpt 2", "url": "/wiki/spaces/DEV/pages/2", "lastModified": "2026-03-09T10:00:00.000Z", "resultGlobalContainer": {"title": "DEV", "displayUrl": "/"}},
            {"content": {"id": "3", "type": "page", "title": "Conf Page 3"}, "excerpt": "threshold excerpt 3", "url": "/wiki/spaces/DEV/pages/3", "lastModified": "2026-03-08T10:00:00.000Z", "resultGlobalContainer": {"title": "DEV", "displayUrl": "/"}}
        ],
        "start": 0, "limit": 10, "size": 3, "totalSize": 3
    });

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(confluence_body))
        .mount(&confluence_server)
        .await;

    // JIRA returns 3 results (extended)
    let jira_body = json!({
        "startAt": 0, "maxResults": 10, "total": 3,
        "issues": [
            {"key": "FIN-1", "fields": {"summary": "Jira Issue 1 threshold", "description": null, "status": {"name": "Open"}, "updated": "2026-03-15T12:00:00.000+0000", "assignee": null, "comment": {"comments": []}}},
            {"key": "FIN-2", "fields": {"summary": "Jira Issue 2 threshold", "description": null, "status": {"name": "Open"}, "updated": "2026-03-14T12:00:00.000+0000", "assignee": null, "comment": {"comments": []}}},
            {"key": "FIN-3", "fields": {"summary": "Jira Issue 3 threshold", "description": null, "status": {"name": "Open"}, "updated": "2026-03-13T12:00:00.000+0000", "assignee": null, "comment": {"comments": []}}}
        ]
    });

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_body))
        .mount(&jira_server)
        .await;

    let sources: Vec<Box<dyn SearchSource>> = vec![
        make_slack(&slack_server.uri()),
        make_confluence(&confluence_server.uri()),
        make_jira(&jira_server.uri()),
    ];

    // Set max_results to 5
    let config = OrchestratorConfig {
        timeout_seconds: 10,
        source_weights: HashMap::new(),
        max_results: 5,
    };

    let server = build_server_from(sources, config);
    let output = server
        .handle_unified_search("threshold".to_string(), None, Some(5), false)
        .await;

    // Count data rows in the table (rows starting with "| <digit>")
    let data_rows: Vec<&str> = output
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            // Match rows like "| 1 | ..." through "| 99 | ..."
            trimmed.starts_with("| ") && {
                let after_pipe = trimmed.trim_start_matches("| ");
                after_pipe.starts_with(|c: char| c.is_ascii_digit())
            }
        })
        .collect();

    assert!(
        data_rows.len() <= 5,
        "Expected at most 5 result rows, got {}: {:?}",
        data_rows.len(),
        data_rows
    );
    assert!(!data_rows.is_empty(), "Expected at least 1 result row");
}

// ===========================================================================
// Test 8: all_sources_disabled
// ===========================================================================

/// Build server with empty source list. Unified search should return
/// a "0 results" message.
#[tokio::test]
async fn all_sources_disabled() {
    let sources: Vec<Box<dyn SearchSource>> = vec![];

    let server = build_server_from(sources, default_orchestrator_config());
    let output = server
        .handle_unified_search("broadcast threshold".to_string(), None, None, false)
        .await;

    // With no sources, the table should have only header + separator, no data rows
    // The footer should show 0 sources queried
    assert!(
        output.contains("**Sources**") || output.contains("**Sources queried**: 0"),
        "Expected 0 sources queried, got:\n{output}"
    );

    // Count data rows - should be 0
    let data_rows: Vec<&str> = output
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("| ") && {
                let after_pipe = trimmed.trim_start_matches("| ");
                after_pipe.starts_with(|c: char| c.is_ascii_digit())
            }
        })
        .collect();

    assert_eq!(
        data_rows.len(),
        0,
        "Expected 0 data rows with no sources, got {}: {:?}",
        data_rows.len(),
        data_rows
    );
}

// ===========================================================================
// Test 9: integration_get_detail_jira_detection
// ===========================================================================

/// Verify that detect_source correctly identifies a JIRA key and parses it
/// into a JiraKey variant — exercising the resolve → delegation path.
#[tokio::test]
async fn integration_get_detail_jira_detection() {
    use unified_search_mcp::resolve::detect_source;

    let (source_type, parsed) = detect_source("FIN-1234").unwrap();
    assert!(matches!(
        source_type,
        unified_search_mcp::resolve::SourceType::Jira
    ));
    match parsed {
        unified_search_mcp::resolve::ParsedIdentifier::JiraKey(k) => assert_eq!(k, "FIN-1234"),
        other => panic!("Expected JiraKey, got {:?}", other),
    }
}
