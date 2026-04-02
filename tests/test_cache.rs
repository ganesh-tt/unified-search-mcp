use std::collections::HashMap;
use std::time::Duration;
use unified_search_mcp::cache::ResponseCache;
use unified_search_mcp::models::{SearchResult, UnifiedSearchResponse, PerSourceStats};

fn make_response(n_results: usize) -> UnifiedSearchResponse {
    UnifiedSearchResponse {
        results: (0..n_results)
            .map(|i| SearchResult {
                source: "test".to_string(),
                title: format!("Result {}", i),
                snippet: format!("Snippet {}", i),
                url: Some(format!("https://example.com/{}", i)),
                timestamp: None,
                relevance: 1.0 - (i as f32 * 0.1),
                metadata: HashMap::new(),
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
    assert!(cache.get("test query", &["slack"]).is_none());
    let response = make_response(3);
    cache.put("test query", &["slack"], response);
    let cached = cache.get("test query", &["slack"]);
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().results.len(), 3);
}

#[test]
fn cache_key_normalized() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(300));
    cache.put("Test Query", &["slack", "jira"], make_response(1));
    let cached = cache.get("test query", &["jira", "slack"]);
    assert!(cached.is_some(), "Should be case-insensitive and source-order-independent");
}

#[test]
fn cache_ttl_expiry() {
    let mut cache = ResponseCache::new(100, Duration::from_millis(50));
    cache.put("query", &["slack"], make_response(1));
    assert!(cache.get("query", &["slack"]).is_some());
    std::thread::sleep(Duration::from_millis(60));
    assert!(cache.get("query", &["slack"]).is_none());
}

#[test]
fn cache_eviction_at_max() {
    let mut cache = ResponseCache::new(3, Duration::from_secs(300));
    for i in 0..4 {
        cache.put(&format!("query{}", i), &["slack"], make_response(1));
    }
    // Should have exactly 3 entries
    let present: usize = (0..=3)
        .filter(|i| cache.get(&format!("query{}", i), &["slack"]).is_some())
        .count();
    assert_eq!(present, 3);
}

#[test]
fn cache_disabled_with_zero_ttl() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(0));
    cache.put("query", &["slack"], make_response(1));
    assert!(cache.get("query", &["slack"]).is_none());
}

#[test]
fn different_sources_different_keys() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(300));
    cache.put("query", &["slack"], make_response(1));
    cache.put("query", &["jira"], make_response(2));
    assert_eq!(cache.get("query", &["slack"]).unwrap().results.len(), 1);
    assert_eq!(cache.get("query", &["jira"]).unwrap().results.len(), 2);
}

#[test]
fn cache_hit_flag_set() {
    let mut cache = ResponseCache::new(100, Duration::from_secs(300));
    cache.put("query", &["slack"], make_response(1));
    let cached = cache.get("query", &["slack"]).unwrap();
    assert!(cached.cache_hit, "Cached response should have cache_hit=true");
}
