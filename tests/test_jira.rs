use chrono::{TimeZone, Utc};
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::matchers::{method, path, query_param_contains};
use wiremock::{Mock, MockServer, ResponseTemplate};

use unified_search_mcp::models::*;
use unified_search_mcp::sources::jira::{JiraConfig, JiraSource};
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Helpers
// ===========================================================================

fn make_config(base_url: &str) -> JiraConfig {
    JiraConfig {
        base_url: base_url.to_string(),
        email: "user@example.com".to_string(),
        api_token: "test-token".to_string(),
        projects: vec![],
        max_results: 25,
    }
}

fn make_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        max_results: 25,
        filters: SearchFilters::default(),
    }
}

/// Build a JIRA search API response JSON with the given issues.
fn jira_search_response(issues: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "startAt": 0,
        "maxResults": 25,
        "total": issues.len(),
        "issues": issues
    })
}

/// Build a single JIRA issue JSON.
fn jira_issue(
    key: &str,
    summary: &str,
    description: Option<serde_json::Value>,
    status: &str,
    assignee: Option<&str>,
    updated: &str,
) -> serde_json::Value {
    let assignee_val = assignee.map_or(serde_json::Value::Null, |name| {
        json!({"displayName": name})
    });
    let desc_val = description.unwrap_or(serde_json::Value::Null);
    json!({
        "key": key,
        "fields": {
            "summary": summary,
            "description": desc_val,
            "status": {"name": status},
            "assignee": assignee_val,
            "updated": updated,
            "comment": {"comments": []}
        }
    })
}

/// Build an ADF document with a single paragraph containing the given text.
fn adf_text(text: &str) -> serde_json::Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": text
            }]
        }]
    })
}

// ===========================================================================
// Test 1: successful_search_maps_results
// ===========================================================================

#[tokio::test]
async fn successful_search_maps_results() {
    let server = MockServer::start().await;

    let issues = vec![
        jira_issue(
            "FIN-100",
            "Fix OOM in broadcast",
            Some(adf_text("Broadcast was unbounded")),
            "In Progress",
            Some("Alice"),
            "2026-03-15T10:00:00.000+0000",
        ),
        jira_issue(
            "FIN-101",
            "Add retry logic",
            Some(adf_text("Retries for flaky calls")),
            "Open",
            Some("Bob"),
            "2026-03-14T09:00:00.000+0000",
        ),
        jira_issue(
            "PLAT-200",
            "Upgrade Spark",
            Some(adf_text("Spark 3.5 migration")),
            "Done",
            None,
            "2026-03-13T08:00:00.000+0000",
        ),
    ];

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(issues)))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);

    let results = source.search(&make_query("broadcast")).await.unwrap();

    assert_eq!(results.len(), 3);

    // First result
    assert_eq!(results[0].source, "jira");
    assert_eq!(results[0].title, "FIN-100: Fix OOM in broadcast");
    assert_eq!(results[0].snippet, "Broadcast was unbounded");
    assert!(results[0].url.as_ref().unwrap().ends_with("/browse/FIN-100"));
    assert!(results[0].timestamp.is_some());

    // Second result
    assert_eq!(results[1].title, "FIN-101: Add retry logic");

    // Third result
    assert_eq!(results[2].title, "PLAT-200: Upgrade Spark");
}

// ===========================================================================
// Test 2: project_filter_in_jql
// ===========================================================================

#[tokio::test]
async fn project_filter_in_jql() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .and(query_param_contains("jql", "project IN"))
        .and(query_param_contains("jql", "\"FIN\""))
        .and(query_param_contains("jql", "\"PLAT\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(vec![])))
        .mount(&server)
        .await;

    let mut config = make_config(&server.uri());
    config.projects = vec!["FIN".to_string(), "PLAT".to_string()];
    let source = JiraSource::new(config);

    let results = source.search(&make_query("test")).await.unwrap();
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// Test 3: description_truncated
// ===========================================================================

#[tokio::test]
async fn description_truncated() {
    let server = MockServer::start().await;

    let long_text = "A".repeat(1000);
    let issues = vec![jira_issue(
        "FIN-300",
        "Long description",
        Some(adf_text(&long_text)),
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
    // Should be truncated to 300 chars + "..."
    assert_eq!(results[0].snippet.len(), 303);
    assert!(results[0].snippet.ends_with("..."));
    assert!(results[0].snippet.starts_with("AAA"));
}

// ===========================================================================
// Test 4: empty_results
// ===========================================================================

#[tokio::test]
async fn empty_results() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(vec![])))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source.search(&make_query("nonexistent")).await.unwrap();

    assert_eq!(results.len(), 0);
}

// ===========================================================================
// Test 5: auth_failure_401
// ===========================================================================

#[tokio::test]
async fn auth_failure_401() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let err = source.search(&make_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.to_lowercase().contains("auth"),
        "Expected auth-related error message, got: {}",
        err_msg
    );
}

