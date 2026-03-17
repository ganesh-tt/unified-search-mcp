use std::path::Path;

use crate::config::AppConfig;
use crate::models::HealthStatus;
use crate::sources::confluence::ConfluenceSource;
use crate::sources::jira::JiraSource;
use crate::sources::local_text::LocalTextSource;
use crate::sources::slack::SlackSource;
use crate::sources::SearchSource;

/// Run preflight checks on all configured sources.
///
/// Prints human-readable status lines with `[OK]`, `[WARN]`, or `[FAIL]` prefixes.
/// Returns `true` if no critical failures were detected, `false` otherwise.
pub async fn verify(config: &AppConfig, config_path: &str) -> bool {
    let mut fail_count: usize = 0;
    let mut healthy_count: usize = 0;
    let mut source_count: usize = 0;

    println!("unified-search-mcp v0.1.0 — preflight check\n");

    // Count enabled sources
    let enabled = count_enabled(config);
    println!("[OK]  Config loaded from {} ({} sources enabled)", config_path, enabled);

    // --- Slack ---
    if let Some(ref slack_cfg) = config.sources.slack {
        if slack_cfg.enabled {
            source_count += 1;

            // Token type check
            if !slack_cfg.config.user_token.starts_with("xoxp-") {
                println!("[WARN] Slack: token does not start with xoxp- (user token)");
                println!("       Hint: search.messages requires a user token (xoxp-...), not a bot token (xoxb-...)");
            }

            let source = SlackSource::new(slack_cfg.config.clone());
            let health = source.health_check().await;
            match health.status {
                HealthStatus::Healthy => {
                    let detail = health.message.unwrap_or_else(|| "OK".to_string());
                    let latency = format_latency(health.latency_ms);
                    println!("[OK]  Slack: {} {}", detail, latency);
                    healthy_count += 1;
                }
                HealthStatus::Degraded => {
                    let detail = health.message.unwrap_or_else(|| "degraded".to_string());
                    println!("[WARN] Slack: {}", detail);
                    healthy_count += 1; // degraded counts as non-fatal
                }
                HealthStatus::Unavailable => {
                    let detail = health.message.unwrap_or_else(|| "unavailable".to_string());
                    println!("[FAIL] Slack: {}", detail);
                    println!("       Fix: check your user_token and network connectivity");
                    fail_count += 1;
                }
            }
        }
    }

    // --- Confluence ---
    if let Some(ref confluence_cfg) = config.sources.confluence {
        if confluence_cfg.enabled {
            source_count += 1;
            let source = ConfluenceSource::new(confluence_cfg.config.clone());
            let health = source.health_check().await;
            match health.status {
                HealthStatus::Healthy => {
                    let detail = health.message.unwrap_or_else(|| "OK".to_string());
                    let latency = format_latency(health.latency_ms);
                    println!("[OK]  Confluence: {} {}", detail, latency);
                    healthy_count += 1;
                }
                HealthStatus::Degraded => {
                    let detail = health.message.unwrap_or_else(|| "degraded".to_string());
                    println!("[WARN] Confluence: {}", detail);
                    healthy_count += 1;
                }
                HealthStatus::Unavailable => {
                    let detail = health.message.unwrap_or_else(|| "unavailable".to_string());
                    println!("[FAIL] Confluence: {}", detail);
                    println!("       Fix: check base_url, email, and api_token");
                    fail_count += 1;
                }
            }
        }
    }

    // --- JIRA ---
    if let Some(ref jira_cfg) = config.sources.jira {
        if jira_cfg.enabled {
            source_count += 1;
            let source = JiraSource::new(jira_cfg.config.clone());
            let health = source.health_check().await;
            match health.status {
                HealthStatus::Healthy => {
                    let detail = health.message.unwrap_or_else(|| "OK".to_string());
                    let latency = format_latency(health.latency_ms);
                    println!("[OK]  JIRA: {} {}", detail, latency);
                    healthy_count += 1;
                }
                HealthStatus::Degraded => {
                    let detail = health.message.unwrap_or_else(|| "degraded".to_string());
                    println!("[WARN] JIRA: {}", detail);
                    healthy_count += 1;
                }
                HealthStatus::Unavailable => {
                    let detail = health.message.unwrap_or_else(|| "unavailable".to_string());
                    println!("[FAIL] JIRA: {}", detail);
                    println!("       Fix: check base_url, email, and api_token");
                    fail_count += 1;
                }
            }
        }
    }

    // --- Local Text ---
    if let Some(ref local_cfg) = config.sources.local_text {
        if local_cfg.enabled {
            source_count += 1;
            let source = LocalTextSource::new(local_cfg.config.clone());
            let health = source.health_check().await;

            match health.status {
                HealthStatus::Healthy => {
                    println!("[OK]  Local text: paths accessible");
                    healthy_count += 1;
                }
                HealthStatus::Degraded => {
                    let detail = health.message.unwrap_or_else(|| "degraded".to_string());
                    println!("[WARN] Local text: {}", detail);
                    healthy_count += 1;
                }
                HealthStatus::Unavailable => {
                    let detail = health.message.unwrap_or_else(|| "unavailable".to_string());
                    println!("[FAIL] Local text: {}", detail);
                    println!("       Fix: ensure at least one configured path exists");
                    fail_count += 1;
                }
            }

            // Detailed per-path check
            for path in &local_cfg.config.paths {
                if path.exists() && path.is_dir() {
                    let file_count = count_matching_files(path, &local_cfg.config.include_patterns);
                    println!("       {} — directory, {} matching files", path.display(), file_count);
                } else if path.exists() {
                    println!("       {} — exists (not a directory)", path.display());
                } else {
                    println!("[WARN] {} — path does not exist", path.display());
                }
            }
        }
    }

    // --- Ripgrep check ---
    check_ripgrep();

    // --- Summary ---
    println!();
    if fail_count == 0 {
        println!("Ready! {} sources configured, {} healthy.", source_count, healthy_count);
    } else {
        println!(
            "{} source(s) failed out of {} configured. Fix the [FAIL] items above.",
            fail_count, source_count
        );
    }

    fail_count == 0
}

