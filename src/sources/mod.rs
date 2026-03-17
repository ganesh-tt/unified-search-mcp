use async_trait::async_trait;
use crate::models::{SearchQuery, SearchResult, SearchError, SourceHealth};

pub mod slack;
pub mod confluence;
pub mod jira;
pub mod local_text;
// pub mod local_vector; // Phase 2

#[async_trait]
pub trait SearchSource: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn health_check(&self) -> SourceHealth;
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
}
