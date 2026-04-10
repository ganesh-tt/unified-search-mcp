use std::time::Duration;

use chrono::{TimeZone, Utc};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use unified_search_mcp::models::*;
use unified_search_mcp::sources::slack::{SlackConfig, SlackSource};
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Helpers
// ===========================================================================

/// Build a `SlackConfig` pointing at the wiremock server.
fn test_config(base_url: &str) -> SlackConfig {
    SlackConfig {
        user_token: "xoxp-test-token-12345".to_string(),
        max_results: 20,
        base_url: base_url.to_string(),
    }
}

/// Build a default `SearchQuery`.
fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        max_results: 20,
        filters: SearchFilters::default(),
    }
}

/// Build a Slack search API response body with the given matches.
fn slack_search_response(matches: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "messages": {
            "matches": matches,
            "total": matches.len()
        }
    })
}

/// Build a single Slack message match object.
fn slack_message(
    text: &str,
    permalink: &str,
    channel_name: &str,
    username: &str,
    ts: &str,
    score: f64,
) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "permalink": permalink,
        "channel": {
            "name": channel_name
        },
        "username": username,
        "ts": ts,
        "score": score
    })
}

// ===========================================================================
// Test 1: successful_search_maps_results
// ===========================================================================

/// 3 messages with correct snippets, permalinks, channel names
#[tokio::test]
async fn successful_search_maps_results() {
    let server = MockServer::start().await;

    let matches = vec![
        slack_message(
            "Hello team, the deploy is done",
            "https://slack.com/archives/C123/p111",
            "engineering",
            "ganesh",
            "1710700800.123456",
            0.9,
        ),
        slack_message(
            "PR review requested",
            "https://slack.com/archives/C456/p222",
            "code-review",
            "alice",
            "1710700900.654321",
            0.7,
        ),
        slack_message(
            "Build failed on CI",
            "https://slack.com/archives/C789/p333",
            "ci-alerts",
            "bot",
            "1710701000.000001",
            0.5,
        ),
    ];

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .and(header("Authorization", "Bearer xoxp-test-token-12345"))
        .and(query_param("query", "deploy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_search_response(matches)))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let results = source.search(&query("deploy")).await.unwrap();

    assert_eq!(results.len(), 3);

    // Check snippets
    assert_eq!(results[0].snippet, "Hello team, the deploy is done");
    assert_eq!(results[1].snippet, "PR review requested");
    assert_eq!(results[2].snippet, "Build failed on CI");

    // Check URLs (permalinks)
    assert_eq!(
        results[0].url,
        Some("https://slack.com/archives/C123/p111".to_string())
    );
    assert_eq!(
        results[1].url,
        Some("https://slack.com/archives/C456/p222".to_string())
    );
    assert_eq!(
        results[2].url,
        Some("https://slack.com/archives/C789/p333".to_string())
    );

    // All results are from "slack" source
    assert!(results.iter().all(|r| r.source == "slack"));
}

// ===========================================================================
// Test 2: channel_name_in_metadata
// ===========================================================================

/// metadata["channel"] = "engineering"
#[tokio::test]
async fn channel_name_in_metadata() {
    let server = MockServer::start().await;

    let matches = vec![slack_message(
        "test message",
        "https://slack.com/archives/C123/p111",
        "engineering",
        "ganesh",
        "1710700800.123456",
        0.8,
    )];

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .and(header("Authorization", "Bearer xoxp-test-token-12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_search_response(matches)))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let results = source.search(&query("test")).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].metadata.get("channel"),
        Some(&"engineering".to_string())
    );
}

// ===========================================================================
// Test 3: username_in_metadata
// ===========================================================================

/// metadata["user"] = "ganesh"
#[tokio::test]
async fn username_in_metadata() {
    let server = MockServer::start().await;

    let matches = vec![slack_message(
        "test message",
        "https://slack.com/archives/C123/p111",
        "general",
        "ganesh",
        "1710700800.123456",
        0.8,
    )];

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .and(header("Authorization", "Bearer xoxp-test-token-12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_search_response(matches)))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let results = source.search(&query("test")).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].metadata.get("user"), Some(&"ganesh".to_string()));
}

// ===========================================================================
// Test 4: empty_results
// ===========================================================================

/// 0 matches, no error
#[tokio::test]
async fn empty_results() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .and(header("Authorization", "Bearer xoxp-test-token-12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_search_response(vec![])))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let results = source.search(&query("nonexistent")).await.unwrap();

    assert!(results.is_empty());
}

// ===========================================================================
// Test 5: ok_false_response
// ===========================================================================

/// {"ok": false, "error": "invalid_auth"} → error includes "invalid_auth"
#[tokio::test]
async fn ok_false_response() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": false,
            "error": "invalid_auth"
        })))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let err = source.search(&query("test")).await.unwrap_err();

    let err_msg = err.to_string();
    assert!(
        err_msg.contains("invalid_auth"),
        "Error should contain 'invalid_auth', got: {err_msg}"
    );
}

// ===========================================================================
// Test 6: wrong_token_type_hint
// ===========================================================================

/// error="not_allowed_token_type" → error mentions xoxp- vs xoxb-
#[tokio::test]
async fn wrong_token_type_hint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": false,
            "error": "not_allowed_token_type"
        })))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let err = source.search(&query("test")).await.unwrap_err();

    let err_msg = err.to_string();
    assert!(
        err_msg.contains("xoxp-") || err_msg.contains("xoxb-"),
        "Error should hint about xoxp- vs xoxb- token types, got: {err_msg}"
    );
}

