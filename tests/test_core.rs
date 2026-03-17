use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use pretty_assertions::assert_eq;
use tokio::time::Instant;

use unified_search_mcp::core::{OrchestratorConfig, SearchOrchestrator};
use unified_search_mcp::models::*;
use unified_search_mcp::sources::SearchSource;

// ===========================================================================
// Mock sources — defined here, NOT in src/
// ===========================================================================

/// A configurable mock source for testing the orchestrator.
///
/// - Returns `results` after an optional `delay`.
/// - If `should_error` is true, returns a `SearchError::Source` with `error_msg`.
struct MockSource {
    source_name: String,
    description: String,
    results: Vec<SearchResult>,
    delay: Option<Duration>,
    should_error: bool,
    error_msg: String,
    healthy: bool,
}

impl MockSource {
    /// Create a healthy mock source that returns `results` immediately.
    fn new(name: &str, results: Vec<SearchResult>) -> Self {
        Self {
            source_name: name.to_string(),
            description: format!("Mock source: {name}"),
            results,
            delay: None,
            should_error: false,
            error_msg: String::new(),
            healthy: true,
        }
    }

    /// Add a delay before returning results.
    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Make this source return an error instead of results.
    fn with_error(mut self, msg: &str) -> Self {
        self.should_error = true;
        self.error_msg = msg.to_string();
        self
    }

    /// Make this source report as unhealthy in health checks.
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
        &self.description
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
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        if self.should_error {
            return Err(SearchError::Source {
                source_name: self.source_name.clone(),
                message: self.error_msg.clone(),
            });
        }
        Ok(self.results.clone())
    }
}

/// A source that panics on `search()`.
///
/// The orchestrator must catch this via `tokio::spawn` and surface it as a
/// warning — NOT crash the entire search operation.
struct PanicSource;

#[async_trait]
impl SearchSource for PanicSource {
    fn name(&self) -> &str {
        "panic_source"
    }

    fn description(&self) -> &str {
        "A source that panics on search"
    }

    async fn health_check(&self) -> SourceHealth {
        SourceHealth {
            source: "panic_source".to_string(),
            status: HealthStatus::Healthy,
            message: Some("OK".to_string()),
            latency_ms: Some(1),
        }
    }

    async fn search(&self, _query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        panic!("intentional test panic");
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Shorthand to box a source as a trait object for use in Vec<Box<dyn SearchSource>>.
fn boxed(source: impl SearchSource + 'static) -> Box<dyn SearchSource> {
    Box::new(source)
}

/// Build a `SearchResult` with sensible defaults for fields we don't care about.
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

/// Build a `SearchResult` with a specific URL.
fn make_result_with_url(source: &str, title: &str, relevance: f32, url: &str) -> SearchResult {
    SearchResult {
        source: source.to_string(),
        title: title.to_string(),
        snippet: format!("Snippet for {title}"),
        url: Some(url.to_string()),
        timestamp: Some(Utc.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap()),
        relevance,
        metadata: HashMap::new(),
    }
}

/// Build a `SearchResult` with a specific snippet (for dedup-by-snippet tests).
fn make_result_with_snippet(
    source: &str,
    title: &str,
    relevance: f32,
    snippet: &str,
) -> SearchResult {
    SearchResult {
        source: source.to_string(),
        title: title.to_string(),
        snippet: snippet.to_string(),
        url: Some(format!("https://{source}.example.com/{title}")),
        timestamp: Some(Utc.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap()),
        relevance,
        metadata: HashMap::new(),
    }
}

/// Create a default `OrchestratorConfig` with no weights and generous timeout.
fn default_config() -> OrchestratorConfig {
    OrchestratorConfig {
        timeout_seconds: 30,
        source_weights: HashMap::new(),
        max_results: 50,
    }
}

/// Create a default `SearchQuery` with the given text.
fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        max_results: 50,
        filters: SearchFilters::default(),
    }
}

// ===========================================================================
// Test 1: single_source_happy_path
// ===========================================================================

