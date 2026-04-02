use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::process::Command;
use tokio::time::timeout;

use crate::models::{
    HealthStatus, SearchError, SearchQuery, SearchResult, SourceHealth,
};
use super::SearchSource;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub orgs: Vec<String>,
    pub repos: Vec<String>,
    pub max_results: usize,
    /// Path to the `gh` CLI binary. Defaults to `"gh"` in production;
    /// override with a test script path in tests.
    pub gh_path: String,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            orgs: Vec::new(),
            repos: Vec::new(),
            max_results: 20,
            gh_path: "gh".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

pub struct GitHubSource {
    config: GitHubConfig,
}

impl GitHubSource {
    pub fn new(config: GitHubConfig) -> Self {
        Self { config }
    }

    /// Run a `gh` CLI command and return its stdout on success.
    async fn run_gh(&self, args: &[&str]) -> Result<String, SearchError> {
        let result = timeout(Duration::from_secs(10), async {
            let child = Command::new(&self.config.gh_path)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| SearchError::Source {
                    source_name: "github".to_string(),
                    message: format!("Failed to spawn gh CLI: {}", e),
                })?;

            let output = child.wait_with_output().await.map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to read gh output: {}", e),
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Check for rate limiting
                if stderr.to_lowercase().contains("rate limit") {
                    return Err(SearchError::RateLimited {
                        source_name: "github".to_string(),
                        retry_after_secs: 60,
                    });
                }

                // Check for auth errors
                if stderr.to_lowercase().contains("authentication")
                    || stderr.to_lowercase().contains("not logged")
                {
                    return Err(SearchError::Auth {
                        source_name: "github".to_string(),
                        message: stderr.trim().to_string(),
                    });
                }

                return Err(SearchError::Source {
                    source_name: "github".to_string(),
                    message: format!(
                        "gh exited with status {}: {}",
                        output.status,
                        stderr.trim()
                    ),
                });
            }

            String::from_utf8(output.stdout).map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Invalid UTF-8 in gh output: {}", e),
            })
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => Err(SearchError::Source {
                source_name: "github".to_string(),
                message: "gh command timed out after 10 seconds".to_string(),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // get_detail: PRs
    // -----------------------------------------------------------------------

    /// Fetch full detail for a GitHub Pull Request and render as Markdown.
    pub async fn get_detail_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<String, SearchError> {
        // Fetch all three endpoints in parallel
        let pr_path = format!("repos/{}/{}/pulls/{}", owner, repo, number);
        let reviews_path = format!("repos/{}/{}/pulls/{}/reviews", owner, repo, number);
        let comments_path = format!("repos/{}/{}/pulls/{}/comments", owner, repo, number);

        let pr_args = ["api", pr_path.as_str()];
        let reviews_args = ["api", reviews_path.as_str()];
        let comments_args = ["api", comments_path.as_str()];

        let (pr_result, reviews_result, comments_result) = tokio::join!(
            self.run_gh(&pr_args),
            self.run_gh(&reviews_args),
            self.run_gh(&comments_args),
        );

        let pr_json: serde_json::Value = serde_json::from_str(&pr_result?)
            .map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse PR JSON: {}", e),
            })?;

        let reviews_json: serde_json::Value = serde_json::from_str(&reviews_result?)
            .map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse reviews JSON: {}", e),
            })?;

        let comments_json: serde_json::Value = serde_json::from_str(&comments_result?)
            .map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse comments JSON: {}", e),
            })?;

        // Extract PR metadata
        let title = pr_json.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let state = pr_json.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
        let merged_at = pr_json.get("merged_at").and_then(|v| v.as_str());
        let status = if merged_at.is_some() {
            "Merged"
        } else {
            match state {
                "open" => "Open",
                "closed" => "Closed",
                _ => state,
            }
        };
        let author = pr_json
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let head_ref = pr_json
            .get("head")
            .and_then(|h| h.get("ref"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let base_ref = pr_json
            .get("base")
            .and_then(|b| b.get("ref"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let created_at = pr_json.get("created_at").and_then(|v| v.as_str()).unwrap_or("?");
        let updated_at = pr_json.get("updated_at").and_then(|v| v.as_str()).unwrap_or("?");
        let additions = pr_json.get("additions").and_then(|v| v.as_u64()).unwrap_or(0);
        let deletions = pr_json.get("deletions").and_then(|v| v.as_u64()).unwrap_or(0);
        let changed_files = pr_json.get("changed_files").and_then(|v| v.as_u64()).unwrap_or(0);
        let body = pr_json.get("body").and_then(|v| v.as_str()).unwrap_or("*No description provided.*");

        let mut md = String::new();
        md.push_str(&format!("# {}/{}#{}: {}\n\n", owner, repo, number, title));
        md.push_str("| Field | Value |\n|---|---|\n");
        md.push_str(&format!("| Status | {} |\n", status));
        md.push_str(&format!("| Author | {} |\n", author));
        md.push_str(&format!("| Branch | {} → {} |\n", head_ref, base_ref));
        md.push_str(&format!("| Created | {} |\n", created_at));
        md.push_str(&format!("| Updated | {} |\n", updated_at));
        if let Some(m) = merged_at {
            md.push_str(&format!("| Merged | {} |\n", m));
        }
        md.push_str(&format!(
            "| Changes | +{} -{} across {} files |\n",
            additions, deletions, changed_files
        ));

        md.push_str(&format!("\n## Description\n\n{}\n", body));

        // Reviews
        let reviews = reviews_json.as_array().cloned().unwrap_or_default();
        md.push_str(&format!("\n## Reviews ({})\n", reviews.len()));
        for review in &reviews {
            let reviewer = review
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let review_state = review.get("state").and_then(|v| v.as_str()).unwrap_or("PENDING");
            let submitted_at = review.get("submitted_at").and_then(|v| v.as_str()).unwrap_or("?");
            let review_body = review.get("body").and_then(|v| v.as_str()).unwrap_or("");

            md.push_str(&format!("\n### @{} — {} — {}\n", reviewer, review_state, submitted_at));
            if !review_body.is_empty() {
                md.push_str(&format!("{}\n", review_body));
            }
        }

        // Review comments (line-level)
        let comments = comments_json.as_array().cloned().unwrap_or_default();
        md.push_str(&format!("\n## Review Comments ({})\n", comments.len()));
        for comment in &comments {
            let commenter = comment
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let path = comment.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let line = comment.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
            let comment_created = comment.get("created_at").and_then(|v| v.as_str()).unwrap_or("?");
            let comment_body = comment.get("body").and_then(|v| v.as_str()).unwrap_or("");

            md.push_str(&format!(
                "\n### @{} on {}:{} — {}\n{}\n",
                commenter, path, line, comment_created, comment_body
            ));
        }

        Ok(md)
    }

    // -----------------------------------------------------------------------
    // get_detail: Issues
    // -----------------------------------------------------------------------

    /// Fetch full detail for a GitHub Issue and render as Markdown.
    pub async fn get_detail_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<String, SearchError> {
        let issue_path = format!("repos/{}/{}/issues/{}", owner, repo, number);
        let comments_path = format!("repos/{}/{}/issues/{}/comments", owner, repo, number);

        let issue_args = ["api", issue_path.as_str()];
        let comments_args = ["api", comments_path.as_str()];

        let (issue_result, comments_result) = tokio::join!(
            self.run_gh(&issue_args),
            self.run_gh(&comments_args),
        );

        let issue_json: serde_json::Value = serde_json::from_str(&issue_result?)
            .map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse issue JSON: {}", e),
            })?;

        let comments_json: serde_json::Value = serde_json::from_str(&comments_result?)
            .map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse comments JSON: {}", e),
            })?;

        // Extract issue metadata
        let title = issue_json.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let state = issue_json.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
        let status = match state {
            "open" => "Open",
            "closed" => "Closed",
            _ => state,
        };
        let author = issue_json
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let created_at = issue_json.get("created_at").and_then(|v| v.as_str()).unwrap_or("?");
        let updated_at = issue_json.get("updated_at").and_then(|v| v.as_str()).unwrap_or("?");
        let body = issue_json.get("body").and_then(|v| v.as_str()).unwrap_or("*No description provided.*");

        // Labels
        let labels: Vec<&str> = issue_json
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        let mut md = String::new();
        md.push_str(&format!("# {}/{}#{}: {}\n\n", owner, repo, number, title));
        md.push_str("| Field | Value |\n|---|---|\n");
        md.push_str(&format!("| Status | {} |\n", status));
        md.push_str(&format!("| Author | {} |\n", author));
        if !labels.is_empty() {
            md.push_str(&format!("| Labels | {} |\n", labels.join(", ")));
        }
        md.push_str(&format!("| Created | {} |\n", created_at));
        md.push_str(&format!("| Updated | {} |\n", updated_at));

        md.push_str(&format!("\n## Description\n\n{}\n", body));

        // Comments
        let comments = comments_json.as_array().cloned().unwrap_or_default();
        md.push_str(&format!("\n## Comments ({})\n", comments.len()));
        for comment in &comments {
            let commenter = comment
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let comment_created = comment.get("created_at").and_then(|v| v.as_str()).unwrap_or("?");
            let comment_body = comment.get("body").and_then(|v| v.as_str()).unwrap_or("");

            md.push_str(&format!("\n### @{} — {}\n{}\n", commenter, comment_created, comment_body));
        }

        Ok(md)
    }

    /// Build the query qualifier for org/repo scoping.
    fn build_scope_qualifier(&self) -> String {
        if !self.config.repos.is_empty() {
            // When specific repos are configured, use repo: qualifiers
            self.config
                .repos
                .iter()
                .map(|r| format!("repo:{}", r))
                .collect::<Vec<_>>()
                .join(" ")
        } else if !self.config.orgs.is_empty() {
            // When only orgs are configured, use org: qualifiers
            self.config
                .orgs
                .iter()
                .map(|o| format!("org:{}", o))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        }
    }

    /// Search issues and PRs via `gh api search/issues`.
    async fn search_issues(
        &self,
        query: &str,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let scope = self.build_scope_qualifier();
        let full_query = if scope.is_empty() {
            query.to_string()
        } else {
            format!("{} {}", query, scope)
        };

        let per_page = self.config.max_results.to_string();
        let stdout = self
            .run_gh(&[
                "api",
                "search/issues",
                "--method",
                "GET",
                "-f",
                &format!("q={}", full_query),
                "-f",
                &format!("per_page={}", per_page),
            ])
            .await?;

        let body: serde_json::Value =
            serde_json::from_str(&stdout).map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse issues JSON: {}", e),
            })?;

        let items = body
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Find max score for normalization
        let max_score = items
            .iter()
            .filter_map(|item| item.get("score").and_then(|s| s.as_f64()))
            .fold(0.0_f64, f64::max);

        let results: Vec<SearchResult> = items
            .iter()
            .map(|item| {
                let number = item
                    .get("number")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let title_text = item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let html_url = item
                    .get("html_url")
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string());
                let body_text = item
                    .get("body")
                    .and_then(|b| b.as_str())
                    .unwrap_or("");
                let state = item
                    .get("state")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                let is_pr = item.get("pull_request").is_some();

                // Extract repo name from repository_url
                let repo_name = item
                    .get("repository_url")
                    .and_then(|u| u.as_str())
                    .and_then(|url| {
                        // https://api.github.com/repos/org/repo -> org/repo
                        url.strip_prefix("https://api.github.com/repos/")
                    })
                    .unwrap_or("unknown");

                let kind = if is_pr { "PR" } else { "Issue" };
                let title = format!("{}#{}: {} [{}]", repo_name, number, title_text, kind);

                // Snippet: first 200 chars of body
                let snippet = if body_text.len() > 200 {
                    format!("{}...", &body_text[..200])
                } else {
                    body_text.to_string()
                };

                // Parse timestamp
                let timestamp = item
                    .get("updated_at")
                    .and_then(|t| t.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                // Normalize score
                let raw_score = item
                    .get("score")
                    .and_then(|s| s.as_f64())
                    .unwrap_or(0.0);
                let relevance = if max_score > 0.0 {
                    (raw_score / max_score) as f32
                } else {
                    0.0
                };

                let mut metadata = HashMap::new();
                metadata.insert("repo".to_string(), repo_name.to_string());
                metadata.insert("state".to_string(), state.to_string());
                metadata.insert("kind".to_string(), kind.to_string());

                SearchResult {
                    source: "github".to_string(),
                    title,
                    snippet,
                    url: html_url,
                    timestamp,
                    relevance,
                    metadata,
                }
            })
            .collect();

        Ok(results)
    }

    /// Search code via `gh api search/code`.
    async fn search_code(
        &self,
        query: &str,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let scope = self.build_scope_qualifier();
        let full_query = if scope.is_empty() {
            query.to_string()
        } else {
            format!("{} {}", query, scope)
        };

        let per_page = self.config.max_results.to_string();
        let stdout = self
            .run_gh(&[
                "api",
                "search/code",
                "--method",
                "GET",
                "-f",
                &format!("q={}", full_query),
                "-f",
                &format!("per_page={}", per_page),
            ])
            .await?;

        let body: serde_json::Value =
            serde_json::from_str(&stdout).map_err(|e| SearchError::Source {
                source_name: "github".to_string(),
                message: format!("Failed to parse code search JSON: {}", e),
            })?;

        let items = body
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Find max score for normalization
        let max_score = items
            .iter()
            .filter_map(|item| item.get("score").and_then(|s| s.as_f64()))
            .fold(0.0_f64, f64::max);

        let results: Vec<SearchResult> = items
            .iter()
            .map(|item| {
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let file_path = item
                    .get("path")
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                let html_url = item
                    .get("html_url")
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string());
                let repo_full_name = item
                    .get("repository")
                    .and_then(|r| r.get("full_name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");

                let title = format!("{}: {}", repo_full_name, file_path);
                let snippet = file_path.to_string();

                // Normalize score
                let raw_score = item
                    .get("score")
                    .and_then(|s| s.as_f64())
                    .unwrap_or(0.0);
                let relevance = if max_score > 0.0 {
                    (raw_score / max_score) as f32
                } else {
                    0.0
                };

                let mut metadata = HashMap::new();
                metadata.insert("repo".to_string(), repo_full_name.to_string());
                metadata.insert("file".to_string(), name.to_string());
                metadata.insert("kind".to_string(), "code".to_string());

                SearchResult {
                    source: "github".to_string(),
                    title,
                    snippet,
                    url: html_url,
                    timestamp: None,
                    relevance,
                    metadata,
                }
            })
            .collect();

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Trait impl
// ---------------------------------------------------------------------------

#[async_trait]
impl SearchSource for GitHubSource {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "GitHub issues, PRs, and code search via gh CLI"
    }

    async fn health_check(&self) -> SourceHealth {
        let start = Instant::now();

        let result = self
            .run_gh(&["auth", "status", "--hostname", "github.com"])
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(_) => SourceHealth {
                source: "github".to_string(),
                status: HealthStatus::Healthy,
                message: Some("gh auth OK".to_string()),
                latency_ms: Some(latency_ms),
            },
            Err(e) => SourceHealth {
                source: "github".to_string(),
                status: HealthStatus::Unavailable,
                message: Some(format!("{}", e)),
                latency_ms: Some(latency_ms),
            },
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        // Run issues/PRs and code searches in parallel
        let (issues_result, code_result) = tokio::join!(
            self.search_issues(&query.text),
            self.search_code(&query.text),
        );

        let mut results = Vec::new();

        // Collect issues/PRs results (or warn on error)
        match issues_result {
            Ok(mut items) => results.append(&mut items),
            Err(e) => {
                // If one search fails, still return results from the other.
                // Log via tracing but don't fail the whole search.
                tracing::warn!("GitHub issues search failed: {}", e);
            }
        }

        // Collect code results (or warn on error)
        match code_result {
            Ok(mut items) => results.append(&mut items),
            Err(e) => {
                tracing::warn!("GitHub code search failed: {}", e);
            }
        }

        // Limit to max_results
        results.truncate(query.max_results);

        Ok(results)
    }
}
