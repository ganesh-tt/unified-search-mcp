use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::models::{SearchQuery, SearchResult, SourceHealth, UnifiedSearchResponse};
use crate::sources::SearchSource;

/// Configuration for the search orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub timeout_seconds: u64,
    pub source_weights: HashMap<String, f32>,
    pub max_results: usize,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 10,
            source_weights: HashMap::new(),
            max_results: 20,
        }
    }
}

/// Orchestrates searches across multiple sources with fan-out, merge, rank, and dedup.
pub struct SearchOrchestrator {
    sources: Vec<Arc<dyn SearchSource>>,
    config: OrchestratorConfig,
}

impl SearchOrchestrator {
    pub fn new(sources: Vec<Box<dyn SearchSource>>, config: OrchestratorConfig) -> Self {
        let sources = sources.into_iter().map(|s| Arc::from(s)).collect();
        Self { sources, config }
    }

    pub async fn search(&self, query: &SearchQuery) -> UnifiedSearchResponse {
        let start = Instant::now();

        // Step 1: Determine which sources to query based on filters
        let active_sources: Vec<Arc<dyn SearchSource>> = if let Some(ref filter_sources) =
            query.filters.sources
        {
            self.sources
                .iter()
                .filter(|s| filter_sources.contains(&s.name().to_string()))
                .cloned()
                .collect()
        } else {
            self.sources.clone()
        };

        let total_sources_queried = active_sources.len();

        // Step 2: Fan-out searches with timeout via tokio::spawn
        let timeout_duration = Duration::from_secs(self.config.timeout_seconds);
        let query_clone = query.clone();

        let mut handles = Vec::new();
        for source in &active_sources {
            let source = Arc::clone(source);
            let q = query_clone.clone();
            let timeout_dur = timeout_duration;

            let handle = tokio::spawn(async move {
                let name = source.name().to_string();
                let source_start = std::time::Instant::now();
                let result = tokio::time::timeout(timeout_dur, source.search(&q)).await;
                let latency_ms = source_start.elapsed().as_millis() as u64;
                (name, result, latency_ms)
            });
            handles.push(handle);
        }

        // Step 3: Collect results
        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut per_source_stats: Vec<crate::models::PerSourceStats> = Vec::new();

        for handle in handles {
            match handle.await {
                Ok((source_name, timeout_result, latency_ms)) => match timeout_result {
                    Ok(search_result) => match search_result {
                        Ok(results) => {
                            let count = results.len();
                            let comment_count: usize = results.iter()
                                .filter_map(|r| r.metadata.get("comment_count"))
                                .filter_map(|c| c.parse::<usize>().ok())
                                .sum();
                            per_source_stats.push(crate::models::PerSourceStats {
                                source: source_name,
                                latency_ms,
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
                                latency_ms,
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
                    // Panic caught by tokio::spawn
                    warnings.push("Source task panicked or crashed".to_string());
                }
            }
        }

        // Step 5: Apply source weights for sorting
        // Build a vec of (effective_score, result) for sorting
        let mut scored_results: Vec<(f32, SearchResult)> = all_results
            .into_iter()
            .map(|r| {
                let weight = self
                    .config
                    .source_weights
                    .get(&r.source)
                    .copied()
                    .unwrap_or(1.0);
                let effective_score = r.relevance * weight;
                (effective_score, r)
            })
            .collect();

        // Step 6: Sort by effective score DESC, then by timestamp DESC (None last)
        scored_results.sort_by(|(score_a, result_a), (score_b, result_b)| {
            // Primary: effective score descending
            let score_cmp = score_b
                .partial_cmp(score_a)
                .unwrap_or(std::cmp::Ordering::Equal);
            if score_cmp != std::cmp::Ordering::Equal {
                return score_cmp;
            }
            // Secondary: timestamp descending (None last)
            match (&result_a.timestamp, &result_b.timestamp) {
                (Some(a), Some(b)) => b.cmp(a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        // Step 7: Dedup
        let mut deduped: Vec<SearchResult> = Vec::new();

        for (_score, result) in scored_results {
            let dominated = deduped.iter().any(|kept| {
                // Same URL dedup (both Some and equal)
                if let (Some(ref url_a), Some(ref url_b)) = (&result.url, &kept.url) {
                    if url_a == url_b {
                        return true;
                    }
                }
                // Same normalized snippet prefix dedup
                let norm_a = normalize_snippet_prefix(&result.snippet);
                let norm_b = normalize_snippet_prefix(&kept.snippet);
                if norm_a == norm_b {
                    return true;
                }
                false
            });

            if !dominated {
                deduped.push(result);
            }
        }

        // Step 8: Truncate to min(query.max_results, config.max_results)
        let max = std::cmp::min(query.max_results, self.config.max_results);
        deduped.truncate(max);

        let query_time_ms = start.elapsed().as_millis() as u64;

        UnifiedSearchResponse {
            results: deduped,
            warnings,
            total_sources_queried,
            query_time_ms,
            per_source_stats,
            cache_hit: false,
        }
    }

    pub async fn health_check_all(&self) -> Vec<SourceHealth> {
        let mut results = Vec::new();
        for source in &self.sources {
            results.push(source.health_check().await);
        }
        results
    }
}

/// Normalize a snippet for dedup: take first 200 chars, collapse whitespace to single spaces.
fn normalize_snippet_prefix(snippet: &str) -> String {
    let collapsed: String = snippet
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    collapsed.chars().take(200).collect()
}
