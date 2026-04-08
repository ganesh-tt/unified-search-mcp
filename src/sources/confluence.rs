use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use reqwest::{Client, Response};
use serde::Deserialize;

use crate::models::{HealthStatus, SearchError, SearchQuery, SearchResult, SourceHealth};
use crate::sources::SearchSource;

/// Warn threshold for response bodies (10 MB). Larger responses are still read
/// (user needs complete data) but a warning is logged for monitoring.
const LARGE_RESPONSE_WARN_BYTES: u64 = 10 * 1024 * 1024;

/// Read a response body as text, logging a warning for very large responses.
async fn read_body_checked(response: Response, source: &str) -> Result<String, SearchError> {
    if let Some(len) = response.content_length() {
        if len > LARGE_RESPONSE_WARN_BYTES {
            tracing::warn!(
                source = source,
                content_length = len,
                "Large response body ({:.1} MB) — may cause high memory usage",
                len as f64 / (1024.0 * 1024.0),
            );
        }
    }
    response.text().await.map_err(|e| SearchError::Source {
        source_name: source.to_string(),
        message: format!("Failed to read response body: {}", e),
    })
}

/// Configuration for the Confluence search source.
#[derive(Clone)]
pub struct ConfluenceConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    /// Optional list of space keys to restrict search to.
    pub spaces: Vec<String>,
    pub max_results: usize,
}

impl std::fmt::Debug for ConfluenceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfluenceConfig")
            .field("base_url", &self.base_url)
            .field("email", &self.email)
            .field("api_token", &"[REDACTED]")
            .field("spaces", &self.spaces)
            .field("max_results", &self.max_results)
            .finish()
    }
}

/// Confluence search source using the v1 REST API with CQL.
pub struct ConfluenceSource {
    config: ConfluenceConfig,
    client: Client,
    html_tag_re: Regex,
}

