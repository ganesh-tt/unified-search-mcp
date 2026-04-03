use std::path::PathBuf;
use chrono::{DateTime, Utc, Duration};

pub fn run_stats(metrics_path: &str, days: u64) {
    println!("Note: --stats scans ~/.claude/projects/ conversation logs to detect bypass tool calls.");
    println!("No conversation content is read — only tool call names are counted.\n");

    let cutoff = Utc::now() - Duration::days(days as i64);
    let path = PathBuf::from(shellexpand::tilde(metrics_path).to_string());

    println!("=== Unified Search Adoption Report (last {} days) ===\n", days);

    let entries = read_metrics(&path, &cutoff);
    if entries.is_empty() {
        println!("No metrics found in {}", path.display());
        println!("Metrics are recorded automatically when the MCP server handles tool calls.");
        return;
    }

    // Categorize
    let mut search_calls: Vec<&serde_json::Value> = Vec::new();
    let mut source_calls: Vec<&serde_json::Value> = Vec::new();
    let mut detail_calls: Vec<&serde_json::Value> = Vec::new();

    for entry in &entries {
        match entry.get("tool").and_then(|v| v.as_str()) {
            Some("unified_search") => search_calls.push(entry),
            Some("search_source") => source_calls.push(entry),
            Some("get_detail") => detail_calls.push(entry),
            _ => {}
        }
    }

    println!("Tool Calls:");
    report_tool_stats("  unified_search", &search_calls);
    report_tool_stats("  search_source", &source_calls);
    report_tool_stats("  get_detail", &detail_calls);

    // Bypasses
    println!("\nBypasses (Claude used individual MCPs for search/read):");
    let bypass_counts = scan_claude_code_logs(&cutoff);
    if bypass_counts.is_empty() {
        println!("  (Claude Code logs not found or no bypasses detected)");
    } else {
        for (tool, count) in &bypass_counts {
            println!("  {}: {} calls", tool, count);
        }
    }

    // Adoption rate
    let total_unified = search_calls.len() + source_calls.len() + detail_calls.len();
    let total_bypasses: usize = bypass_counts.values().sum();
    let total = total_unified + total_bypasses;
    if total > 0 {
        let rate = (total_unified as f64 / total as f64) * 100.0;
        println!(
            "\nAdoption Rate: {:.0}% ({} unified / {} total search-like operations)",
            rate, total_unified, total
        );
    }

    println!();
}

fn read_metrics(path: &PathBuf, cutoff: &DateTime<Utc>) -> Vec<serde_json::Value> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    content
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| {
            entry.get("ts")
                .and_then(|ts| ts.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc) >= *cutoff)
                .unwrap_or(false)
        })
        .collect()
}

fn report_tool_stats(label: &str, entries: &[&serde_json::Value]) {
    if entries.is_empty() {
        println!("{}:  0 calls", label);
        return;
    }

    let latencies: Vec<u64> = entries
        .iter()
        .filter_map(|e| e.get("total_ms").or(e.get("latency_ms")))
        .filter_map(|v| v.as_u64())
        .collect();

    if latencies.is_empty() {
        println!("{}:  {} calls", label, entries.len());
        return;
    }

    let mut sorted = latencies.clone();
    sorted.sort();
    let avg = sorted.iter().sum::<u64>() / sorted.len() as u64;
    let p50 = sorted[sorted.len() / 2];
    let p95_idx = (sorted.len() as f64 * 0.95).ceil() as usize;
    let p95 = sorted[p95_idx.min(sorted.len()) - 1];

    println!(
        "{}:  {} calls  (avg {}ms, p50 {}ms, p95 {}ms)",
        label, entries.len(), avg, p50, p95
    );
}

fn scan_claude_code_logs(cutoff: &DateTime<Utc>) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    let base = shellexpand::tilde("~/.claude/projects").to_string();
    let base_path = PathBuf::from(&base);

    if !base_path.exists() {
        return counts;
    }

    let bypass_tools = ["jira_get", "mcp__jira__jira_get", "conf_get"];

    if let Ok(entries) = glob_conversation_files(&base_path) {
        for file_path in entries {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                for line in content.lines() {
                    if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                        let in_range = entry.get("timestamp").or(entry.get("ts"))
                            .and_then(|ts| ts.as_str())
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&Utc) >= *cutoff)
                            .unwrap_or(true);

                        if !in_range { continue; }

                        if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
                            for bypass in &bypass_tools {
                                if name.contains(bypass) {
                                    *counts.entry(name.to_string()).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    counts
}

fn glob_conversation_files(base: &PathBuf) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let convos = path.join("conversations");
                if convos.exists() {
                    if let Ok(conv_entries) = std::fs::read_dir(&convos) {
                        for conv in conv_entries.flatten() {
                            let conv_path = conv.path();
                            if conv_path.extension().map_or(false, |e| e == "jsonl") {
                                files.push(conv_path);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(files)
}