/// Count how many sources are enabled in the config.
fn count_enabled(config: &AppConfig) -> usize {
    let mut n = 0;
    if config.sources.slack.as_ref().map_or(false, |s| s.enabled) {
        n += 1;
    }
    if config.sources.confluence.as_ref().map_or(false, |s| s.enabled) {
        n += 1;
    }
    if config.sources.jira.as_ref().map_or(false, |s| s.enabled) {
        n += 1;
    }
    if config.sources.local_text.as_ref().map_or(false, |s| s.enabled) {
        n += 1;
    }
    n
}

/// Format latency for display.
fn format_latency(ms: Option<u64>) -> String {
    match ms {
        Some(ms) => format!("({}ms)", ms),
        None => String::new(),
    }
}

/// Count files in a directory that match the include patterns.
/// If no include patterns are specified, counts all files.
fn count_matching_files(dir: &Path, include_patterns: &[String]) -> usize {
    let walker = walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file());

    if include_patterns.is_empty() {
        return walker.count();
    }

    walker
        .filter(|entry| {
            let name = entry
                .file_name()
                .to_str()
                .unwrap_or("");
            include_patterns.iter().any(|pat| {
                if pat.starts_with("*.") {
                    let ext = &pat[1..]; // e.g., ".rs"
                    name.ends_with(ext)
                } else {
                    name == pat
                }
            })
        })
        .count()
}

/// Check if ripgrep (rg) is available in PATH.
fn check_ripgrep() {
    match std::process::Command::new("rg").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let first_line = version.lines().next().unwrap_or("unknown version");
            println!("[OK]  ripgrep: {}", first_line);
        }
        Ok(_) => {
            println!("[WARN] ripgrep: rg found but returned non-zero exit code");
            println!("       Local text search will fall back to built-in grep");
        }
        Err(_) => {
            println!("[WARN] ripgrep: rg not found in PATH");
            println!("       Local text search will fall back to built-in grep (slower)");
            println!("       Install: brew install ripgrep / cargo install ripgrep");
        }
    }
}
