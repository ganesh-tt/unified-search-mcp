use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use crate::models::{
    HealthStatus, SearchError, SearchQuery, SearchResult, SourceHealth,
};
use crate::sources::SearchSource;

// ===========================================================================
// Config
// ===========================================================================

#[derive(Debug, Clone)]
pub struct JiraConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    pub projects: Vec<String>,
    pub max_results: usize,
}

// ===========================================================================
// Source
// ===========================================================================

pub struct JiraSource {
    config: JiraConfig,
    client: Client,
}

impl JiraSource {
    pub fn new(config: JiraConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        Self { config, client }
    }

    /// Build JQL query string from search text, project filters, and time filters.
    fn build_jql(&self, query: &SearchQuery) -> String {
        let escaped = query.text.replace('"', "\\\"");
        let mut parts = vec![format!("text ~ \"{}\"", escaped)];

        // Project filter
        if !self.config.projects.is_empty() {
            let project_list: Vec<String> = self
                .config
                .projects
                .iter()
                .map(|p| format!("\"{}\"", p))
                .collect();
            parts.push(format!("project IN ({})", project_list.join(",")));
        }

        // Time filters
        if let Some(after) = &query.filters.after {
            parts.push(format!("updated >= \"{}\"", after.format("%Y-%m-%d")));
        }
        if let Some(before) = &query.filters.before {
            parts.push(format!("updated <= \"{}\"", before.format("%Y-%m-%d")));
        }

        parts.join(" AND ")
    }

    /// Extract plain text from an Atlassian Document Format (ADF) JSON value.
    /// Walks the content tree recursively, collecting text from "text" nodes.
    fn extract_adf_text(value: &serde_json::Value) -> String {
        let mut result = String::new();
        Self::walk_adf_node(value, &mut result);
        result
    }

    fn walk_adf_node(node: &serde_json::Value, out: &mut String) {
        if let Some(obj) = node.as_object() {
            // If this node has a "text" field and type is "text", grab it
            if obj.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                    out.push_str(text);
                }
            }
            // Recurse into "content" array
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    Self::walk_adf_node(child, out);
                }
            }
        }
    }

    /// Truncate text to max_len chars, appending "..." if truncated.
    fn truncate_description(text: &str, max_len: usize) -> String {
        if text.len() > max_len {
            let truncated: String = text.chars().take(max_len).collect();
            format!("{}...", truncated)
        } else {
            text.to_string()
        }
    }

    /// Extract project key from issue key (e.g., "FIN-1234" -> "FIN").
    fn project_from_key(key: &str) -> String {
        key.split('-').next().unwrap_or(key).to_string()
    }
}

#[async_trait]
impl SearchSource for JiraSource {
    fn name(&self) -> &str {
        "jira"
    }

    fn description(&self) -> &str {
        "JIRA issue search"
    }

    async fn health_check(&self) -> SourceHealth {
        let start = Instant::now();
        let url = format!("{}/rest/api/3/myself", self.config.base_url);

        let result = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) if resp.status().is_success() => SourceHealth {
                source: "jira".to_string(),
                status: HealthStatus::Healthy,
                message: Some("OK".to_string()),
                latency_ms: Some(latency_ms),
            },
            Ok(resp) => SourceHealth {
                source: "jira".to_string(),
                status: HealthStatus::Unavailable,
                message: Some(format!("HTTP {}", resp.status())),
                latency_ms: Some(latency_ms),
            },
            Err(e) => SourceHealth {
                source: "jira".to_string(),
                status: HealthStatus::Unavailable,
                message: Some(format!("Connection error: {}", e)),
                latency_ms: Some(latency_ms),
            },
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let jql = self.build_jql(query);
        let url = format!("{}/rest/api/3/search", self.config.base_url);

        let fields = "summary,description,comment,status,updated,assignee";

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .query(&[
                ("jql", jql.as_str()),
                ("maxResults", &self.config.max_results.to_string()),
                ("fields", fields),
            ])
            .send()
            .await
            .map_err(|e| SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SearchError::Auth {
                source_name: "jira".to_string(),
                message: "Authentication failed — check email and API token".to_string(),
            });
        }

        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::Source {
                source_name: "jira".to_string(),
                message: "Permission denied — forbidden".to_string(),
            });
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(SearchError::RateLimited {
                source_name: "jira".to_string(),
                retry_after_secs: retry_after,
            });
        }

        if status.is_server_error() {
            return Err(SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("Server error: HTTP {}", status),
            });
        }

        if !status.is_success() {
            return Err(SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("Unexpected HTTP status: {}", status),
            });
        }

        let body: serde_json::Value =
            response.json().await.map_err(|e| SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("Failed to parse response JSON: {}", e),
            })?;

        let issues = body
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let total = issues.len();
        let mut results = Vec::with_capacity(total);

        for (i, issue) in issues.iter().enumerate() {
            let key = issue
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");

            let fields_obj = issue.get("fields").cloned().unwrap_or(serde_json::Value::Null);

            let summary = fields_obj
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let title = format!("{}: {}", key, summary);

            // Extract description text from ADF format
            let snippet = if let Some(desc) = fields_obj.get("description") {
                if desc.is_null() {
                    String::new()
                } else {
                    let raw_text = Self::extract_adf_text(desc);
                    Self::truncate_description(&raw_text, 300)
                }
            } else {
                String::new()
            };

            let url = Some(format!("{}/browse/{}", self.config.base_url, key));

            // Parse timestamp
            let timestamp = fields_obj
                .get("updated")
                .and_then(|v| v.as_str())
                .and_then(|s| {
                    // JIRA uses ISO 8601 with timezone offset
                    DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });

            // Position-based relevance: first result gets highest score
            let relevance = if total > 1 {
                1.0 - (i as f32 / total as f32)
            } else {
                1.0
            };

            // Build metadata
            let mut metadata = HashMap::new();
            metadata.insert("project".to_string(), Self::project_from_key(key));

            if let Some(status_name) = fields_obj
                .get("status")
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
            {
                metadata.insert("status".to_string(), status_name.to_string());
            }

            if let Some(assignee_name) = fields_obj
                .get("assignee")
                .and_then(|a| a.get("displayName"))
                .and_then(|n| n.as_str())
            {
                metadata.insert("assignee".to_string(), assignee_name.to_string());
            }

            results.push(SearchResult {
                source: "jira".to_string(),
                title,
                snippet,
                url,
                timestamp,
                relevance,
                metadata,
            });
        }

        Ok(results)
    }
}
