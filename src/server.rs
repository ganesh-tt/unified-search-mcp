use std::fmt::Write;

use serde_json;

use crate::core::SearchOrchestrator;
use crate::metrics::{MetricsEntry, MetricsLogger};
use crate::models::{SearchFilters, SearchQuery, SearchResult};
use crate::resolve::{detect_source, force_source, ParsedIdentifier, SourceType};
use crate::sources::confluence::ConfluenceSource;
use crate::sources::github::GitHubSource;
use crate::sources::jira::JiraSource;
use crate::sources::slack::SlackSource;

/// MCP server wrapping a [`SearchOrchestrator`].
///
/// Each `handle_*` method corresponds to one MCP tool:
///
/// | Tool                | Method                    |
/// |---------------------|---------------------------|
/// | `unified_search`    | `handle_unified_search`   |
/// | `search_source`     | `handle_search_source`    |
/// | `list_sources`      | `handle_list_sources`     |
/// | `index_local`       | `handle_index_local`      |
pub struct UnifiedSearchServer {
    orchestrator: SearchOrchestrator,
    jira_source: Option<JiraSource>,
    confluence_source: Option<ConfluenceSource>,
    slack_source: Option<SlackSource>,
    github_source: Option<GitHubSource>,
    metrics: Option<MetricsLogger>,
}

impl UnifiedSearchServer {
    /// Create a new server backed by the given orchestrator, with optional
    /// per-source instances for `get_detail` lookups.
    pub fn new(
        orchestrator: SearchOrchestrator,
        jira_source: Option<JiraSource>,
        confluence_source: Option<ConfluenceSource>,
        slack_source: Option<SlackSource>,
        github_source: Option<GitHubSource>,
        metrics: Option<MetricsLogger>,
    ) -> Self {
        Self {
            orchestrator,
            jira_source,
            confluence_source,
            slack_source,
            github_source,
            metrics,
        }
    }

    // -----------------------------------------------------------------------
    // Tool: unified_search
    // -----------------------------------------------------------------------

