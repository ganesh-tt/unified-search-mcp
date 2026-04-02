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

    /// Fetch full details for a single JIRA issue and return Markdown.
    pub async fn get_detail_issue(&self, key: &str) -> Result<String, SearchError> {
        let url = format!("{}/rest/api/3/issue/{}", self.config.base_url, key);
        let fields = "summary,description,status,assignee,reporter,labels,fixVersions,issuelinks,subtasks,comment,priority,issuetype,created,updated";

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.email, Some(&self.config.api_token))
            .query(&[("fields", fields)])
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

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SearchError::Source {
                source_name: "jira".to_string(),
                message: format!("Issue {} not found (404)", key),
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

        let issue_key = body
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or(key);

        let fields_obj = body.get("fields").cloned().unwrap_or(serde_json::Value::Null);

        let summary = fields_obj
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let status_name = fields_obj
            .get("status")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown");

        let issue_type = fields_obj
            .get("issuetype")
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown");

        let priority = fields_obj
            .get("priority")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown");

        let assignee = fields_obj
            .get("assignee")
            .and_then(|a| a.get("displayName"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unassigned");

        let reporter = fields_obj
            .get("reporter")
            .and_then(|r| r.get("displayName"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown");

        let labels: Vec<&str> = fields_obj
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let fix_versions: Vec<&str> = fields_obj
            .get("fixVersions")
            .and_then(|fv| fv.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        let created = fields_obj
            .get("created")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let updated = fields_obj
            .get("updated")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Build Markdown output
        let mut md = String::new();

        // Title
        md.push_str(&format!("# {}: {}\n\n", issue_key, summary));

        // Metadata table
        md.push_str("| Field | Value |\n");
        md.push_str("|-------|-------|\n");
        md.push_str(&format!("| Status | {} |\n", status_name));
        md.push_str(&format!("| Type | {} |\n", issue_type));
        md.push_str(&format!("| Priority | {} |\n", priority));
        md.push_str(&format!("| Assignee | {} |\n", assignee));
        md.push_str(&format!("| Reporter | {} |\n", reporter));
        md.push_str(&format!("| Labels | {} |\n", labels.join(", ")));
        md.push_str(&format!("| Fix Versions | {} |\n", fix_versions.join(", ")));
        md.push_str(&format!("| Created | {} |\n", created));
        md.push_str(&format!("| Updated | {} |\n", updated));
        md.push('\n');

        // Description
        md.push_str("## Description\n\n");
        if let Some(desc) = fields_obj.get("description") {
            if !desc.is_null() {
                let desc_text = Self::extract_adf_text(desc);
                if !desc_text.is_empty() {
                    md.push_str(&desc_text);
                } else {
                    md.push_str("_No description._");
                }
            } else {
                md.push_str("_No description._");
            }
        } else {
            md.push_str("_No description._");
        }
        md.push_str("\n\n");

        // Linked Issues
        let issue_links = fields_obj
            .get("issuelinks")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default();

        if !issue_links.is_empty() {
            md.push_str("## Linked Issues\n\n");
            for link in &issue_links {
                let link_type = link
                    .get("type")
                    .and_then(|t| t.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("related");

                if let Some(outward_issue) = link.get("outwardIssue") {
                    let outward_label = link
                        .get("type")
                        .and_then(|t| t.get("outward"))
                        .and_then(|n| n.as_str())
                        .unwrap_or(link_type);
                    let linked_key = outward_issue
                        .get("key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("?");
                    let linked_summary = outward_issue
                        .get("fields")
                        .and_then(|f| f.get("summary"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let linked_status = outward_issue
                        .get("fields")
                        .and_then(|f| f.get("status"))
                        .and_then(|s| s.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown");
                    md.push_str(&format!(
                        "- **{}** {}: {} [{}]\n",
                        outward_label, linked_key, linked_summary, linked_status
                    ));
                }

                if let Some(inward_issue) = link.get("inwardIssue") {
                    let inward_label = link
                        .get("type")
                        .and_then(|t| t.get("inward"))
                        .and_then(|n| n.as_str())
                        .unwrap_or(link_type);
                    let linked_key = inward_issue
                        .get("key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("?");
                    let linked_summary = inward_issue
                        .get("fields")
                        .and_then(|f| f.get("summary"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let linked_status = inward_issue
                        .get("fields")
                        .and_then(|f| f.get("status"))
                        .and_then(|s| s.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown");
                    md.push_str(&format!(
                        "- **{}** {}: {} [{}]\n",
                        inward_label, linked_key, linked_summary, linked_status
                    ));
                }
            }
            md.push('\n');
        }

        // Subtasks
        let subtasks = fields_obj
            .get("subtasks")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();

        if !subtasks.is_empty() {
            md.push_str("## Subtasks\n\n");
            for subtask in &subtasks {
                let st_key = subtask
                    .get("key")
                    .and_then(|k| k.as_str())
                    .unwrap_or("?");
                let st_summary = subtask
                    .get("fields")
                    .and_then(|f| f.get("summary"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let st_status = subtask
                    .get("fields")
                    .and_then(|f| f.get("status"))
                    .and_then(|s| s.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("Unknown");
                let checkbox = if st_status == "Done" { "[x]" } else { "[ ]" };
                md.push_str(&format!("- {} {} — {}\n", checkbox, st_key, st_summary));
            }
            md.push('\n');
        }

        // Comments
        let comments = fields_obj
            .get("comment")
            .and_then(|c| c.get("comments"))
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        let comment_total = fields_obj
            .get("comment")
            .and_then(|c| c.get("total"))
            .and_then(|t| t.as_u64())
            .unwrap_or(comments.len() as u64);

        if !comments.is_empty() {
            md.push_str(&format!("## Comments ({})\n\n", comment_total));
            for comment in &comments {
                let author = comment
                    .get("author")
                    .and_then(|a| a.get("displayName"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("Unknown");
                let created_date = comment
                    .get("created")
                    .and_then(|c| c.as_str())
                    .and_then(|s| {
                        chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
                            .ok()
                            .map(|dt| dt.format("%Y-%m-%d").to_string())
                    })
                    .unwrap_or_default();
                let body_text = comment
                    .get("body")
                    .map(|b| Self::extract_adf_text(b))
                    .unwrap_or_default();

                md.push_str(&format!("### {} — {}\n\n{}\n\n", author, created_date, body_text));
            }
        }

        Ok(md)
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
            let mut snippet = if let Some(desc) = fields_obj.get("description") {
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

            // Extract comments from the search response
            let comments = fields_obj
                .get("comment")
                .and_then(|c| c.get("comments"))
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            let comment_count = fields_obj
                .get("comment")
                .and_then(|c| c.get("total"))
                .and_then(|t| t.as_u64())
                .unwrap_or(comments.len() as u64);

            metadata.insert("comment_count".to_string(), comment_count.to_string());

            // Append latest 3 comments to snippet (most recent first)
            if !comments.is_empty() {
                let mut comment_texts: Vec<(String, String, String)> = Vec::new();
                for comment in comments.iter().rev().take(3) {
                    let author = comment
                        .get("author")
                        .and_then(|a| a.get("displayName"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown");
                    let created = comment
                        .get("created")
                        .and_then(|c| c.as_str())
                        .and_then(|s| {
                            DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
                                .ok()
                                .map(|dt| dt.format("%Y-%m-%d").to_string())
                        })
                        .unwrap_or_default();
                    let body_raw = comment
                        .get("body")
                        .map(|b| Self::extract_adf_text(b))
                        .unwrap_or_default();
                    let body = Self::truncate_description(&body_raw, 150);
                    comment_texts.push((author.to_string(), created, body));
                }

                snippet.push_str(&format!("\n---\nComments ({} total):", comment_count));
                for (author, date, body) in &comment_texts {
                    snippet.push_str(&format!("\n[{}, {}]: {}", author, date, body));
                }
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