/// One MockSource returning 3 results -> response has 3 results, 0 warnings,
/// total_sources_queried=1.
#[tokio::test]
async fn single_source_happy_path() {
    let results = vec![
        make_result("slack", "msg1", 0.9),
        make_result("slack", "msg2", 0.7),
        make_result("slack", "msg3", 0.5),
    ];

    let source = MockSource::new("slack", results);
    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source)],
        default_config(),
    );

    let response = orchestrator.search(&query("hello")).await;

    assert_eq!(response.results.len(), 3, "Expected 3 results");
    assert!(
        response.warnings.is_empty(),
        "Expected 0 warnings, got: {:?}",
        response.warnings
    );
    assert_eq!(response.total_sources_queried, 1);
}

// ===========================================================================
// Test 2: multiple_sources_merged
// ===========================================================================

/// Three MockSources returning [2, 3, 1] results -> 6 results sorted by
/// relevance descending.
#[tokio::test]
async fn multiple_sources_merged() {
    let source_a = MockSource::new(
        "slack",
        vec![
            make_result("slack", "s1", 0.9),
            make_result("slack", "s2", 0.3),
        ],
    );
    let source_b = MockSource::new(
        "confluence",
        vec![
            make_result("confluence", "c1", 0.8),
            make_result("confluence", "c2", 0.6),
            make_result("confluence", "c3", 0.4),
        ],
    );
    let source_c = MockSource::new(
        "jira",
        vec![make_result("jira", "j1", 0.7)],
    );

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source_a), boxed(source_b), boxed(source_c)],
        default_config(),
    );

    let response = orchestrator.search(&query("search term")).await;

    assert_eq!(response.results.len(), 6, "Expected 6 merged results");
    assert!(
        response.warnings.is_empty(),
        "Expected 0 warnings, got: {:?}",
        response.warnings
    );

    // Verify sorted by relevance descending (no weights -> default weight 1.0)
    let relevances: Vec<f32> = response.results.iter().map(|r| r.relevance).collect();
    for window in relevances.windows(2) {
        assert!(
            window[0] >= window[1],
            "Results not sorted by relevance descending: {:?}",
            relevances
        );
    }
}

// ===========================================================================
// Test 3: source_timeout_returns_partial
// ===========================================================================

/// Fast MockSource + slow MockSource (30s delay), timeout=2s -> fast results
/// returned, warning about timeout, total time < 3s.
#[tokio::test]
async fn source_timeout_returns_partial() {
    let fast_source = MockSource::new(
        "fast",
        vec![make_result("fast", "quick_result", 0.9)],
    );
    let slow_source = MockSource::new(
        "slow",
        vec![make_result("slow", "slow_result", 0.8)],
    )
    .with_delay(Duration::from_secs(30));

    let config = OrchestratorConfig {
        timeout_seconds: 2,
        source_weights: HashMap::new(),
        max_results: 50,
    };

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(fast_source), boxed(slow_source)],
        config,
    );

    let start = Instant::now();
    let response = orchestrator.search(&query("test")).await;
    let elapsed = start.elapsed();

    // Fast source results should be present
    assert_eq!(
        response.results.len(),
        1,
        "Expected 1 result from the fast source"
    );
    assert_eq!(response.results[0].source, "fast");

    // Should have a warning about the slow source timing out
    assert!(
        !response.warnings.is_empty(),
        "Expected at least one warning about timeout"
    );
    let warnings_lower: Vec<String> = response
        .warnings
        .iter()
        .map(|w: &String| w.to_lowercase())
        .collect();
    let has_timeout_warning = warnings_lower
        .iter()
        .any(|w: &String| w.contains("slow") || w.contains("timeout") || w.contains("timed out"));
    assert!(
        has_timeout_warning,
        "Expected a warning mentioning timeout or the slow source, got: {:?}",
        response.warnings
    );

    // Total time should be bounded by the timeout, not the slow source's 30s delay
    assert!(
        elapsed < Duration::from_secs(5),
        "Search took {:?}, expected < 5s (timeout is 2s)",
        elapsed
    );
}

// ===========================================================================
// Test 4: source_error_returns_partial
// ===========================================================================