    /// Search across all (or selected) sources and return results as a
    /// Markdown table suitable for display in an MCP-capable client.
    ///
    /// If more than 50 results are returned the full set is saved to
    /// `~/.unified-search/last-search-results.json` and only the top 20 are
    /// included in the response.
    pub async fn handle_unified_search(
        &self,
        query: String,
        sources: Option<Vec<String>>,
        max_results: Option<usize>,
        no_cache: bool,
    ) -> String {
        let max = max_results.unwrap_or(20);
        let search_query = SearchQuery {
            text: query,
            max_results: max,
            filters: SearchFilters {
                sources,
                after: None,
                before: None,
            },
        };

        let response = self.orchestrator.search(&search_query, no_cache).await;

        // Determine if we need to truncate and save
        let display_results: &[SearchResult];
        let overflow_note: Option<String>;

        if response.results.len() > 50 {
            // Save full results to disk
            let save_path = save_full_results(&response.results);
            display_results = &response.results[..20];
            overflow_note = Some(format!(
                "\n> **Note**: {} total results. Showing top 20. Full results saved to `{}`.\n",
                response.results.len(),
                save_path,
            ));
        } else {
            display_results = &response.results;
            overflow_note = None;
        }

        // Build Markdown table
        let mut md = String::new();
        let _ = writeln!(md, "| # | Source | Title | Snippet | URL |");
        let _ = writeln!(md, "|---|--------|-------|---------|-----|");

        for (i, result) in display_results.iter().enumerate() {
            let snippet = truncate_snippet(&result.snippet, 80);
            let url = result
                .url
                .as_deref()
                .unwrap_or("-");
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} | {} |",
                i + 1,
                result.source,
                result.title,
                snippet,
                url,
            );
        }

        if let Some(note) = overflow_note {
            md.push_str(&note);
        }

        // Footer: warnings
        md.push('\n');
        if !response.warnings.is_empty() {
            let warnings_joined = response.warnings.join("; ");
            let _ = writeln!(md, "**Warnings**: {warnings_joined}");
        }

        // Footer: sources queried + time
        let _ = write!(
            md,
            "**Sources queried**: {} | **Time**: {}ms",
            response.total_sources_queried, response.query_time_ms,
        );

        if response.cache_hit {
            let _ = write!(md, " | **Cache**: HIT");
        }

        // Emit metrics
        if let Some(ref metrics) = self.metrics {
            let sources_queried: Vec<String> = response
                .per_source_stats
                .iter()
                .map(|s| s.source.clone())
                .collect();
            let sources_list = if sources_queried.is_empty() {
                vec!["unknown".to_string()]
            } else {
                sources_queried
            };
            metrics
                .log(MetricsEntry::Search {
                    tool: "unified_search".to_string(),
                    query: search_query.text.clone(),
                    sources_queried: sources_list,
                    total_results: response.results.len(),
                    deduped_results: response.results.len(),
                    total_ms: response.query_time_ms,
                })
                .await;
        }

        md
    }

    // -----------------------------------------------------------------------
    // Tool: search_source
    // -----------------------------------------------------------------------

    /// Search a single named source and return results as a JSON array.
    pub async fn handle_search_source(
        &self,
        source: String,
        query: String,
        max_results: Option<usize>,
        no_cache: bool,
    ) -> String {
        let max = max_results.unwrap_or(20);
        let search_query = SearchQuery {
            text: query,
            max_results: max,
            filters: SearchFilters {
                sources: Some(vec![source.clone()]),
                after: None,
                before: None,
            },
        };

        let response = self.orchestrator.search(&search_query, no_cache).await;

        // Emit metrics
        if let Some(ref metrics) = self.metrics {
            let sources_queried: Vec<String> = response
                .per_source_stats
                .iter()
                .map(|s| s.source.clone())
                .collect();
            let sources_list = if sources_queried.is_empty() {
                vec![source]
            } else {
                sources_queried
            };
            metrics
                .log(MetricsEntry::Search {
                    tool: "search_source".to_string(),
                    query: search_query.text.clone(),
                    sources_queried: sources_list,
                    total_results: response.results.len(),
                    deduped_results: response.results.len(),
                    total_ms: response.query_time_ms,
                })
                .await;
        }

        serde_json::to_string_pretty(&response.results)
            .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"))
    }

    // -----------------------------------------------------------------------
    // Tool: list_sources
    // -----------------------------------------------------------------------

    /// Return the health status of all configured sources as a Markdown list.
    pub async fn handle_list_sources(&self) -> String {
        let health_results = self.orchestrator.health_check_all().await;

        let mut md = String::from("# Source Health\n\n");

        for h in &health_results {
            let status_icon = match h.status {
                crate::models::HealthStatus::Healthy => "OK",
                crate::models::HealthStatus::Degraded => "DEGRADED",
                crate::models::HealthStatus::Unavailable => "DOWN",
            };
            let msg = h.message.as_deref().unwrap_or("-");
            let latency = h
                .latency_ms
                .map(|l| format!("{l}ms"))
                .unwrap_or_else(|| "-".to_string());

            let _ = writeln!(
                md,
                "- **{}** — {} ({}) | latency: {}",
                h.source, h.status, status_icon, latency,
            );
            if msg != "-" && msg != "OK" {
                let _ = writeln!(md, "  - {msg}");
            }
        }

        if health_results.is_empty() {
            md.push_str("_No sources configured._\n");
        }

        md
    }

    // -----------------------------------------------------------------------
    // Tool: index_local
    // -----------------------------------------------------------------------

    /// Phase 1 stub — vector search is not yet available.
    pub async fn handle_index_local(&self) -> String {
        "Vector search not enabled. Local file indexing will be available in a future release."
            .to_string()
    }

    // -----------------------------------------------------------------------
    // Tool: get_detail
    // -----------------------------------------------------------------------

    /// Fetch full details for a JIRA ticket, Confluence page, or Slack thread.
    ///
    /// `identifier` can be a JIRA key, a JIRA/Confluence/Slack URL, or a
    /// Confluence page title. The source is auto-detected unless `source` is
    /// explicitly provided.
    pub async fn handle_get_detail(
        &self,
        identifier: String,
        source: Option<String>,
        _max_comments: Option<usize>,
    ) -> String {
        let start = std::time::Instant::now();

        let detection = if let Some(ref src) = source {
            force_source(&identifier, src)
        } else {
            detect_source(&identifier)
        };

        let (source_type, parsed) = match detection {
            Some(pair) => pair,
            None => {
                let result = format!(
                    "Error: Could not detect source type for '{}'. \
                     Provide a `source` parameter ('jira', 'confluence', 'slack').",
                    identifier
                );
                self.emit_detail_metrics(
                    &identifier,
                    "unknown",
                    source.as_deref(),
                    start,
                    0,
                    Some(&result),
                )
                .await;
                return result;
            }
        };

        let detected_source_name = match source_type {
            SourceType::Jira => "jira",
            SourceType::Confluence => "confluence",
            SourceType::Slack => "slack",
            SourceType::GitHub => "github",
        };

        // Execute the detail fetch; capture (result, error, comment_count)
        let (result_text, error_text): (String, Option<String>) = match source_type {
            SourceType::Jira => {
                let key = match parsed {
                    ParsedIdentifier::JiraKey(k) => Some(k),
                    ParsedIdentifier::JiraUrl { key, .. } => Some(key),
                    _ => None,
                };
                match key {
                    None => {
                        let msg = "Error: unexpected parsed identifier for JIRA".to_string();
                        (msg.clone(), Some(msg))
                    }
                    Some(k) => match &self.jira_source {
                        Some(src) => match src.get_detail_issue(&k).await {
                            Ok(md) => (md, None),
                            Err(e) => {
                                let msg = format!("Error: {}", e);
                                (msg.clone(), Some(msg))
                            }
                        },
                        None => {
                            let msg = "Error: JIRA source not configured".to_string();
                            (msg.clone(), Some(msg))
                        }
                    },
                }
            }
            SourceType::Confluence => match parsed {
                ParsedIdentifier::ConfluencePageId(page_id) => {
                    match &self.confluence_source {
                        Some(src) => match src.get_detail_page(&page_id).await {
                            Ok(md) => (md, None),
                            Err(e) => {
                                let msg = format!("Error: {}", e);
                                (msg.clone(), Some(msg))
                            }
                        },
                        None => {
                            let msg = "Error: Confluence source not configured".to_string();
                            (msg.clone(), Some(msg))
                        }
                    }
                }
                ParsedIdentifier::ConfluenceTitle { title, space } => {
                    let msg = format!(
                        "Error: Confluence title lookup not yet implemented. \
                         Use a page URL or ID instead. (title='{}', space={:?})",
                        title, space
                    );
                    (msg.clone(), Some(msg))
                }
                _ => {
                    let msg =
                        "Error: unexpected parsed identifier for Confluence".to_string();
                    (msg.clone(), Some(msg))
                }
            },
            SourceType::Slack => match parsed {
                ParsedIdentifier::SlackPermalink { channel, ts } => {
                    match &self.slack_source {
                        Some(src) => match src.get_detail_thread(&channel, &ts).await {
                            Ok(md) => (md, None),
                            Err(e) => {
                                let msg = format!("Error: {}", e);
                                (msg.clone(), Some(msg))
                            }
                        },
                        None => {
                            let msg = "Error: Slack source not configured".to_string();
                            (msg.clone(), Some(msg))
                        }
                    }
                }
                _ => {
                    let msg =
                        "Error: unexpected parsed identifier for Slack".to_string();
                    (msg.clone(), Some(msg))
                }
            },
            SourceType::GitHub => {
                match &self.github_source {
                    Some(src) => {
                        match parsed {
                            ParsedIdentifier::GitHubPR { owner, repo, number } => {
                                match src.get_detail_pr(&owner, &repo, number).await {
                                    Ok(md) => (md, None),
                                    Err(e) => {
                                        let msg = format!("Error: {}", e);
                                        (msg.clone(), Some(msg))
                                    }
                                }
                            }
                            ParsedIdentifier::GitHubIssue { owner, repo, number } => {
                                match src.get_detail_issue(&owner, &repo, number).await {
                                    Ok(md) => (md, None),
                                    Err(e) => {
                                        let msg = format!("Error: {}", e);
                                        (msg.clone(), Some(msg))
                                    }
                                }
                            }
                            ParsedIdentifier::GitHubShorthand { repo, number } => {
                                // Need to determine if it's a PR or issue -- try PR first, fall back to issue.
                                // For shorthand, we need the org. Use the first org from config.
                                // This is a limitation -- shorthand only works with single-org setups.
                                let owner = src.default_org().unwrap_or_else(|| "unknown".to_string());
                                match src.get_detail_pr(&owner, &repo, number).await {
                                    Ok(md) => (md, None),
                                    Err(_) => {
                                        match src.get_detail_issue(&owner, &repo, number).await {
                                            Ok(md) => (md, None),
                                            Err(e) => {
                                                let msg = format!("Error: {}", e);
                                                (msg.clone(), Some(msg))
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                let msg = "Error: unexpected parsed identifier for GitHub".to_string();
                                (msg.clone(), Some(msg))
                            }
                        }
                    }
                    None => {
                        let msg = "Error: GitHub source not configured".to_string();
                        (msg.clone(), Some(msg))
                    }
                }
            }
        };

        self.emit_detail_metrics(
            &identifier,
            detected_source_name,
            source.as_deref(),
            start,
            0,
            error_text.as_deref(),
        )
        .await;

        result_text
    }

    /// Helper to emit Detail metrics for get_detail calls.
    async fn emit_detail_metrics(
        &self,
        identifier: &str,
        detected_source: &str,
        explicit_source: Option<&str>,
        start: std::time::Instant,
        comments_returned: usize,
        error: Option<&str>,
    ) {
        if let Some(ref metrics) = self.metrics {
            metrics
                .log(MetricsEntry::Detail {
                    tool: "get_detail".to_string(),
                    identifier: identifier.to_string(),
                    detected_source: detected_source.to_string(),
                    explicit_source: explicit_source.map(|s| s.to_string()),
                    latency_ms: start.elapsed().as_millis() as u64,
                    comments_returned,
                    error: error.map(|s| s.to_string()),
                })
                .await;
        }
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Truncate a snippet to `max_chars`, appending "..." if truncated.
fn truncate_snippet(snippet: &str, max_chars: usize) -> String {
    if snippet.len() <= max_chars {
        snippet.to_string()
    } else {
        let truncated: String = snippet.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// Save full search results to `~/.unified-search/last-search-results.json`.
/// Returns the path as a string (for display in the response).
fn save_full_results(results: &[SearchResult]) -> String {
    let dir = shellexpand::tilde("~/.unified-search").to_string();
    let path = format!("{dir}/last-search-results.json");

    // Best-effort: create dir and write file
    let _ = std::fs::create_dir_all(&dir);
    match serde_json::to_string_pretty(results) {
        Ok(json) => {
            let _ = std::fs::write(&path, json);
        }
        Err(_) => {}
    }

    path
}
