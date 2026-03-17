use std::fmt::Write;

use serde_json;

use crate::core::SearchOrchestrator;
use crate::models::{SearchFilters, SearchQuery, SearchResult};

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
}

impl UnifiedSearchServer {
    /// Create a new server backed by the given orchestrator.
    pub fn new(orchestrator: SearchOrchestrator) -> Self {
        Self { orchestrator }
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

        let response = self.orchestrator.search(&search_query).await;

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
    ) -> String {
        let max = max_results.unwrap_or(20);
        let search_query = SearchQuery {
            text: query,
            max_results: max,
            filters: SearchFilters {
                sources: Some(vec![source]),
                after: None,
                before: None,
            },
        };

        let response = self.orchestrator.search(&search_query).await;
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
