use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResponse {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    pub total_sources_queried: usize,
    pub query_time_ms: u64,
}

#[derive(thiserror::Error, Debug)]
pub enum SearchError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{source_name}: authentication failed — {message}")]
    Auth { source_name: String, message: String },
    #[error("{source_name}: rate limited — retry after {retry_after_secs}s")]
    RateLimited { source_name: String, retry_after_secs: u64 },
    #[error("{source_name}: {message}")]
    Source { source_name: String, message: String },
    #[error("Config error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self { text: String::new(), max_results: 20, filters: SearchFilters::default() }
    }
}
