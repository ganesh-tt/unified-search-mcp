use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;

use crate::models::{HealthStatus, SearchError, SearchQuery, SearchResult, SourceHealth};
use crate::sources::SearchSource;

/// Configuration for the Confluence search source.
#[derive(Debug, Clone)]
pub struct ConfluenceConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    /// Optional list of space keys to restrict search to.
    pub spaces: Vec<String>,
    pub max_results: usize,
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

// Comment API response types
#[derive(Debug, Deserialize)]
struct ConfluenceCommentResponse {
    results: Vec<ConfluenceComment>,
}

#[derive(Debug, Deserialize)]
struct ConfluenceComment {
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
    pub fn new(config: ConfluenceConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build reqwest client");

        let html_tag_re = Regex::new(r"<[^>]+>").expect("Invalid HTML tag regex");

        Self {
            config,
            client,
            html_tag_re,
        }
    }

    /// Build the CQL query string from the search query and config.
    fn build_cql(&self, query: &SearchQuery) -> String {
        // Escape double quotes in the query text to prevent CQL injection
        let escaped_text = query.text.replace('"', r#"\""#);

        let mut cql = format!(r#"siteSearch ~ "{}""#, escaped_text);

        // Add space filter if configured
        if !self.config.spaces.is_empty() {
            let space_list: Vec<String> = self
                .config
                .spaces
                .iter()
                .map(|s| format!(r#""{}""#, s))
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

    /// Build the Basic Auth header value.
    fn auth_header(&self) -> String {
        let credentials = format!("{}:{}", self.config.email, self.config.api_token);
        let encoded = base64_encode_simple(&credentials);
        format!("Basic {}", encoded)
    }
}

/// Simple base64 encoding without external dependency.
fn base64_encode_simple(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
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
            .header("Authorization", self.auth_header())
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
            .header("Authorization", self.auth_header())
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

        let body_text = response.text().await.map_err(|e| SearchError::Source {
            source_name: "confluence".to_string(),
            message: format!("Failed to read response body: {}", e),
        })?;

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

        let results = self.enrich_with_comments(results).await;
        Ok(results)
    }
}

impl ConfluenceSource {
    /// Fetch full page details and return a formatted Markdown string.
    pub async fn get_detail_page(&self, page_id: &str) -> Result<String, SearchError> {
        let url = format!("{}/wiki/rest/api/content/{}", self.config.base_url, page_id);

        let response = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
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

        let body_text = response.text().await.map_err(|e| SearchError::Source {
            source_name: "confluence".to_string(),
            message: format!("Failed to read response body: {}", e),
        })?;

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

        Ok(md)
    }
}

impl ConfluenceSource {
    /// Fetch comments in parallel for each result that has a page_id in metadata.
    async fn enrich_with_comments(&self, mut results: Vec<SearchResult>) -> Vec<SearchResult> {
        use futures::future::join_all;

        let client = self.client.clone();
        let base_url = self.config.base_url.clone();
        let auth = self.auth_header();

        // Build futures for each result
        let futures: Vec<_> = results
            .iter()
            .map(|r| {
                let page_id = r.metadata.get("page_id").cloned();
                let client = client.clone();
                let base_url = base_url.clone();
                let auth = auth.clone();

                async move {
                    let id = match page_id {
                        Some(id) => id,
                        None => return None,
                    };

                    let url = format!(
                        "{}/wiki/rest/api/content/{}/child/comment",
                        base_url, id
                    );

                    let resp = client
                        .get(&url)
                        .header("Authorization", auth)
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
                }
            })
            .collect();

        let comment_batches = join_all(futures).await;

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

                        // Latest 3 comments (reversed = most recent first)
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
                                .and_then(|w| {
                                    chrono::DateTime::parse_from_rfc3339(w).ok()
                                })
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
                            let body_truncated = if body_text.len() > 150 {
                                format!("{}…", &body_text[..150])
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

        results
    }
}