/// One working MockSource + one ErrorSource -> ok results + warning about the
/// error.
#[tokio::test]
async fn source_error_returns_partial() {
    let ok_source = MockSource::new(
        "working",
        vec![
            make_result("working", "good1", 0.8),
            make_result("working", "good2", 0.6),
        ],
    );
    let err_source = MockSource::new("broken", vec![])
        .with_error("connection refused");

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(ok_source), boxed(err_source)],
        default_config(),
    );

    let response = orchestrator.search(&query("test")).await;

    // Should have results from the working source
    assert_eq!(response.results.len(), 2, "Expected 2 results from ok source");
    assert!(
        response.results.iter().all(|r| r.source == "working"),
        "All results should be from the working source"
    );

    // Should have a warning about the error
    assert!(
        !response.warnings.is_empty(),
        "Expected at least one warning about the failed source"
    );
    let warnings_joined = response.warnings.join(" ");
    let has_error_warning =
        warnings_joined.contains("connection refused") || warnings_joined.to_lowercase().contains("broken");
    assert!(
        has_error_warning,
        "Expected a warning about the connection refused error, got: {:?}",
        response.warnings
    );
}

// ===========================================================================
// Test 5: all_sources_fail
// ===========================================================================

/// Two ErrorSources -> 0 results, 2 warnings.
#[tokio::test]
async fn all_sources_fail() {
    let err_source_a = MockSource::new("source_a", vec![])
        .with_error("timeout");
    let err_source_b = MockSource::new("source_b", vec![])
        .with_error("auth failure");

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(err_source_a), boxed(err_source_b)],
        default_config(),
    );

    let response = orchestrator.search(&query("test")).await;

    assert_eq!(response.results.len(), 0, "Expected 0 results when all sources fail");
    assert_eq!(
        response.warnings.len(),
        2,
        "Expected 2 warnings (one per failed source), got: {:?}",
        response.warnings
    );
}

// ===========================================================================
// Test 6: source_weights_affect_ranking
// ===========================================================================

/// source_a (weight=2.0) returns result(relevance=0.5) -> weighted 1.0;
/// source_b (weight=1.0) returns result(relevance=0.8) -> weighted 0.8.
/// source_a result should rank first.
#[tokio::test]
async fn source_weights_affect_ranking() {
    let source_a = MockSource::new(
        "source_a",
        vec![make_result("source_a", "weighted_high", 0.5)],
    );
    let source_b = MockSource::new(
        "source_b",
        vec![make_result("source_b", "raw_high", 0.8)],
    );

    let mut weights = HashMap::new();
    weights.insert("source_a".to_string(), 2.0f32);
    weights.insert("source_b".to_string(), 1.0f32);

    let config = OrchestratorConfig {
        timeout_seconds: 30,
        source_weights: weights,
        max_results: 50,
    };

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source_a), boxed(source_b)],
        config,
    );

    let response = orchestrator.search(&query("test")).await;

    assert_eq!(response.results.len(), 2, "Expected 2 results");

    // source_a's result (weighted 1.0) should rank above source_b's (weighted 0.8)
    assert_eq!(
        response.results[0].source, "source_a",
        "source_a (weighted score 1.0) should rank first, but got source={}, results={:?}",
        response.results[0].source,
        response.results.iter().map(|r| (&r.source, r.relevance)).collect::<Vec<_>>()
    );
    assert_eq!(
        response.results[1].source, "source_b",
        "source_b (weighted score 0.8) should rank second"
    );
}

// ===========================================================================
// Test 7: dedup_by_url
// ===========================================================================

/// Two sources return results with same URL, different relevances ->
/// only the higher-relevance result is kept.
#[tokio::test]
async fn dedup_by_url() {
    let shared_url = "https://example.com/page";

    let source_a = MockSource::new(
        "source_a",
        vec![make_result_with_url("source_a", "high_rel", 0.9, shared_url)],
    );
    let source_b = MockSource::new(
        "source_b",
        vec![make_result_with_url("source_b", "low_rel", 0.5, shared_url)],
    );

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source_a), boxed(source_b)],
        default_config(),
    );

    let response = orchestrator.search(&query("test")).await;

    // Same URL -> dedup -> keep only the higher-scored one (0.9)
    assert_eq!(
        response.results.len(),
        1,
        "Expected 1 result after URL dedup, got {}",
        response.results.len()
    );
    assert_eq!(
        response.results[0].relevance, 0.9,
        "Should keep the higher-relevance result"
    );
}

