use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{
    HealthStatus, SearchError, SearchQuery, SearchResult, SourceHealth,
};
use crate::sources::SearchSource;

// ===========================================================================
// Config
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTextConfig {
    pub paths: Vec<PathBuf>,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub max_file_size_bytes: u64,
}

impl Default for LocalTextConfig {
    fn default() -> Self {
        Self {
            paths: vec![],
            include_patterns: vec![],
            exclude_patterns: vec![],
            max_file_size_bytes: 10 * 1024 * 1024, // 10 MB
        }
    }
}

// ===========================================================================
// Source
// ===========================================================================

pub struct LocalTextSource {
    config: LocalTextConfig,
}

impl LocalTextSource {
    pub fn new(config: LocalTextConfig) -> Self {
        Self { config }
    }

    /// Try ripgrep first; fall back to grep-regex + walkdir if rg is not found.
    async fn do_search(
        &self,
        query: &SearchQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let text = query.text.trim();
        if text.is_empty() {
            return Ok(vec![]);
        }

        // Filter out paths that don't exist to avoid rg errors.
        let valid_paths: Vec<&Path> = self
            .config
            .paths
            .iter()
            .filter(|p| p.exists())
            .map(|p| p.as_path())
            .collect();

        if valid_paths.is_empty() {
            return Ok(vec![]);
        }

        // Escape regex special chars so the query is treated as literal text.
        let escaped_query = regex::escape(text);

        match self.search_with_ripgrep(&escaped_query, &valid_paths, query).await {
            Ok(results) => Ok(results),
            Err(_) => {
                // Ripgrep not available — use fallback.
                self.search_with_fallback(&escaped_query, &valid_paths, query)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Ripgrep path
    // -----------------------------------------------------------------------

    async fn search_with_ripgrep(
        &self,
        escaped_query: &str,
        paths: &[&Path],
        query: &SearchQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--json")
            .arg("--max-count")
            .arg("5")
            .arg("--max-filesize")
            .arg(format!("{}B", self.config.max_file_size_bytes))
            .arg("-C") // context lines (default 2 before/after)
            .arg("2");

        // Include/exclude globs
        for pat in &self.config.include_patterns {
            cmd.arg("--glob").arg(pat);
        }
        for pat in &self.config.exclude_patterns {
            cmd.arg("--glob").arg(format!("!{pat}"));
        }

        cmd.arg("--").arg(escaped_query);

        for p in paths {
            cmd.arg(p);
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| SearchError::Source {
            source_name: "local_text".to_string(),
            message: format!("Failed to spawn rg: {e}"),
        })?;

        // rg exit code 1 = no matches (not an error), 2 = actual error
        if output.status.code() == Some(2) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SearchError::Source {
                source_name: "local_text".to_string(),
                message: format!("rg error: {stderr}"),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_rg_json(&stdout, query)
    }

    /// Parse ripgrep JSON output and aggregate matches per file.
    fn parse_rg_json(
        &self,
        json_output: &str,
        query: &SearchQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // Accumulate per-file: (path, match_count, first_snippet_lines, mtime)
        let mut file_matches: HashMap<PathBuf, FileMatch> = HashMap::new();

        for line in json_output.lines() {
            let val: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");

            if msg_type == "match" {
                let data = match val.get("data") {
                    Some(d) => d,
                    None => continue,
                };

                let path_str = data
                    .get("path")
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if path_str.is_empty() {
                    continue;
                }

                let path = PathBuf::from(path_str);
                let line_text = data
                    .get("lines")
                    .and_then(|l| l.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                let entry = file_matches.entry(path.clone()).or_insert_with(|| {
                    FileMatch {
                        path: path.clone(),
                        match_count: 0,
                        snippet_lines: vec![],
                    }
                });
                entry.match_count += 1;
                if entry.snippet_lines.len() < 10 {
                    entry.snippet_lines.push(line_text);
                }
            } else if msg_type == "context" {
                // Add context lines to the most recent file's snippet
                let data = match val.get("data") {
                    Some(d) => d,
                    None => continue,
                };
                let path_str = data
                    .get("path")
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if path_str.is_empty() {
                    continue;
                }
                let path = PathBuf::from(path_str);
                let line_text = data
                    .get("lines")
                    .and_then(|l| l.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(entry) = file_matches.get_mut(&path) {
                    if entry.snippet_lines.len() < 10 {
                        entry.snippet_lines.push(line_text);
                    }
                }
            }
        }

        // Apply time filters and build results
        let max_match_count = file_matches
            .values()
            .map(|fm| fm.match_count)
            .max()
            .unwrap_or(1)
            .max(1);

        let mut results: Vec<SearchResult> = Vec::new();

        for fm in file_matches.values() {
            let mtime = file_mtime(&fm.path);

            // Time filters
            if let Some(after) = &query.filters.after {
                if let Some(mt) = &mtime {
                    if mt < after {
                        continue;
                    }
                }
            }
            if let Some(before) = &query.filters.before {
                if let Some(mt) = &mtime {
                    if mt > before {
                        continue;
                    }
                }
            }

            let relevance = fm.match_count as f32 / max_match_count as f32;
            let snippet = fm.snippet_lines.join("").trim_end().to_string();
            let title = fm.path.display().to_string();
            let url = path_to_file_url(&fm.path);

            let mut metadata = HashMap::new();
            metadata.insert("match_count".to_string(), fm.match_count.to_string());

            results.push(SearchResult {
                source: "local_text".to_string(),
                title,
                snippet,
                url: Some(url),
                timestamp: mtime,
                relevance,
                metadata,
            });
        }

        // Sort by relevance descending
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Truncate to max_results
        results.truncate(query.max_results);

        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Fallback path: grep-regex + walkdir
    // -----------------------------------------------------------------------

    fn search_with_fallback(
        &self,
        escaped_query: &str,
        paths: &[&Path],
        query: &SearchQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        use grep_regex::RegexMatcher;
        use grep_searcher::sinks::UTF8;
        use grep_searcher::Searcher;
        use walkdir::WalkDir;

        let matcher = RegexMatcher::new(escaped_query).map_err(|e| SearchError::Source {
            source_name: "local_text".to_string(),
            message: format!("regex error: {e}"),
        })?;

        let mut file_matches: HashMap<PathBuf, FileMatch> = HashMap::new();

        for base_path in paths {
            for entry in WalkDir::new(base_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                // Skip directories
                if !path.is_file() {
                    continue;
                }

                // Check file size
                if let Ok(meta) = path.metadata() {
                    if meta.len() > self.config.max_file_size_bytes {
                        continue;
                    }
                }

                // Apply include patterns
                if !self.config.include_patterns.is_empty() {
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let matches_any = self.config.include_patterns.iter().any(|pat| {
                        glob_match(pat, file_name, path)
                    });
                    if !matches_any {
                        continue;
                    }
                }

                // Apply exclude patterns
                if !self.config.exclude_patterns.is_empty() {
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let matches_any = self.config.exclude_patterns.iter().any(|pat| {
                        glob_match(pat, file_name, path)
                    });
                    if matches_any {
                        continue;
                    }
                }

                // Search the file
                let mut searcher = Searcher::new();
                let canonical = path.to_path_buf();
                let fm = file_matches.entry(canonical.clone()).or_insert_with(|| {
                    FileMatch {
                        path: canonical.clone(),
                        match_count: 0,
                        snippet_lines: vec![],
                    }
                });

                let fm_count = &mut fm.match_count;
                let fm_snippets = &mut fm.snippet_lines;

                let _ = searcher.search_path(
                    &matcher,
                    path,
                    UTF8(|_line_num, line| {
                        *fm_count += 1;
                        if fm_snippets.len() < 10 {
                            fm_snippets.push(line.to_string());
                        }
                        Ok(true)
                    }),
                );

                // Remove entry if no matches
                if file_matches.get(&canonical).map_or(true, |fm| fm.match_count == 0) {
                    file_matches.remove(&canonical);
                }
            }
        }

        // Build results (same logic as rg path)
        let max_match_count = file_matches
            .values()
            .map(|fm| fm.match_count)
            .max()
            .unwrap_or(1)
            .max(1);

        let mut results: Vec<SearchResult> = Vec::new();

        for fm in file_matches.values() {
            let mtime = file_mtime(&fm.path);

            if let Some(after) = &query.filters.after {
                if let Some(mt) = &mtime {
                    if mt < after {
                        continue;
                    }
                }
            }
            if let Some(before) = &query.filters.before {
                if let Some(mt) = &mtime {
                    if mt > before {
                        continue;
                    }
                }
            }

            let relevance = fm.match_count as f32 / max_match_count as f32;
            let snippet = fm.snippet_lines.join("").trim_end().to_string();
            let title = fm.path.display().to_string();
            let url = path_to_file_url(&fm.path);

            let mut metadata = HashMap::new();
            metadata.insert("match_count".to_string(), fm.match_count.to_string());

            results.push(SearchResult {
                source: "local_text".to_string(),
                title,
                snippet,
                url: Some(url),
                timestamp: mtime,
                relevance,
                metadata,
            });
        }

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(query.max_results);

        Ok(results)
    }
}

// ===========================================================================
// SearchSource trait
// ===========================================================================

#[async_trait]
impl SearchSource for LocalTextSource {
    fn name(&self) -> &str {
        "local_text"
    }

    fn description(&self) -> &str {
        "Local file text search (ripgrep with grep-regex fallback)"
    }

    async fn health_check(&self) -> SourceHealth {
        let any_exists = self.config.paths.iter().any(|p| p.exists());
        if any_exists {
            SourceHealth {
                source: "local_text".to_string(),
                status: HealthStatus::Healthy,
                message: Some("At least one configured path exists".to_string()),
                latency_ms: Some(0),
            }
        } else {
            SourceHealth {
                source: "local_text".to_string(),
                status: HealthStatus::Unavailable,
                message: Some("No configured paths exist".to_string()),
                latency_ms: None,
            }
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        self.do_search(query).await
    }
}

// ===========================================================================
// Internal helpers
// ===========================================================================

struct FileMatch {
    path: PathBuf,
    match_count: usize,
    snippet_lines: Vec<String>,
}

/// Get file modification time as a chrono DateTime<Utc>.
fn file_mtime(path: &Path) -> Option<DateTime<Utc>> {
    let meta = path.metadata().ok()?;
    let mtime: SystemTime = meta.modified().ok()?;
    let dt: DateTime<Utc> = mtime.into();
    Some(dt)
}

/// Convert an absolute path to a file:// URL.
fn path_to_file_url(path: &Path) -> String {
    // Canonicalize if possible, fall back to the original path
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", abs.display())
}

/// Simple glob matching for include/exclude patterns.
/// Handles `*.ext` (extension match) and `**/dir/**` (directory match).
fn glob_match(pattern: &str, file_name: &str, full_path: &Path) -> bool {
    let full_path_str = full_path.display().to_string();

    // Pattern like `**/target/**` — match if any path component is `target`
    if pattern.starts_with("**/") && pattern.ends_with("/**") {
        let dir_name = &pattern[3..pattern.len() - 3];
        let sep = std::path::MAIN_SEPARATOR;
        return full_path_str.contains(&format!("{sep}{dir_name}{sep}"))
            || full_path_str.starts_with(&format!("{dir_name}{sep}"));
    }

    // Pattern like `*.rs` — match file extension
    if pattern.starts_with("*.") {
        let ext = &pattern[1..]; // e.g., ".rs"
        return file_name.ends_with(ext);
    }

    // Exact name match
    file_name == pattern
}