// Page detail API response types
#[derive(Debug, Deserialize)]
struct ConfluencePageDetail {
    #[allow(dead_code)]
    id: Option<String>,
    title: Option<String>,
    space: Option<ConfluencePageSpace>,
    body: Option<ConfluencePageBody>,
    version: Option<ConfluencePageVersion>,
    children: Option<ConfluencePageChildren>,
    metadata: Option<ConfluencePageMetadata>,
    #[serde(rename = "_links")]
    links: Option<ConfluencePageLinks>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageSpace {
    key: Option<String>,
    #[allow(dead_code)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageBody {
    storage: Option<ConfluencePageStorage>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageStorage {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageVersion {
    by: Option<ConfluencePageVersionBy>,
    when: Option<String>,
    #[allow(dead_code)]
    number: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageVersionBy {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageChildren {
    page: Option<ConfluenceChildPageCollection>,
    comment: Option<ConfluenceChildCommentCollection>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceChildPageCollection {
    results: Vec<ConfluenceChildPage>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceChildPage {
    title: Option<String>,
    #[serde(rename = "_links")]
    links: Option<ConfluenceChildPageLinks>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceChildPageLinks {
    webui: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceChildCommentCollection {
    results: Vec<ConfluenceChildComment>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceChildComment {
    body: Option<ConfluencePageBody>,
    version: Option<ConfluencePageVersion>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageMetadata {
    labels: Option<ConfluenceLabelCollection>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceLabelCollection {
    results: Vec<ConfluenceLabel>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceLabel {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluencePageLinks {
    webui: Option<String>,
    base: Option<String>,
}


impl ConfluenceSource {
    pub fn new(config: ConfluenceConfig) -> Self {
        let client = Self::build_client();
        Self::new_with_client(config, client)
    }

    /// Create with a shared reqwest::Client (avoids duplicate connection pools).
    pub fn new_with_client(config: ConfluenceConfig, client: Client) -> Self {
        let html_tag_re = Regex::new(r"<[^>]+>").expect("Invalid HTML tag regex");
        Self { config, client, html_tag_re }
    }

    /// Build the default HTTP client for Confluence.
    pub fn build_client() -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("Failed to build reqwest client")
    }

    /// Build the CQL query string from the search query and config.
    fn build_cql(&self, query: &SearchQuery) -> String {
        // Escape backslashes FIRST, then quotes, to prevent CQL injection.
        // If we escape quotes first, a literal `\"` in input becomes `\\"` which
        // closes the CQL string and allows injection.
        let escaped_text = query.text.replace('\\', r#"\\"#).replace('"', r#"\""#);

        let mut cql = format!(r#"siteSearch ~ "{}""#, escaped_text);

        // Add space filter if configured
        if !self.config.spaces.is_empty() {
            let space_list: Vec<String> = self
                .config
                .spaces
                .iter()
                .map(|s| {
                    let escaped = s.replace('\\', r#"\\"#).replace('"', r#"\""#);
                    format!(r#""{}""#, escaped)
                })
                .collect();
            cql.push_str(&format!(" AND space IN ({})", space_list.join(",")));
        }

        // Add time filters
        if let Some(ref after) = query.filters.after {
            let date_str = after.format("%Y-%m-%d").to_string();
            cql.push_str(&format!(r#" AND lastmodified >= "{}""#, date_str));
        }
        if let Some(ref before) = query.filters.before {
            let date_str = before.format("%Y-%m-%d").to_string();
            cql.push_str(&format!(r#" AND lastmodified <= "{}""#, date_str));
        }

        cql
    }

    /// Strip HTML tags from an excerpt string.
    fn strip_html(&self, html: &str) -> String {
        self.html_tag_re.replace_all(html, "").to_string()
    }

}

// Confluence API response types (v1)
#[derive(Debug, Deserialize)]
struct ConfluenceSearchResponse {
    results: Vec<ConfluenceSearchResult>,
    #[allow(dead_code)]
    #[serde(default)]
    size: usize,
}

#[derive(Debug, Deserialize)]
struct ConfluenceSearchResult {
    content: Option<ConfluenceContent>,
    excerpt: Option<String>,
    url: Option<String>,
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    #[serde(rename = "resultGlobalContainer")]
    result_global_container: Option<ConfluenceContainer>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceContent {
    id: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    content_type: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceContainer {
    #[allow(dead_code)]
    title: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "displayUrl")]
    display_url: Option<String>,
}

#[async_trait]
impl SearchSource for ConfluenceSource {
    fn name(&self) -> &str {
        "confluence"
    }

    fn description(&self) -> &str {
        "Confluence wiki search via CQL"
    }

    async fn health_check(&self) -> SourceHealth {
        let start = Instant::now();
        let url = format!("{}/wiki/rest/api/space", self.config.base_url);

        let result = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .query(&[("limit", "1")])
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) if resp.status().is_success() => SourceHealth {
                source: "confluence".to_string(),
                status: HealthStatus::Healthy,
                message: Some("OK".to_string()),
                latency_ms: Some(latency_ms),
            },
            Ok(resp) => SourceHealth {
                source: "confluence".to_string(),
                status: HealthStatus::Unavailable,
                message: Some(format!("HTTP {}", resp.status().as_u16())),
                latency_ms: Some(latency_ms),
            },
            Err(e) => SourceHealth {
                source: "confluence".to_string(),
                status: HealthStatus::Unavailable,
                message: Some(format!("Connection error: {}", e)),
                latency_ms: Some(latency_ms),
            },
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let cql = self.build_cql(query);
        let url = format!("{}/wiki/rest/api/search", self.config.base_url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .query(&[
                ("cql", cql.as_str()),
                ("limit", &self.config.max_results.to_string()),
            ])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SearchError::Source {
                        source_name: "confluence".to_string(),
                        message: format!("Request timed out: {}", e),
                    }
                } else {
                    SearchError::Http(e)
                }
            })?;

        let status = response.status();

        if status.as_u16() == 401 {
            return Err(SearchError::Auth {
                source_name: "confluence".to_string(),
                message: "authentication failed — check email and api_token".to_string(),
            });
        }

        if status.as_u16() == 403 {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: "permission denied — forbidden (403)".to_string(),
            });
        }

        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);

            return Err(SearchError::RateLimited {
                source_name: "confluence".to_string(),
                retry_after_secs: retry_after,
            });
        }

        if status.is_server_error() {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("server error ({})", status.as_u16()),
            });
        }

        if !status.is_success() {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("unexpected status {}", status.as_u16()),
            });
        }

        let body_text = read_body_checked(response, "confluence").await?;

        let api_response: ConfluenceSearchResponse =
            serde_json::from_str(&body_text).map_err(|e| SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("Failed to parse JSON: {}", e),
            })?;

        let total = api_response.results.len();
        let results = api_response
            .results
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                let title = r
                    .content
                    .as_ref()
                    .and_then(|c| c.title.clone())
                    .unwrap_or_default();

                let snippet = r
                    .excerpt
                    .map(|e| self.strip_html(&e))
                    .unwrap_or_default();

                let full_url = r.url.map(|u| {
                    if u.starts_with("http") {
                        u
                    } else {
                        format!("{}{}", self.config.base_url, u)
                    }
                });

                let timestamp: Option<DateTime<Utc>> = r
                    .last_modified
                    .and_then(|lm| DateTime::parse_from_rfc3339(&lm).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                // Position-based relevance: first=1.0, linearly decreasing
                let relevance = if total <= 1 {
                    1.0
                } else {
                    1.0 - (i as f32 / (total as f32 - 1.0)) * (1.0 - 0.1)
                };

                let mut metadata = HashMap::new();
                if let Some(ref container) = r.result_global_container {
                    if let Some(ref space_title) = container.title {
                        metadata.insert("space".to_string(), space_title.clone());
                    }
                }

                // Store page_id for comment enrichment
                if let Some(ref content) = r.content {
                    if let Some(ref id) = content.id {
                        metadata.insert("page_id".to_string(), id.clone());
                    }
                }

                SearchResult {
                    source: "confluence".to_string(),
                    title,
                    snippet,
                    url: full_url,
                    timestamp,
                    relevance,
                    metadata,
                }
            })
            .collect();

        // Comment enrichment skipped during search — it added 800ms-40s of
        // latency (up to 20 extra HTTP calls) for data that `get_detail` already
        // returns in full. The comment_count metadata field is left as "0"; callers
        // should use `get_detail` for comment details.
        Ok(results)
    }
}

impl ConfluenceSource {
    /// Fetch full page details and return a formatted Markdown string.
    pub async fn get_detail_page(&self, page_id: &str) -> Result<String, SearchError> {
        let page_start = std::time::Instant::now();
        tracing::info!(page_id = %page_id, "confluence: get_detail_page starting");

        // Validate page_id is all digits to prevent path-traversal and URL injection.
        if page_id.is_empty() || !page_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("Invalid page ID (must be numeric): {}", page_id),
            });
        }

        let url = format!("{}/wiki/rest/api/content/{}", self.config.base_url, page_id);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .query(&[(
                "expand",
                "body.storage,version,children.page,children.comment.body.storage,metadata.labels,space",
            )])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SearchError::Source {
                        source_name: "confluence".to_string(),
                        message: format!("Request timed out: {}", e),
                    }
                } else {
                    SearchError::Http(e)
                }
            })?;

        let status = response.status();
        tracing::debug!(
            page_id = %page_id,
            status = status.as_u16(),
            elapsed_ms = page_start.elapsed().as_millis() as u64,
            "confluence: HTTP response received",
        );

        if status.as_u16() == 401 {
            return Err(SearchError::Auth {
                source_name: "confluence".to_string(),
                message: "authentication failed — check email and api_token".to_string(),
            });
        }

        if status.as_u16() == 404 {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("page not found (404): {}", page_id),
            });
        }

        if !status.is_success() {
            return Err(SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("unexpected status {}", status.as_u16()),
            });
        }