// ===========================================================================
// Test 8: dedup_by_snippet_prefix
// ===========================================================================

/// Two sources return results with different URLs but identical first 200 chars
/// of snippet (whitespace-normalized) -> only the higher-scored one is kept.
#[tokio::test]
async fn dedup_by_snippet_prefix() {
    // Both snippets share the same first 200 chars (whitespace-normalized).
    // They have DIFFERENT URLs, so URL dedup doesn't apply -- snippet dedup does.
    let shared_prefix = "A".repeat(200);

    // source_a: snippet is the shared prefix + extra unique text
    let snippet_a = format!("{shared_prefix} unique tail from source A with more content");
    // source_b: snippet is the shared prefix (with extra interior whitespace that
    // normalizes to the same thing) + different extra text
    let snippet_b = format!("{shared_prefix}  unique tail from source B completely different");

    let source_a = MockSource::new(
        "source_a",
        vec![make_result_with_snippet(
            "source_a",
            "doc_a",
            0.9,
            &snippet_a,
        )],
    );
    let source_b = MockSource::new(
        "source_b",
        vec![make_result_with_snippet(
            "source_b",
            "doc_b",
            0.5,
            &snippet_b,
        )],
    );

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source_a), boxed(source_b)],
        default_config(),
    );

    let response = orchestrator.search(&query("test")).await;

    // Same normalized snippet prefix -> dedup -> keep higher-scored (0.9)
    assert_eq!(
        response.results.len(),
        1,
        "Expected 1 result after snippet-prefix dedup, got {}",
        response.results.len()
    );
    assert_eq!(
        response.results[0].relevance, 0.9,
        "Should keep the higher-relevance result"
    );
    assert_eq!(response.results[0].source, "source_a");
}

// ===========================================================================
// Test 9: max_results_truncation
// ===========================================================================

/// Two sources each return 15 results (30 total), max_results=20 ->
/// exactly 20 returned.
#[tokio::test]
async fn max_results_truncation() {
    let results_a: Vec<SearchResult> = (0..15)
        .map(|i| make_result("source_a", &format!("a_{i}"), 0.9 - (i as f32) * 0.01))
        .collect();
    let results_b: Vec<SearchResult> = (0..15)
        .map(|i| make_result("source_b", &format!("b_{i}"), 0.85 - (i as f32) * 0.01))
        .collect();

    let source_a = MockSource::new("source_a", results_a);
    let source_b = MockSource::new("source_b", results_b);

    let config = OrchestratorConfig {
        timeout_seconds: 30,
        source_weights: HashMap::new(),
        max_results: 20,
    };

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(source_a), boxed(source_b)],
        config,
    );

    let q = SearchQuery {
        text: "test".to_string(),
        max_results: 20,
        filters: SearchFilters::default(),
    };

    let response = orchestrator.search(&q).await;

    assert_eq!(
        response.results.len(),
        20,
        "Expected exactly 20 results after truncation, got {}",
        response.results.len()
    );
}

// ===========================================================================
// Test 10: source_filter_in_query
// ===========================================================================

/// Orchestrator has [slack, confluence, jira]; query filters to sources=["slack"]
/// -> only slack queried, total_sources_queried=1.
#[tokio::test]
async fn source_filter_in_query() {
    let slack_source = MockSource::new(
        "slack",
        vec![make_result("slack", "slack_msg", 0.9)],
    );
    let confluence_source = MockSource::new(
        "confluence",
        vec![make_result("confluence", "conf_page", 0.8)],
    );
    let jira_source = MockSource::new(
        "jira",
        vec![make_result("jira", "jira_ticket", 0.7)],
    );

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(slack_source), boxed(confluence_source), boxed(jira_source)],
        default_config(),
    );

    // Filter: only query "slack"
    let q = SearchQuery {
        text: "test".to_string(),
        max_results: 50,
        filters: SearchFilters {
            sources: Some(vec!["slack".to_string()]),
            after: None,
            before: None,
        },
    };

    let response = orchestrator.search(&q).await;

    // Only slack results should appear
    assert_eq!(
        response.results.len(),
        1,
        "Expected 1 result (slack only), got {}",
        response.results.len()
    );
    assert_eq!(response.results[0].source, "slack");

    // total_sources_queried should reflect that only 1 source was queried
    assert_eq!(
        response.total_sources_queried, 1,
        "Expected total_sources_queried=1 (only slack), got {}",
        response.total_sources_queried
    );
}