// ===========================================================================
// Test 6: forbidden_403
// ===========================================================================

#[tokio::test]
async fn forbidden_403() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let err = source.search(&make_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.to_lowercase().contains("permission") || err_msg.to_lowercase().contains("forbidden"),
        "Expected permission/forbidden error, got: {}",
        err_msg
    );
}

// ===========================================================================
// Test 7: rate_limited_429
// ===========================================================================

#[tokio::test]
async fn rate_limited_429() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(429).insert_header("Retry-After", "60"),
        )
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let err = source.search(&make_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.to_lowercase().contains("rate limit"),
        "Expected rate limit error, got: {}",
        err_msg
    );
}

// ===========================================================================
// Test 8: server_error_500
// ===========================================================================

#[tokio::test]
async fn server_error_500() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let err = source.search(&make_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    // Should surface the error, not silently succeed
    assert!(
        !err_msg.is_empty(),
        "Expected a non-empty error message for 500"
    );
}

// ===========================================================================
// Test 9: network_timeout
// ===========================================================================

#[tokio::test]
async fn network_timeout() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jira_search_response(vec![]))
                .set_delay(std::time::Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    let mut config = make_config(&server.uri());
    // The JiraSource should use a reasonable timeout internally
    config.max_results = 10;
    let source = JiraSource::new(config);
    let err = source.search(&make_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        !err_msg.is_empty(),
        "Expected a non-empty error for timeout"
    );
}

// ===========================================================================
// Test 10: malformed_json
// ===========================================================================

#[tokio::test]
async fn malformed_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("this is not json {{{"),
        )
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let err = source.search(&make_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        !err_msg.is_empty(),
        "Expected a non-empty error for malformed JSON"
    );
}

// ===========================================================================
// Test 11: health_check
// ===========================================================================

#[tokio::test]
async fn health_check() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"displayName": "Test User", "emailAddress": "user@example.com"})),
        )
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let health = source.health_check().await;

    assert_eq!(health.source, "jira");
    assert!(matches!(health.status, HealthStatus::Healthy));
    assert!(health.latency_ms.is_some());
}

// ===========================================================================
// Test 12: metadata_includes_fields
// ===========================================================================

#[tokio::test]
async fn metadata_includes_fields() {
    let server = MockServer::start().await;

    let issues = vec![jira_issue(
        "FIN-500",
        "Test metadata",
        Some(adf_text("desc")),
        "In Review",
        Some("Charlie"),
        "2026-03-15T10:00:00.000+0000",
    )];

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(issues)))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source.search(&make_query("metadata")).await.unwrap();

    assert_eq!(results.len(), 1);
    let meta = &results[0].metadata;

    assert_eq!(meta.get("project").unwrap(), "FIN");
    assert_eq!(meta.get("status").unwrap(), "In Review");
    assert_eq!(meta.get("assignee").unwrap(), "Charlie");
}

// ===========================================================================
// Test 13: browse_url_construction
// ===========================================================================

