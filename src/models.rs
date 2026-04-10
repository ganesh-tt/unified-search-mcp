use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub max_results: usize,
    pub filters: SearchFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchFilters {
    pub sources: Option<Vec<String>>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub source: String,
    pub title: String,
    pub snippet: String,
    pub url: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub relevance: f32,
    pub metadata: HashMap<String, String>,
}

impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SearchResult {}

impl PartialOrd for SearchResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SearchResult {
    fn cmp(&self, other: &Self) -> Ordering {
        // Primary: relevance descending (higher relevance sorts first / is "less")
        let rel_cmp = other
            .relevance
            .partial_cmp(&self.relevance)
            .unwrap_or(Ordering::Equal);
        if rel_cmp != Ordering::Equal {
            return rel_cmp;
        }
        // Secondary: timestamp descending (more recent first), None sorts last
        match (&self.timestamp, &other.timestamp) {
            (Some(at), Some(bt)) => bt.cmp(at),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceHealth {
    pub source: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unavailable,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unavailable => write!(f, "unavailable"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerSourceStats {
    pub source: String,
    pub latency_ms: u64,
    pub result_count: usize,
    pub comment_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResponse {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    pub total_sources_queried: usize,
    pub query_time_ms: u64,
    pub per_source_stats: Vec<PerSourceStats>,
    #[serde(default)]
    pub cache_hit: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum SearchError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{source_name}: authentication failed — {message}")]
    Auth {
        source_name: String,
        message: String,
    },
    #[error("{source_name}: rate limited — retry after {retry_after_secs}s")]
    RateLimited {
        source_name: String,
        retry_after_secs: u64,
    },
    #[error("{source_name}: {message}")]
    Source {
        source_name: String,
        message: String,
    },
    #[error("Config error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            max_results: 20,
            filters: SearchFilters::default(),
        }
    }
}
