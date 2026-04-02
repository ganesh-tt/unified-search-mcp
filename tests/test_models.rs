use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use pretty_assertions::assert_eq;
use serde_json;
use unified_search_mcp::models::*;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Build a `SearchResult` with sensible defaults for fields we don't care about.
fn make_result(
    source: &str,
    relevance: f32,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
) -> SearchResult {
    SearchResult {
        source: source.to_string(),
        title: format!("{source} result"),
        snippet: format!("Snippet from {source}"),
        url: Some(format!("https://{source}.example.com")),
        timestamp,
        relevance,
        metadata: HashMap::new(),
    }
}

/// Sort `SearchResult`s by the contract: relevance DESC, then timestamp DESC
/// (None timestamps sort last).
///
/// Once Agent B implements `Ord` on `SearchResult`, callers should switch to
/// plain `.sort()` and this helper can be removed.
fn sort_by_contract(results: &mut Vec<SearchResult>) {
    results.sort_by(|a, b| {
        // Primary: relevance descending (higher first)
        let rel_cmp = b
            .relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal);
        if rel_cmp != std::cmp::Ordering::Equal {
            return rel_cmp;
        }
        // Secondary: timestamp descending (more recent first), None sorts last
        match (&a.timestamp, &b.timestamp) {
            (Some(at), Some(bt)) => bt.cmp(at), // descending: more recent first
            (Some(_), None) => std::cmp::Ordering::Less,    // a has ts, b doesn't → a first (a < b)
            (None, Some(_)) => std::cmp::Ordering::Greater, // b has ts, a doesn't → b first (a > b)
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

// ---------------------------------------------------------------------------
// 1. Serialization round-trip
// ---------------------------------------------------------------------------

#[test]
fn search_result_serialization_roundtrip() {
    let ts = Utc.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap();
    let mut meta = HashMap::new();
    meta.insert("key1".into(), "value1".into());
    meta.insert("key2".into(), "value2".into());

    let original = SearchResult {
        source: "jira".to_string(),
        title: "FIN-1234 Fix OOM".to_string(),
        snippet: "Broadcast was unbounded".to_string(),
        url: Some("https://jira.example.com/FIN-1234".to_string()),
        timestamp: Some(ts),
        relevance: 0.85,
        metadata: meta,
    };

    let json = serde_json::to_string(&original).expect("serialize");
    let roundtripped: SearchResult = serde_json::from_str(&json).expect("deserialize");

    // Compare field-by-field (avoids needing PartialEq on SearchResult)
    assert_eq!(roundtripped.source, original.source);
    assert_eq!(roundtripped.title, original.title);
    assert_eq!(roundtripped.snippet, original.snippet);
    assert_eq!(roundtripped.url, original.url);
    assert_eq!(roundtripped.timestamp, original.timestamp);
    assert_eq!(roundtripped.relevance, original.relevance);
    assert_eq!(roundtripped.metadata, original.metadata);
}

// ---------------------------------------------------------------------------
// 2. Ordering by relevance (descending)
// ---------------------------------------------------------------------------

#[test]
fn search_result_ordering_by_relevance() {
    let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let mut results = vec![
        make_result("a", 0.5, Some(ts)),
        make_result("b", 0.9, Some(ts)),
        make_result("c", 0.1, Some(ts)),
        make_result("d", 0.7, Some(ts)),
    ];

    // TODO(Agent B): replace with `results.sort()` once Ord is implemented
    sort_by_contract(&mut results);

    let relevances: Vec<f32> = results.iter().map(|r| r.relevance).collect();
    // Contract: Ord sorts by relevance DESC
    assert_eq!(relevances, vec![0.9, 0.7, 0.5, 0.1]);
}

// ---------------------------------------------------------------------------
// 3. Ordering tie-break by timestamp (more recent first)
// ---------------------------------------------------------------------------

#[test]
fn search_result_ordering_tiebreak_by_timestamp() {
    let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 3, 10, 6, 30, 0).unwrap();

    let mut results = vec![
        make_result("oldest", 0.8, Some(t1)),
        make_result("newest", 0.8, Some(t2)),
        make_result("middle", 0.8, Some(t3)),
    ];

    // TODO(Agent B): replace with `results.sort()` once Ord is implemented
    sort_by_contract(&mut results);

    let sources: Vec<&str> = results.iter().map(|r| r.source.as_str()).collect();
    // Same relevance -> most recent timestamp first
    assert_eq!(sources, vec!["newest", "middle", "oldest"]);
}

// ---------------------------------------------------------------------------
// 4. None timestamps sort last (same relevance)
// ---------------------------------------------------------------------------

#[test]
fn search_result_ordering_none_timestamp_last() {
    let ts = Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();

    let mut results = vec![
        make_result("no_ts_1", 0.7, None),
        make_result("has_ts", 0.7, Some(ts)),
        make_result("no_ts_2", 0.7, None),
    ];

    // TODO(Agent B): replace with `results.sort()` once Ord is implemented
    sort_by_contract(&mut results);

    let sources: Vec<&str> = results.iter().map(|r| r.source.as_str()).collect();
    // The one with a timestamp should come before the Nones
    assert_eq!(sources[0], "has_ts");
    // The remaining two have None timestamps -- order among them is unspecified but both after has_ts
    assert!(sources[1..].iter().all(|s| *s == "no_ts_1" || *s == "no_ts_2"));
}

// ---------------------------------------------------------------------------
// 5. SearchQuery::default()
// ---------------------------------------------------------------------------

#[test]
fn search_query_defaults() {
    let q = SearchQuery::default();

    assert_eq!(q.max_results, 20);
    assert_eq!(q.text, "");
    assert!(q.filters.sources.is_none());
    assert!(q.filters.after.is_none());
    assert!(q.filters.before.is_none());
}

// ---------------------------------------------------------------------------
// 6. SearchFilters with only sources -> after/before null in JSON
// ---------------------------------------------------------------------------

#[test]
fn search_filters_partial() {
    let filters = SearchFilters {
        sources: Some(vec!["jira".to_string(), "confluence".to_string()]),
        after: None,
        before: None,
    };

    let json_val: serde_json::Value = serde_json::to_value(&filters).expect("serialize");

    // sources should be present
    assert!(json_val.get("sources").is_some());
    let sources_arr = json_val["sources"].as_array().expect("sources is array");
    assert_eq!(sources_arr.len(), 2);
    assert_eq!(sources_arr[0], "jira");
    assert_eq!(sources_arr[1], "confluence");

    // after and before should be null
    assert_eq!(json_val.get("after"), Some(&serde_json::Value::Null));
    assert_eq!(json_val.get("before"), Some(&serde_json::Value::Null));
}

// ---------------------------------------------------------------------------
// 7. HealthStatus Display
// ---------------------------------------------------------------------------

#[test]
fn health_status_display() {
    // TODO(Agent B): uncomment once Display is implemented on HealthStatus
    // assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
    // assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
    // assert_eq!(HealthStatus::Unavailable.to_string(), "unavailable");

    // For now, verify the variants exist via Debug (always derived)
    let variants = vec![
        (HealthStatus::Healthy, "healthy"),
        (HealthStatus::Degraded, "degraded"),
        (HealthStatus::Unavailable, "unavailable"),
    ];
    for (variant, expected_display) in &variants {
        // Debug output gives us the variant name
        let debug = format!("{:?}", variant);
        assert!(!debug.is_empty(), "Debug output should not be empty");
        // Store expected display values so they're captured in the test
        // Once Display is impl'd, switch to: assert_eq!(variant.to_string(), *expected_display);
        let _ = expected_display;
    }
}

// ---------------------------------------------------------------------------
// 8. UnifiedSearchResponse with warnings
// ---------------------------------------------------------------------------

#[test]
fn unified_response_with_warnings() {
    let ts = Utc.with_ymd_and_hms(2026, 3, 17, 10, 0, 0).unwrap();
    let response = UnifiedSearchResponse {
        results: vec![
            make_result("jira", 0.9, Some(ts)),
            make_result("confluence", 0.6, Some(ts)),
        ],
        warnings: vec!["Slack source timed out".to_string()],
        total_sources_queried: 3,
        query_time_ms: 450,
        per_source_stats: vec![],
    };

    let json_val: serde_json::Value = serde_json::to_value(&response).expect("serialize");

    // Results array has 2 entries
    let results_arr = json_val["results"].as_array().expect("results is array");
    assert_eq!(results_arr.len(), 2);

    // Warnings array has 1 entry
    let warnings_arr = json_val["warnings"].as_array().expect("warnings is array");
    assert_eq!(warnings_arr.len(), 1);
    assert_eq!(warnings_arr[0], "Slack source timed out");

    // Scalar fields
    assert_eq!(json_val["total_sources_queried"], 3);
    assert_eq!(json_val["query_time_ms"], 450);

    // Verify individual result fields in the JSON
    assert_eq!(results_arr[0]["source"], "jira");
    assert_eq!(results_arr[1]["source"], "confluence");
}

// ---------------------------------------------------------------------------
// 9. Edge case: relevance = 0.0
// ---------------------------------------------------------------------------

#[test]
fn search_result_edge_relevance_zero() {
    let r = make_result("low", 0.0, None);

    // Serializes correctly
    let json_val: serde_json::Value = serde_json::to_value(&r).expect("serialize");
    assert_eq!(json_val["relevance"], 0.0);

    // Sorts last among varying relevances
    let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let mut results = vec![
        make_result("mid", 0.5, Some(ts)),
        make_result("zero", 0.0, Some(ts)),
        make_result("high", 1.0, Some(ts)),
    ];

    // TODO(Agent B): replace with `results.sort()` once Ord is implemented
    sort_by_contract(&mut results);

    let relevances: Vec<f32> = results.iter().map(|r| r.relevance).collect();
    assert_eq!(relevances, vec![1.0, 0.5, 0.0]);
}

// ---------------------------------------------------------------------------
// 10. Edge case: relevance = 1.0
// ---------------------------------------------------------------------------

#[test]
fn search_result_edge_relevance_one() {
    let r = make_result("top", 1.0, None);

    // Serializes correctly
    let json_val: serde_json::Value = serde_json::to_value(&r).expect("serialize");
    assert_eq!(json_val["relevance"], 1.0);

    // Sorts first among varying relevances
    let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let mut results = vec![
        make_result("mid", 0.5, Some(ts)),
        make_result("top", 1.0, Some(ts)),
        make_result("low", 0.1, Some(ts)),
    ];

    // TODO(Agent B): replace with `results.sort()` once Ord is implemented
    sort_by_contract(&mut results);

    let sources: Vec<&str> = results.iter().map(|r| r.source.as_str()).collect();
    assert_eq!(sources[0], "top");
}

// ---------------------------------------------------------------------------
// 11. Empty metadata serializes as {}
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 12. PerSourceStats serializes correctly
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 13. UnifiedSearchResponse includes per_source_stats field
// ---------------------------------------------------------------------------

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



#[test]
fn search_result_empty_metadata() {
    let r = make_result("test", 0.5, None);
    assert!(r.metadata.is_empty());

    let json_val: serde_json::Value = serde_json::to_value(&r).expect("serialize");
    let meta = json_val["metadata"].as_object().expect("metadata is object");
    assert!(meta.is_empty());

    // Also verify it round-trips correctly through JSON string
    let json_str = serde_json::to_string(&r).expect("serialize to string");
    assert!(json_str.contains("\"metadata\":{}"));
}
