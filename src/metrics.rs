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

    /// Log a metrics entry. Fire-and-forget via tokio::spawn.
    pub async fn log(&self, entry: MetricsEntry) {
        let path = self.path.clone();
        tokio::spawn(async move {
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
        obj.insert("ts".to_string(), serde_json::Value::String(Utc::now().to_rfc3339()));
    }

    let line = serde_json::to_string(&json_value).unwrap_or_default();

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    writeln!(file, "{}", line)?;
    Ok(())
}