        let body_text = read_body_checked(response, "confluence").await?;

        tracing::debug!(
            page_id = %page_id,
            body_bytes = body_text.len(),
            elapsed_ms = page_start.elapsed().as_millis() as u64,
            "confluence: response body read complete",
        );

        let page: ConfluencePageDetail =
            serde_json::from_str(&body_text).map_err(|e| SearchError::Source {
                source_name: "confluence".to_string(),
                message: format!("Failed to parse JSON: {}", e),
            })?;

        let mut md = String::new();

        // Title
        let title = page.title.as_deref().unwrap_or("Untitled");
        md.push_str(&format!("# {}\n\n", title));

        // Metadata table
        let space_key = page
            .space
            .as_ref()
            .and_then(|s| s.key.as_deref())
            .unwrap_or("");
        let author = page
            .version
            .as_ref()
            .and_then(|v| v.by.as_ref())
            .and_then(|b| b.display_name.as_deref())
            .unwrap_or("");
        let last_updated = page
            .version
            .as_ref()
            .and_then(|v| v.when.as_deref())
            .and_then(|w| chrono::DateTime::parse_from_rfc3339(w).ok())
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let labels: Vec<&str> = page
            .metadata
            .as_ref()
            .and_then(|m| m.labels.as_ref())
            .map(|lc| lc.results.iter().filter_map(|l| l.name.as_deref()).collect())
            .unwrap_or_default();
        let labels_str = labels.join(", ");

