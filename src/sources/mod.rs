use crate::models::{SearchError, SearchQuery, SearchResult, SourceHealth};
use async_trait::async_trait;

pub mod confluence;
pub mod confluence_markdown;
pub mod github;
pub mod jira;
pub mod local_text;
pub mod slack;
// pub mod local_vector; // Phase 2

#[async_trait]
pub trait SearchSource: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn health_check(&self) -> SourceHealth;
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
}
