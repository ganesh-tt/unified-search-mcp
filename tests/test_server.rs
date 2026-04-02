use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use unified_search_mcp::core::{OrchestratorConfig, SearchOrchestrator};
use unified_search_mcp::models::*;
use unified_search_mcp::server::UnifiedSearchServer;
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Mock source (same pattern as test_core.rs)
// ===========================================================================

struct MockSource {
    source_name: String,
    description_text: String,
    results: Vec<SearchResult>,
    healthy: bool,
}

impl MockSource {
    fn new(name: &str, results: Vec<SearchResult>) -> Self {
        Self {
            source_name: name.to_string(),
            description_text: format!("Mock source: {name}"),
            results,
            healthy: true,
        }
    }

    fn unhealthy(mut self) -> Self {
        self.healthy = false;
        self
    }
}

#[async_trait]
impl SearchSource for MockSource {
    fn name(&self) -> &str {
        &self.source_name
    }

    fn description(&self) -> &str {
        &self.description_text
    }

    async fn health_check(&self) -> SourceHealth {
        if self.healthy {
            SourceHealth {
                source: self.source_name.clone(),
                status: HealthStatus::Healthy,
                message: Some("OK".to_string()),
                latency_ms: Some(1),
            }
        } else {
            SourceHealth {
                source: self.source_name.clone(),
                status: HealthStatus::Unavailable,
                message: Some("Mock unhealthy".to_string()),
                latency_ms: None,
            }
        }
    }