        // Build page URL
        let page_url = {
            let base = page
                .links
                .as_ref()
                .and_then(|l| l.base.as_deref())
                .unwrap_or(&self.config.base_url);
            let webui = page
                .links
                .as_ref()
                .and_then(|l| l.webui.as_deref())
                .unwrap_or("");
            format!("{}{}", base, webui)
        };

        md.push_str("| Field | Value |\n");
        md.push_str("|-------|-------|\n");
        md.push_str(&format!("| Space | {} |\n", space_key));
        md.push_str(&format!("| Author | {} |\n", author));
        md.push_str(&format!("| Last Updated | {} |\n", last_updated));
        md.push_str(&format!("| Labels | {} |\n", labels_str));
        md.push_str(&format!("| URL | {} |\n", page_url));
        md.push('\n');

        // Body content
        md.push_str("## Content\n\n");
        let body_html = page
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .and_then(|s| s.value.as_deref())
            .unwrap_or("");
        md.push_str(&super::confluence_markdown::to_markdown(body_html));
        md.push_str("\n\n");

        // Child pages
        if let Some(children) = &page.children {
            if let Some(child_pages) = &children.page {
                if !child_pages.results.is_empty() {
                    md.push_str("## Child Pages\n\n");
                    for child in &child_pages.results {
                        let child_title = child.title.as_deref().unwrap_or("Untitled");
                        let child_link = child
                            .links
                            .as_ref()
                            .and_then(|l| l.webui.as_deref())
                            .unwrap_or("");
                        if child_link.is_empty() {
                            md.push_str(&format!("- {}\n", child_title));
                        } else {
                            md.push_str(&format!(
                                "- [{}]({}{})\n",
                                child_title, self.config.base_url, child_link
                            ));
                        }
                    }
                    md.push('\n');
                }
            }

            // Comments
            if let Some(comment_collection) = &children.comment {
                let comments = &comment_collection.results;
                if !comments.is_empty() {
                    md.push_str(&format!("## Comments ({})\n\n", comments.len()));
                    for comment in comments {
                        let comment_author = comment
                            .version
                            .as_ref()
                            .and_then(|v| v.by.as_ref())
                            .and_then(|b| b.display_name.as_deref())
                            .unwrap_or("Unknown");
                        let comment_date = comment
                            .version
                            .as_ref()
                            .and_then(|v| v.when.as_deref())
                            .and_then(|w| chrono::DateTime::parse_from_rfc3339(w).ok())
                            .map(|dt| dt.format("%Y-%m-%d").to_string())
                            .unwrap_or_default();
                        let comment_html = comment
                            .body
                            .as_ref()
                            .and_then(|b| b.storage.as_ref())
                            .and_then(|s| s.value.as_deref())
                            .unwrap_or("");
                        let comment_text = super::confluence_markdown::to_markdown(comment_html);

                        md.push_str(&format!(
                            "**{} — {}**\n{}\n\n",
                            comment_author,
                            comment_date,
                            comment_text.trim()
                        ));
                    }
                }
            }
        }

        tracing::info!(
            page_id = %page_id,
            elapsed_ms = page_start.elapsed().as_millis() as u64,
            markdown_bytes = md.len(),
            "confluence: get_detail_page complete",
        );
        Ok(md)
    }
}

// ---------------------------------------------------------------------------
// Comment-enriched search (separate path from default search)
// ---------------------------------------------------------------------------

