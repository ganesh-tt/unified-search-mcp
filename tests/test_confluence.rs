use chrono::{TimeZone, Utc};
use wiremock::matchers::{method, path, query_param_contains};
use wiremock::{Mock, MockServer, ResponseTemplate};

use unified_search_mcp::models::*;
use unified_search_mcp::sources::confluence::{ConfluenceConfig, ConfluenceSource};
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Helpers
// ===========================================================================

fn default_config(base_url: &str) -> ConfluenceConfig {
    ConfluenceConfig {
        base_url: base_url.to_string(),
        email: "user@example.com".to_string(),
        api_token: "test-token".to_string(),
        spaces: vec![],
        max_results: 10,
    }
}

fn default_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        max_results: 20,
        filters: SearchFilters::default(),
    }
}

// ===========================================================================
// 1. successful_search_maps_results
// ===========================================================================

/// 3 results from the fixture are correctly mapped: titles, snippets (HTML
/// stripped), and full URLs.
#[tokio::test]
async fn successful_search_maps_results() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_success.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source
        .search(&default_query("broadcast threshold"))
        .await
        .unwrap();

    assert_eq!(results.len(), 3);

    // First result
    assert_eq!(results[0].title, "Broadcast Threshold Design");
    assert_eq!(results[0].source, "confluence");
    assert!(results[0]
        .url
        .as_ref()
        .unwrap()
        .contains("/wiki/spaces/DEV/pages/12345"));

    // Snippet should have HTML stripped
    assert!(!results[0].snippet.contains("<b>"));
    assert!(!results[0].snippet.contains("</b>"));
    assert!(results[0].snippet.contains("broadcast threshold"));

    // Second result
    assert_eq!(results[1].title, "Spark Configuration Guide");

    // Third result
    assert_eq!(results[2].title, "OOM Post-Mortem Feb 2026");
}

// ===========================================================================
// 2. html_stripped_from_excerpt
// ===========================================================================

/// HTML tags are removed from excerpts: `<b>bold</b>` becomes "bold".
#[tokio::test]
async fn html_stripped_from_excerpt() {
    let server = MockServer::start().await;

    let body = r#"{
        "results": [{
            "content": {"id": "1", "type": "page", "title": "Test"},
            "excerpt": "<b>bold</b> and <em>italic</em> and <a href=\"x\">link</a> text",
            "url": "/wiki/spaces/DEV/pages/1",
            "lastModified": "2026-03-10T10:00:00.000Z",
            "resultGlobalContainer": {"title": "DEV", "displayUrl": "/wiki/spaces/DEV"}
        }],
        "start": 0, "limit": 10, "size": 1, "totalSize": 1
    }"#;

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query("test")).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].snippet, "bold and italic and link text");
}

// ===========================================================================
// 3. space_filter_in_cql
// ===========================================================================

/// When spaces=["DEV","OPS"], the CQL includes `space IN ("DEV","OPS")`.
#[tokio::test]
async fn space_filter_in_cql() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_empty.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains("cql", "space IN"))
        .and(query_param_contains("cql", "\"DEV\""))
        .and(query_param_contains("cql", "\"OPS\""))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let mut config = default_config(&server.uri());
    config.spaces = vec!["DEV".to_string(), "OPS".to_string()];

    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query("test")).await.unwrap();

    // If mock matched, it means the CQL had the space IN clause
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 4. empty_results
// ===========================================================================

/// 0 results from the API produces an empty vec, no error.
#[tokio::test]
async fn empty_results() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_empty.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query("nonexistent")).await.unwrap();

    assert!(results.is_empty());
}

// ===========================================================================
// 5. auth_failure_401
// ===========================================================================

/// 401 response produces an Auth error mentioning email/token.
#[tokio::test]
async fn auth_failure_401() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_auth_failure.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(401).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let err = source.search(&default_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.to_lowercase().contains("auth")
            || err_msg.to_lowercase().contains("email")
            || err_msg.to_lowercase().contains("token"),
        "Error should mention auth/email/token, got: {}",
        err_msg
    );
}

// ===========================================================================
// 6. forbidden_403
// ===========================================================================