// ===========================================================================
// Test 11: health_check_all
// ===========================================================================

/// Healthy + unhealthy sources -> returns [Healthy, Unavailable].
#[tokio::test]
async fn health_check_all() {
    let healthy_source = MockSource::new("healthy_src", vec![]);
    let unhealthy_source = MockSource::new("unhealthy_src", vec![]).unhealthy();

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(healthy_source), boxed(unhealthy_source)],
        default_config(),
    );

    let health_results: Vec<SourceHealth> = orchestrator.health_check_all().await;

    assert_eq!(health_results.len(), 2, "Expected 2 health check results");

    // Find each source's health by name
    let healthy = health_results
        .iter()
        .find(|h| h.source == "healthy_src")
        .expect("Should have health for healthy_src");
    let unhealthy = health_results
        .iter()
        .find(|h| h.source == "unhealthy_src")
        .expect("Should have health for unhealthy_src");

    assert!(
        matches!(healthy.status, HealthStatus::Healthy),
        "Expected Healthy, got {:?}",
        healthy.status
    );
    assert!(
        matches!(unhealthy.status, HealthStatus::Unavailable),
        "Expected Unavailable, got {:?}",
        unhealthy.status
    );
}

// ===========================================================================
// Test 12: empty_sources_list
// ===========================================================================

/// No sources -> 0 results, 0 warnings, total_sources_queried=0.
#[tokio::test]
async fn empty_sources_list() {
    let orchestrator = SearchOrchestrator::new(vec![], default_config());

    let response = orchestrator.search(&query("anything")).await;

    assert_eq!(response.results.len(), 0, "Expected 0 results with no sources");
    assert!(
        response.warnings.is_empty(),
        "Expected 0 warnings with no sources, got: {:?}",
        response.warnings
    );
    assert_eq!(
        response.total_sources_queried, 0,
        "Expected total_sources_queried=0"
    );
}

// ===========================================================================
// Test 13: panic_source_doesnt_crash
// ===========================================================================

/// One working MockSource + one PanicSource -> ok results + warning about
/// the panic. The orchestrator must NOT crash.
#[tokio::test]
async fn panic_source_doesnt_crash() {
    let ok_source = MockSource::new(
        "reliable",
        vec![
            make_result("reliable", "safe_result_1", 0.9),
            make_result("reliable", "safe_result_2", 0.7),
        ],
    );

    let orchestrator = SearchOrchestrator::new(
        vec![boxed(ok_source), boxed(PanicSource)],
        default_config(),
    );

    let response = orchestrator.search(&query("test")).await;

    // The reliable source's results should still be returned
    assert_eq!(
        response.results.len(),
        2,
        "Expected 2 results from the reliable source"
    );
    assert!(
        response.results.iter().all(|r| r.source == "reliable"),
        "All results should be from the reliable source"
    );

    // Should have a warning about the panic
    assert!(
        !response.warnings.is_empty(),
        "Expected at least one warning about the panicking source"
    );
    let warnings_lower: Vec<String> = response
        .warnings
        .iter()
        .map(|w: &String| w.to_lowercase())
        .collect();
    let has_panic_warning = warnings_lower.iter().any(|w: &String| {
        w.contains("panic")
            || w.contains("panic_source")
            || w.contains("crashed")
            || w.contains("failed")
    });
    assert!(
        has_panic_warning,
        "Expected a warning mentioning the panic or crash, got: {:?}",
        response.warnings
    );
}
