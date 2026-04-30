use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;

/// A single metrics entry. Tagged enum serialized to JSON.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MetricsEntry {
    Search {
        tool: String,
        query: String,
        sources_queried: Vec<String>,
        total_results: usize,
        deduped_results: usize,
        total_ms: u64,
    },
    Detail {
        tool: String,
        identifier: String,
        detected_source: String,
        explicit_source: Option<String>,
        latency_ms: u64,
        comments_returned: usize,
        error: Option<String>,
    },
}

/// Append-only JSONL metrics logger.
#[derive(Clone)]
pub struct MetricsLogger {
    path: PathBuf,
}

impl MetricsLogger {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Log a metrics entry. Fire-and-forget: the write runs on the blocking
    /// pool but the caller never awaits its completion. Keeps the MCP tool
    /// handler off the critical path of file I/O — if APFS, iCloud sync, or
    /// the blocking pool stalls, the tool call still returns on time.
    pub async fn log(&self, entry: MetricsEntry) {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = write_entry(&path, &entry) {
                eprintln!("metrics: failed to write: {}", e);
            }
        });
    }
}

fn write_entry(path: &PathBuf, entry: &MetricsEntry) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // File rotation at 10MB
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() > 10 * 1024 * 1024 {
            let backup = path.with_extension("jsonl.1");
            let _ = std::fs::rename(path, backup);
        }
    }

    // Add timestamp to entry
    let mut json_value = serde_json::to_value(entry).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = json_value.as_object_mut() {
        obj.insert(
            "ts".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );
        // Truncate query text for privacy
        if let Some(q) = obj.get_mut("query") {
            if let Some(s) = q.as_str() {
                if s.len() > 100 {
                    *q = serde_json::Value::String(format!(
                        "{}...",
                        s.chars().take(100).collect::<String>()
                    ));
                }
            }
        }
        if let Some(id) = obj.get_mut("identifier") {
            if let Some(s) = id.as_str() {
                if s.len() > 100 {
                    *id = serde_json::Value::String(format!(
                        "{}...",
                        s.chars().take(100).collect::<String>()
                    ));
                }
            }
        }
    }

    let mut line = serde_json::to_string(&json_value).unwrap_or_default();
    line.push('\n');

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    // Single write_all syscall keeps the line + trailing newline atomic against
    // concurrent appenders (relied on for cross-MCP-process safety since
    // multiple Claude sessions append to the same metrics.jsonl). With
    // O_APPEND set, POSIX guarantees this is atomic for writes ≤ PIPE_BUF.
    file.write_all(line.as_bytes())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}