// Comment API response types (used by search_with_comments)
#[derive(Debug, Deserialize)]
struct ConfluenceCommentResponse {
    results: Vec<ConfluenceCommentResult>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceCommentResult {
    body: Option<ConfluenceCommentBody>,
    version: Option<ConfluenceCommentVersion>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceCommentBody {
    storage: Option<ConfluenceCommentStorage>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceCommentStorage {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceCommentVersion {
    by: Option<ConfluenceCommentAuthor>,
    when: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceCommentAuthor {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

impl ConfluenceSource {
    /// Search + enrich results with comment previews. This is a **separate path**
    /// from the default `search()` (which skips enrichment for speed). Use this
    /// when comment context matters and the caller is willing to wait.
    ///
    /// Bounded: max 5 concurrent comment fetches, 10s timeout per request.
    pub async fn search_with_comments(
        &self,
        query: &SearchQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let mut results = self.search(query).await?;
        self.enrich_with_comments(&mut results).await;
        Ok(results)
    }

    /// Fetch comment previews for each result that has a `page_id` in metadata.
    /// At most 5 requests run in parallel, each with a 10s timeout.
    async fn enrich_with_comments(&self, results: &mut [SearchResult]) {
        use futures::stream::{self, StreamExt};

        let client = self.client.clone();
        let base_url = self.config.base_url.clone();
        let email = self.config.email.clone();
        let token = self.config.api_token.clone();

        let futures: Vec<_> = results
            .iter()
            .map(|r| {
                let page_id = r.metadata.get("page_id").cloned();
                let client = client.clone();
                let base_url = base_url.clone();
                let email = email.clone();
                let token = token.clone();

                async move {
                    let id = match page_id {
                        Some(id) => id,
                        None => return None,
                    };

                    let url = format!(
                        "{}/wiki/rest/api/content/{}/child/comment",
                        base_url, id
                    );

                    let page_id_for_log = id.clone();
                    let fetch = async {
                        let resp = client
                            .get(&url)
                            .basic_auth(&email, Some(&token))
                            .query(&[("expand", "body.storage,version"), ("limit", "25")])
                            .send()
                            .await;

                        match resp {
                            Ok(r) if r.status().is_success() => {
                                let text = r.text().await.ok()?;
                                let parsed: ConfluenceCommentResponse =
                                    serde_json::from_str(&text).ok()?;
                                Some(parsed.results)
                            }
                            _ => None,
                        }
                    };

                    match tokio::time::timeout(std::time::Duration::from_secs(10), fetch).await {
                        Ok(result) => result,
                        Err(_) => {
                            tracing::warn!(
                                page_id = %page_id_for_log,
                                "enrich_with_comments: timed out after 10s",
                            );
                            None
                        }
                    }
                }
            })
            .collect();

        let comment_batches: Vec<_> = stream::iter(futures)
            .buffered(5)
            .collect()
            .await;

        for (result, comments_opt) in results.iter_mut().zip(comment_batches.into_iter()) {
            match comments_opt {
                Some(comments) => {
                    let count = comments.len();
                    result
                        .metadata
                        .insert("comment_count".to_string(), count.to_string());

                    if count > 0 {
                        let mut snippet_addition =
                            format!("\n---\nComments ({} total):", count);

                        for comment in comments.iter().rev().take(3) {
                            let author = comment
                                .version
                                .as_ref()
                                .and_then(|v| v.by.as_ref())
                                .and_then(|b| b.display_name.as_deref())
                                .unwrap_or("Unknown");

                            let date = comment
                                .version
                                .as_ref()
                                .and_then(|v| v.when.as_deref())
                                .and_then(|w| chrono::DateTime::parse_from_rfc3339(w).ok())
                                .map(|dt| dt.format("%Y-%m-%d").to_string())
                                .unwrap_or_default();

                            let body_html = comment
                                .body
                                .as_ref()
                                .and_then(|b| b.storage.as_ref())
                                .and_then(|s| s.value.as_deref())
                                .unwrap_or("");

                            let body_text = self.html_tag_re.replace_all(body_html, "").to_string();
                            let body_text = body_text.trim();
                            let body_truncated = if body_text.chars().count() > 150 {
                                format!("{}...", body_text.chars().take(150).collect::<String>())
                            } else {
                                body_text.to_string()
                            };

                            snippet_addition.push_str(&format!(
                                "\n[{}, {}]: {}",
                                author, date, body_truncated
                            ));
                        }

                        result.snippet.push_str(&snippet_addition);
                    }
                }
                None => {
                    result
                        .metadata
                        .insert("comment_count".to_string(), "0".to_string());
                }
            }
        }
    }
}