    async fn search(&self, _query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        Ok(self.results.clone())
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn boxed(source: impl SearchSource + 'static) -> Box<dyn SearchSource> {
    Box::new(source)
}

fn make_result(source: &str, title: &str, relevance: f32) -> SearchResult {
    SearchResult {
        source: source.to_string(),
        title: title.to_string(),
        snippet: format!("Snippet for {title}"),
        url: Some(format!("https://{source}.example.com/{title}")),
        timestamp: Some(Utc.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap()),
        relevance,
        metadata: HashMap::new(),
    }
}

fn default_config() -> OrchestratorConfig {
    OrchestratorConfig {
        timeout_seconds: 30,
        source_weights: HashMap::new(),
        max_results: 50,
    }
}

fn build_server(sources: Vec<Box<dyn SearchSource>>) -> UnifiedSearchServer {
    let orchestrator = SearchOrchestrator::new(sources, default_config(), 0);
    UnifiedSearchServer::new(orchestrator, None, None, None, None)
}

// ===========================================================================
// Test 1: tools_list_returns_all_four
// ===========================================================================

/// The server must expose 4 tool handler methods:
/// handle_unified_search, handle_search_source, handle_list_sources, handle_index_local.
/// We verify by calling each one and confirming they return non-empty strings.
#[tokio::test]
async fn tools_list_returns_all_four() {
    let server = build_server(vec![
        boxed(MockSource::new("slack", vec![make_result("slack", "msg1", 0.9)])),
    ]);

    // All four handlers should be callable and return non-empty output
    let r1 = server
        .handle_unified_search("test".to_string(), None, None, false)
        .await;
    let r2 = server
        .handle_search_source("slack".to_string(), "test".to_string(), None, false)
        .await;
    let r3 = server.handle_list_sources().await;
    let r4 = server.handle_index_local().await;

    assert!(!r1.is_empty(), "handle_unified_search should return non-empty");
    assert!(!r2.is_empty(), "handle_search_source should return non-empty");
    assert!(!r3.is_empty(), "handle_list_sources should return non-empty");
    assert!(!r4.is_empty(), "handle_index_local should return non-empty");
}

// ===========================================================================
// Test 2: unified_search_tool_dispatch
// ===========================================================================

/// handle_unified_search should return a Markdown table with header row,
/// separator, data rows, and footer with warnings/sources/time.
#[tokio::test]
async fn unified_search_tool_dispatch() {
    let server = build_server(vec![
        boxed(MockSource::new(
            "confluence",
            vec![
                make_result("confluence", "Page One", 0.9),
                make_result("confluence", "Page Two", 0.7),
            ],
        )),
        boxed(MockSource::new(
            "slack",
            vec![make_result("slack", "Message", 0.8)],
        )),
    ]);

    let output = server
        .handle_unified_search("test query".to_string(), None, None, false)
        .await;

    // Should contain Markdown table header
    assert!(
        output.contains("| # |"),
        "Expected Markdown table header, got:\n{output}"
    );
    assert!(
        output.contains("| Source |"),
        "Expected Source column header, got:\n{output}"
    );
    assert!(
        output.contains("|---|"),
        "Expected table separator row, got:\n{output}"
    );

    // Should contain results from both sources
    assert!(
        output.contains("confluence"),
        "Expected 'confluence' in results, got:\n{output}"
    );
    assert!(
        output.contains("slack"),
        "Expected 'slack' in results, got:\n{output}"
    );

    // Should contain footer with sources queried and time
    assert!(
        output.contains("**Sources queried**"),
        "Expected sources queried footer, got:\n{output}"
    );
    assert!(
        output.contains("**Time**"),
        "Expected time footer, got:\n{output}"
    );
}

// ===========================================================================
// Test 3: search_source_tool_dispatch
// ===========================================================================

/// handle_search_source("slack", "test") should return JSON array of results
/// from only the slack source.
#[tokio::test]
async fn search_source_tool_dispatch() {
    let server = build_server(vec![
        boxed(MockSource::new(
            "slack",
            vec![
                make_result("slack", "slack_msg1", 0.9),
                make_result("slack", "slack_msg2", 0.7),
            ],
        )),
        boxed(MockSource::new(
            "confluence",
            vec![make_result("confluence", "conf_page", 0.8)],
        )),
    ]);

    let output = server
        .handle_search_source("slack".to_string(), "test".to_string(), None, false)
        .await;

    // Should be valid JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("handle_search_source output should be valid JSON");

    // Should be an array
    assert!(
        parsed.is_array(),
        "Expected JSON array, got: {parsed}"
    );

    let arr = parsed.as_array().unwrap();
    // Should have results from slack only
    assert!(
        !arr.is_empty(),
        "Expected at least one result from slack"
    );

    // All results should be from slack source
    for item in arr {
        let source = item["source"].as_str().unwrap_or("");
        assert_eq!(
            source, "slack",
            "Expected only slack results, got source='{source}'"
        );
    }
}

// ===========================================================================
// Test 4: list_sources_tool_dispatch
// ===========================================================================

/// handle_list_sources should return Markdown list with health status for each source.
#[tokio::test]
async fn list_sources_tool_dispatch() {
    let server = build_server(vec![
        boxed(MockSource::new("slack", vec![])),
        boxed(MockSource::new("confluence", vec![]).unhealthy()),
    ]);

    let output = server.handle_list_sources().await;

    // Should list both sources
    assert!(
        output.contains("slack"),
        "Expected 'slack' in health list, got:\n{output}"
    );
    assert!(
        output.contains("confluence"),
        "Expected 'confluence' in health list, got:\n{output}"
    );

    // Should contain health status indicators
    assert!(
        output.contains("healthy") || output.contains("Healthy"),
        "Expected healthy status for slack, got:\n{output}"
    );
    assert!(
        output.contains("unavailable") || output.contains("Unavailable"),
        "Expected unavailable status for confluence, got:\n{output}"
    );
}

// ===========================================================================
// Test 5: unknown_tool_returns_error
// ===========================================================================

/// handle_search_source with a non-existent source name should return a
/// message indicating no results or an error about the unknown source.
#[tokio::test]
async fn unknown_source_returns_empty() {
    let server = build_server(vec![
        boxed(MockSource::new("slack", vec![make_result("slack", "msg", 0.9)])),
    ]);

    let output = server
        .handle_search_source("nonexistent".to_string(), "test".to_string(), None, false)
        .await;

    // Should be valid JSON with an empty array (no source matches)
    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("output should be valid JSON");
    let arr = parsed.as_array().expect("Expected JSON array");
    assert!(
        arr.is_empty(),
        "Expected empty results for unknown source, got {} results",
        arr.len()
    );
}

// ===========================================================================
// Test 6: index_local_returns_not_enabled
// ===========================================================================

/// handle_index_local should return a message indicating vector search is
/// not enabled (Phase 1 stub).
#[tokio::test]
async fn index_local_returns_not_enabled() {
    let server = build_server(vec![]);

    let output = server.handle_index_local().await;

    let lower = output.to_lowercase();
    assert!(
        lower.contains("vector search not enabled")
            || lower.contains("not enabled")
            || lower.contains("not available"),
        "Expected 'not enabled' message, got:\n{output}"
    );
}

// ===========================================================================
// Test 7: get_detail_returns_error_for_unknown_identifier
// ===========================================================================

/// handle_get_detail with an unrecognized identifier (no JIRA key pattern,
/// no URL) should return an error message.
#[tokio::test]
async fn get_detail_returns_error_for_unknown_identifier() {
    let server = build_server(vec![]);
    let output = server
        .handle_get_detail("random text".to_string(), None, None)
        .await;
    let lower = output.to_lowercase();
    assert!(
        lower.contains("could not detect")
            || lower.contains("not recognized")
            || lower.contains("error"),
        "Expected error for unrecognized identifier, got:\n{output}"
    );
}

// ===========================================================================
// Test 8: get_detail_jira_key_without_source_returns_not_configured
// ===========================================================================

/// handle_get_detail with a valid JIRA key but no JIRA source configured
/// should return a "not configured" error.
#[tokio::test]
async fn get_detail_jira_key_without_source_returns_not_configured() {
    let server = build_server(vec![]);
    let output = server
        .handle_get_detail("FIN-1234".to_string(), None, None)
        .await;
    let lower = output.to_lowercase();
    assert!(
        lower.contains("not configured"),
        "Expected 'not configured' error for JIRA key without source, got:\n{output}"
    );
}