// ===========================================================================
// Test 7: rate_limited
// ===========================================================================

/// 429 status → rate limit error
#[tokio::test]
async fn rate_limited() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "30")
                .set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "ratelimited"
                })),
        )
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let err = source.search(&query("test")).await.unwrap_err();

    // Should be a RateLimited error variant
    match &err {
        SearchError::RateLimited {
            source_name,
            retry_after_secs,
        } => {
            assert_eq!(source_name, "slack");
            assert_eq!(*retry_after_secs, 30);
        }
        other => panic!("Expected RateLimited error, got: {other:?}"),
    }
}

// ===========================================================================
// Test 8: network_timeout
// ===========================================================================

/// wiremock delay → timeout error
#[tokio::test]
async fn network_timeout() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(slack_search_response(vec![]))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    // Use a short timeout config
    let config = SlackConfig {
        user_token: "xoxp-test-token-12345".to_string(),
        max_results: 20,
        base_url: server.uri(),
    };
    let source = SlackSource::new(config);
    let err = source.search(&query("test")).await.unwrap_err();

    // Should be some kind of timeout/HTTP error
    let err_msg = format!("{err:?}");
    assert!(
        err_msg.to_lowercase().contains("timeout")
            || err_msg.to_lowercase().contains("timed out")
            || err_msg.to_lowercase().contains("http"),
        "Expected timeout-related error, got: {err_msg}"
    );
}

// ===========================================================================
// Test 9: malformed_json
// ===========================================================================

/// parse error, no crash
#[tokio::test]
async fn malformed_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string("this is not valid json {{{"))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let err = source.search(&query("test")).await.unwrap_err();

    // Should be an error, not a panic
    let _err_msg = err.to_string();
    // Just verify it returns an error (not a panic/crash)
}

// ===========================================================================
// Test 10: health_check_auth_test
// ===========================================================================

/// {"ok": true} on auth.test → Healthy
#[tokio::test]
async fn health_check_auth_test() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .and(header("Authorization", "Bearer xoxp-test-token-12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "user": "testuser",
            "team": "testteam"
        })))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let health = source.health_check().await;

    assert_eq!(health.source, "slack");
    assert!(
        matches!(health.status, HealthStatus::Healthy),
        "Expected Healthy status, got {:?}",
        health.status
    );
}

// ===========================================================================
// Test 11: relevance_from_score
// ===========================================================================