/// 403 response produces a permission error.
#[tokio::test]
async fn forbidden_403() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_forbidden.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(403).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let err = source.search(&default_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.to_lowercase().contains("permission")
            || err_msg.to_lowercase().contains("forbidden")
            || err_msg.to_lowercase().contains("403"),
        "Error should mention permission/forbidden, got: {}",
        err_msg
    );
}

// ===========================================================================
// 7. rate_limited_429
// ===========================================================================

/// 429 response produces a RateLimited error with Retry-After.
#[tokio::test]
async fn rate_limited_429() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "30")
                .set_body_raw("{}", "application/json"),
        )
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let err = source.search(&default_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.to_lowercase().contains("rate limit") || err_msg.contains("429"),
        "Error should mention rate limiting, got: {}",
        err_msg
    );
}

// ===========================================================================
// 8. server_error_500
// ===========================================================================

/// 500 response is surfaced as an error.
#[tokio::test]
async fn server_error_500() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_server_error.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(500).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let err = source.search(&default_query("test")).await.unwrap_err();

    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("500") || err_msg.to_lowercase().contains("server error"),
        "Error should mention 500/server error, got: {}",
        err_msg
    );
}

// ===========================================================================
// 9. network_timeout
// ===========================================================================

/// Network timeout produces an error (not a panic).
#[tokio::test]
async fn network_timeout() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)))
        .mount(&server)
        .await;

    let mut config = default_config(&server.uri());
    config.max_results = 10;
    // The source should have a short internal timeout
    let source = ConfluenceSource::new(config);
    let result = source.search(&default_query("test")).await;

    assert!(result.is_err(), "Should have timed out");
}

// ===========================================================================
// 10. malformed_json
// ===========================================================================

/// Malformed JSON response produces a parse error, not a crash.
#[tokio::test]
async fn malformed_json() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_malformed.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let result = source.search(&default_query("test")).await;

    assert!(result.is_err(), "Malformed JSON should produce an error");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.to_lowercase().contains("parse")
            || err_msg.to_lowercase().contains("json")
            || err_msg.to_lowercase().contains("deserialize")
            || err_msg.to_lowercase().contains("decode"),
        "Error should mention parse/json, got: {}",
        err_msg
    );
}

// ===========================================================================
// 11. health_check_success
// ===========================================================================

/// Health check succeeds when /wiki/rest/api/space?limit=1 returns 200.
#[tokio::test]
async fn health_check_success() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/health_success.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/space"))
        .and(query_param_contains("limit", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let health = source.health_check().await;

    assert_eq!(health.source, "confluence");
    assert!(
        matches!(health.status, HealthStatus::Healthy),
        "Expected Healthy, got {:?}",
        health.status
    );
}

// ===========================================================================
// 12. relevance_from_api_order
// ===========================================================================

/// Position-based relevance: first result has highest relevance,
/// decreasing for subsequent results.
#[tokio::test]
async fn relevance_from_api_order() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_success.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source
        .search(&default_query("broadcast threshold"))
        .await
        .unwrap();

    assert_eq!(results.len(), 3);

    // A.relevance > B.relevance > C.relevance (position-based)
    assert!(
        results[0].relevance > results[1].relevance,
        "First result ({}) should have higher relevance than second ({})",
        results[0].relevance,
        results[1].relevance
    );
    assert!(
        results[1].relevance > results[2].relevance,
        "Second result ({}) should have higher relevance than third ({})",
        results[1].relevance,
        results[2].relevance
    );

    // First result should be 1.0
    assert!(
        (results[0].relevance - 1.0).abs() < f32::EPSILON,
        "First result relevance should be 1.0, got {}",
        results[0].relevance
    );
}

// ===========================================================================
// 13. query_with_quotes_escaped
// ===========================================================================