#[tokio::test]
async fn browse_url_construction() {
    let server = MockServer::start().await;

    let issues = vec![jira_issue(
        "PLAT-42",
        "URL test",
        None,
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
    let results = source.search(&make_query("url")).await.unwrap();

    assert_eq!(results.len(), 1);
    let expected_url = format!("{}/browse/PLAT-42", server.uri());
    assert_eq!(results[0].url.as_ref().unwrap(), &expected_url);
}

// ===========================================================================
// Test 14: relevance_from_api_order
// ===========================================================================

#[tokio::test]
async fn relevance_from_api_order() {
    let server = MockServer::start().await;

    let issues = vec![
        jira_issue("FIN-1", "First", Some(adf_text("first")), "Open", None, "2026-03-15T10:00:00.000+0000"),
        jira_issue("FIN-2", "Second", Some(adf_text("second")), "Open", None, "2026-03-14T10:00:00.000+0000"),
        jira_issue("FIN-3", "Third", Some(adf_text("third")), "Open", None, "2026-03-13T10:00:00.000+0000"),
    ];

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(issues)))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source.search(&make_query("test")).await.unwrap();

    assert_eq!(results.len(), 3);
    // Position-based: first result should have highest relevance
    assert!(results[0].relevance > results[1].relevance);
    assert!(results[1].relevance > results[2].relevance);
    // All relevance values should be between 0.0 and 1.0
    for r in &results {
        assert!(r.relevance >= 0.0 && r.relevance <= 1.0);
    }
}

// ===========================================================================
// Test 15: query_with_quotes_escaped
// ===========================================================================

#[tokio::test]
async fn query_with_quotes_escaped() {
    let server = MockServer::start().await;

    // The query has double quotes that must be escaped in JQL
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .and(query_param_contains("jql", "\\\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(vec![])))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source
        .search(&make_query("fix \"broadcast\" issue"))
        .await
        .unwrap();

    assert_eq!(results.len(), 0);
}

// ===========================================================================
// Test 16: query_with_jql_operators_literal
// ===========================================================================

#[tokio::test]
async fn query_with_jql_operators_literal() {
    let server = MockServer::start().await;

    // "AND OR NOT" in the query text should be treated as literal text, not JQL operators
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .and(query_param_contains("jql", "AND OR NOT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(vec![])))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);
    let results = source.search(&make_query("AND OR NOT")).await.unwrap();

    assert_eq!(results.len(), 0);
}

// ===========================================================================
// Test 18: search_extracts_comments_from_response
// ===========================================================================

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

// ===========================================================================
// Test 19: search_handles_empty_comments
// ===========================================================================

#[tokio::test]
async fn search_handles_empty_comments() {
    let server = MockServer::start().await;

    // Existing helper creates issues with empty comment arrays
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
    let count = results[0].metadata.get("comment_count").map(|s| s.as_str()).unwrap_or("0");
    assert_eq!(count, "0");
    assert!(
        !results[0].snippet.contains("Comments ("),
        "Snippet should not have comments section when there are none"
    );
}

// ===========================================================================
// Test 20: get_detail_issue_returns_full_markdown
// ===========================================================================

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

    assert!(result.contains("FIN-1234: Fix broadcast threshold OOM"), "Missing title");
    assert!(result.contains("broadcast queue grows unbounded"), "Missing description");
    assert!(result.contains("In Progress"), "Missing status");
    assert!(result.contains("Alice Chen"), "Missing assignee");
    assert!(result.contains("v6.3.4"), "Missing fix version");
    assert!(result.contains("High"), "Missing priority");
    assert!(result.contains("FIN-1235"), "Missing linked issue");
    assert!(result.contains("blocks"), "Missing link type");
    assert!(result.contains("FIN-1234-1"), "Missing subtask 1");
    assert!(result.contains("FIN-1234-2"), "Missing subtask 2");
    assert!(result.contains("Bob Smith"), "Missing comment author");
    assert!(result.contains("Reproduced on staging"), "Missing comment body");
}

// ===========================================================================
// Test 21: get_detail_issue_404_returns_error
// ===========================================================================

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
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found") || err_msg.contains("404"));
}

// ===========================================================================
// Test 17: time_filter_after_before
// ===========================================================================

#[tokio::test]
async fn time_filter_after_before() {
    let server = MockServer::start().await;

    let after = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let before = Utc.with_ymd_and_hms(2026, 3, 31, 23, 59, 59).unwrap();

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .and(query_param_contains("jql", "updated >= \"2026-01-01\""))
        .and(query_param_contains("jql", "updated <= \"2026-03-31\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(vec![])))
        .mount(&server)
        .await;

    let config = make_config(&server.uri());
    let source = JiraSource::new(config);

    let query = SearchQuery {
        text: "test".to_string(),
        max_results: 25,
        filters: SearchFilters {
            sources: None,
            after: Some(after),
            before: Some(before),
        },
    };

    let results = source.search(&query).await.unwrap();
    assert_eq!(results.len(), 0);
}