/// score field mapped to 0.0–1.0
#[tokio::test]
async fn relevance_from_score() {
    let server = MockServer::start().await;

    // Scores: 10.0, 5.0, 2.5 — should be normalized by dividing by max (10.0)
    let matches = vec![
        slack_message(
            "highest score",
            "https://slack.com/archives/C1/p1",
            "general",
            "user1",
            "1710700800.000000",
            10.0,
        ),
        slack_message(
            "medium score",
            "https://slack.com/archives/C2/p2",
            "general",
            "user2",
            "1710700900.000000",
            5.0,
        ),
        slack_message(
            "low score",
            "https://slack.com/archives/C3/p3",
            "general",
            "user3",
            "1710701000.000000",
            2.5,
        ),
    ];

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_search_response(matches)))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let results = source.search(&query("test")).await.unwrap();

    assert_eq!(results.len(), 3);

    // All relevance values should be in [0.0, 1.0]
    for r in &results {
        assert!(
            (0.0..=1.0).contains(&r.relevance),
            "Relevance {} should be in [0.0, 1.0]",
            r.relevance
        );
    }

    // Highest score (10.0) should normalize to 1.0
    let highest = results
        .iter()
        .find(|r| r.snippet == "highest score")
        .unwrap();
    assert!(
        (highest.relevance - 1.0).abs() < 0.01,
        "Highest score should normalize to ~1.0, got {}",
        highest.relevance
    );

    // Medium score (5.0) should normalize to 0.5
    let medium = results
        .iter()
        .find(|r| r.snippet == "medium score")
        .unwrap();
    assert!(
        (medium.relevance - 0.5).abs() < 0.01,
        "Medium score should normalize to ~0.5, got {}",
        medium.relevance
    );

    // Low score (2.5) should normalize to 0.25
    let low = results.iter().find(|r| r.snippet == "low score").unwrap();
    assert!(
        (low.relevance - 0.25).abs() < 0.01,
        "Low score should normalize to ~0.25, got {}",
        low.relevance
    );
}

// ===========================================================================
// Test 13: get_detail_thread_returns_full_markdown
// ===========================================================================

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

    assert!(result.contains("engineering"), "Missing channel name");
    assert!(
        result.contains("broadcast threshold"),
        "Missing original message"
    );
    assert!(
        result.contains("800 msg/s before OOM"),
        "Missing reply content"
    );
    assert!(
        result.contains("circuit breaker at 750"),
        "Missing second reply"
    );
}

// ===========================================================================
// Test 12: timestamp_from_ts_field
// ===========================================================================

/// ts="1710700800.123456" → correct DateTime
#[tokio::test]
async fn timestamp_from_ts_field() {
    let server = MockServer::start().await;

    let matches = vec![slack_message(
        "timestamped message",
        "https://slack.com/archives/C1/p1",
        "general",
        "user1",
        "1710700800.123456",
        0.8,
    )];

    Mock::given(method("GET"))
        .and(path("/api/search.messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_search_response(matches)))
        .mount(&server)
        .await;

    let config = test_config(&server.uri());
    let source = SlackSource::new(config);
    let results = source.search(&query("test")).await.unwrap();

    assert_eq!(results.len(), 1);

    let ts = results[0].timestamp.expect("Should have a timestamp");
    // 1710700800 seconds = 2024-03-17T18:40:00Z
    let expected = Utc.with_ymd_and_hms(2024, 3, 17, 18, 40, 0).unwrap();
    // Allow small delta for the fractional part (.123456)
    let diff = (ts.timestamp() - expected.timestamp()).abs();
    assert!(
        diff <= 1,
        "Timestamp should be close to 2024-03-17T20:00:00Z, got {ts}"
    );

    // Check the nanosecond fractional part
    let expected_nanos = 123456_000; // .123456 seconds = 123456000 nanoseconds
    let actual_nanos = ts.timestamp_subsec_nanos();
    let nanos_diff = (actual_nanos as i64 - expected_nanos as i64).abs();
    assert!(
        nanos_diff < 1000, // within 1 microsecond tolerance
        "Nanoseconds should be ~{expected_nanos}, got {actual_nanos}"
    );
}