/// Double quotes in query text are escaped as `\"` in the CQL.
#[tokio::test]
async fn query_with_quotes_escaped() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_empty.json");

    // The CQL should contain escaped quotes, NOT raw quotes that break the CQL
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains("cql", r#"\""#))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source
        .search(&default_query(r#"search "quoted term" here"#))
        .await
        .unwrap();

    // If mock matched, quotes were escaped
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 14. query_with_cql_operators_literal
// ===========================================================================

/// CQL operators like AND, OR, NOT in the query text are treated as
/// literal search terms, not CQL operators.
#[tokio::test]
async fn query_with_cql_operators_literal() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_empty.json");

    // The query text "AND OR NOT" should be inside the siteSearch string,
    // not interpreted as CQL operators
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains("cql", "siteSearch"))
        .and(query_param_contains("cql", "AND OR NOT"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query("AND OR NOT")).await.unwrap();

    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 15. time_filter_after
// ===========================================================================

/// When `after` filter is set, CQL includes `lastmodified >= "YYYY-MM-DD"`.
#[tokio::test]
async fn time_filter_after() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_empty.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains(
            "cql",
            "lastmodified >= \"2026-03-01\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);

    let query = SearchQuery {
        text: "test".to_string(),
        max_results: 20,
        filters: SearchFilters {
            sources: None,
            after: Some(Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap()),
            before: None,
        },
    };

    let results = source.search(&query).await.unwrap();
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 16. time_filter_before
// ===========================================================================

/// When `before` filter is set, CQL includes `lastmodified <= "YYYY-MM-DD"`.
#[tokio::test]
async fn time_filter_before() {
    let server = MockServer::start().await;

    let body = include_str!("../fixtures/confluence/search_empty.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains(
            "cql",
            "lastmodified <= \"2026-03-15\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);

    let query = SearchQuery {
        text: "test".to_string(),
        max_results: 20,
        filters: SearchFilters {
            sources: None,
            after: None,
            before: Some(Utc.with_ymd_and_hms(2026, 3, 15, 0, 0, 0).unwrap()),
        },
    };

    let results = source.search(&query).await.unwrap();
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 17. search_enriches_with_comments
// ===========================================================================

/// Comment enrichment is skipped during search (moved to get_detail only).
/// Verifies that search results do NOT contain comment_count metadata or
/// comment text in snippets — this avoids 20+ extra HTTP calls per search.
#[tokio::test]
async fn search_does_not_enrich_comments() {
    let server = MockServer::start().await;

    let search_body = include_str!("../fixtures/confluence/search_success.json");

    // Mount only the search endpoint — no comment endpoints needed
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(search_body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source
        .search(&default_query("broadcast threshold"))
        .await
        .unwrap();

    assert_eq!(results.len(), 3);

    // No comment_count metadata — enrichment is skipped
    for result in &results {
        assert!(
            result.metadata.get("comment_count").is_none(),
            "comment_count should NOT be set during search, got: {:?}",
            result.metadata.get("comment_count"),
        );
    }

    // Snippets should not contain comment authors
    for result in &results {
        assert!(
            !result.snippet.contains("Bob Smith") && !result.snippet.contains("Alice Chen"),
            "Snippet should NOT contain comment authors during search, got: {}",
            result.snippet
        );
    }
}

// ===========================================================================
// 19. get_detail_page_returns_full_markdown
// ===========================================================================

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

    assert!(
        result.contains("Broadcast Threshold Design"),
        "Missing title"
    );
    assert!(result.contains("PROD"), "Missing space");
    assert!(result.contains("500 msg/s"), "Missing body content");
    assert!(result.contains("Load Test Results"), "Missing child page");
    assert!(result.contains("Configuration Guide"), "Missing child page");
    assert!(result.contains("Bob Smith"), "Missing comment author");
    assert!(result.contains("Looks good to me"), "Missing comment text");
    assert!(
        result.contains("Charlie Lee"),
        "Missing second comment author"
    );
    assert!(result.contains("architecture"), "Missing label");
    assert!(!result.contains("<h2>"), "HTML tags should be stripped");
    assert!(!result.contains("<p>"), "HTML tags should be stripped");
    assert!(!result.contains("<b>"), "HTML tags should be stripped");
}

// ===========================================================================
// 20. get_detail_page_404_returns_error
// ===========================================================================

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

// ===========================================================================
// 21. get_detail_page_preserves_markdown_structure
// ===========================================================================

#[tokio::test]
async fn get_detail_page_preserves_markdown_structure() {
    let server = MockServer::start().await;

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

    // Should have Markdown formatting
    assert!(
        result.contains("## Overview"),
        "Should convert h2 to ##, got:\n{}",
        result
    );
    assert!(result.contains("**bold**"), "Should convert strong to bold");
    assert!(result.contains("*italic*"), "Should convert em to italic");
    assert!(
        result.contains("- Item one"),
        "Should convert ul/li to list"
    );
    assert!(result.contains("| Col A | Col B |"), "Should convert table");

    // Should NOT have raw HTML
    assert!(!result.contains("<h2>"), "Should not have raw HTML tags");
    assert!(
        !result.contains("<strong>"),
        "Should not have raw HTML tags"
    );
}

// ===========================================================================
// 22. cql_escapes_backslashes_and_quotes
// ===========================================================================

/// A query containing a lone backslash must have it escaped in CQL.
/// Input `foo\bar` should produce CQL containing `foo\\bar` (backslash doubled).
#[tokio::test]
async fn cql_escapes_backslashes_and_quotes() {
    let server = MockServer::start().await;
    let body = include_str!("../fixtures/confluence/search_empty.json");

    // Input: foo\bar (contains a single backslash, no quotes)
    // Correct CQL: siteSearch ~ "foo\\bar" (backslash doubled)
    // Buggy CQL:   siteSearch ~ "foo\bar"  (backslash NOT doubled)
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains("cql", r#"foo\\bar"#))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source.search(&default_query(r#"foo\bar"#)).await.unwrap();
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 23. get_detail_page_rejects_non_numeric_id
// ===========================================================================

/// Path-traversal or non-numeric strings should be rejected as page IDs.
#[tokio::test]
async fn get_detail_page_rejects_non_numeric_id() {
    let server = MockServer::start().await;
    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);

    let result = source.get_detail_page("../../../etc/passwd").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.to_lowercase().contains("invalid"),
        "Should reject non-numeric page ID, got: {}",
        err
    );
}

// ===========================================================================
// 24. get_detail_page_rejects_empty_id
// ===========================================================================

#[tokio::test]
async fn get_detail_page_rejects_empty_id() {
    let server = MockServer::start().await;
    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);

    let result = source.get_detail_page("").await;
    assert!(result.is_err());
}

// ===========================================================================
// 25. get_detail_page_accepts_valid_numeric_id
// ===========================================================================

#[tokio::test]
async fn get_detail_page_accepts_valid_numeric_id() {
    let server = MockServer::start().await;
    let body = include_str!("../fixtures/confluence/page_detail.json");

    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123456"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let result = source.get_detail_page("123456").await;
    assert!(result.is_ok(), "Valid numeric page ID should be accepted");
}

// ===========================================================================
// 26. cql_escapes_space_names_with_quotes
// ===========================================================================

/// Space names containing quotes in config should be escaped in CQL.
#[tokio::test]
async fn cql_escapes_space_names_with_quotes() {
    let server = MockServer::start().await;
    let body = include_str!("../fixtures/confluence/search_empty.json");

    // Space name with a quote should be escaped
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .and(query_param_contains("cql", "space IN"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let mut config = default_config(&server.uri());
    config.spaces = vec![r#"TE"ST"#.to_string()];

    let source = ConfluenceSource::new(config);
    // Should not panic or produce malformed CQL
    let results = source.search(&default_query("test")).await.unwrap();
    assert_eq!(results.len(), 0);
}

// ===========================================================================
// 18. search_comment_failure_degrades_gracefully
// ===========================================================================

/// With comment enrichment removed from search, this test verifies search
/// works fine without any comment endpoints mounted (regression guard).
#[tokio::test]
async fn search_succeeds_without_comment_endpoints() {
    let server = MockServer::start().await;

    let search_body = include_str!("../fixtures/confluence/search_success.json");

    // Mount only the search endpoint — no comment endpoints
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(search_body, "application/json"))
        .mount(&server)
        .await;

    let config = default_config(&server.uri());
    let source = ConfluenceSource::new(config);
    let results = source
        .search(&default_query("broadcast threshold"))
        .await
        .unwrap();

    // Search should succeed — no comment fetch attempted
    assert_eq!(results.len(), 3);
}
