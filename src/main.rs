use std::collections::HashMap;
use std::env;

use unified_search_mcp::config;
use unified_search_mcp::core::{OrchestratorConfig, SearchOrchestrator};
use unified_search_mcp::mcp;
use unified_search_mcp::server::UnifiedSearchServer;
use unified_search_mcp::sources::confluence::ConfluenceSource;
use unified_search_mcp::sources::jira::JiraSource;
use unified_search_mcp::sources::local_text::LocalTextSource;
use unified_search_mcp::sources::slack::SlackSource;
use unified_search_mcp::sources::SearchSource;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let verify = args.iter().any(|a| a == "--verify");
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("config.yaml");

    // Attempt to load config
    let app_config = match config::load(config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            if verify {
                eprintln!("[FAIL] Could not load config from '{}': {}", config_path, e);
                std::process::exit(1);
            }
            eprintln!("Warning: Could not load config from '{}': {}", config_path, e);
            eprintln!("Starting with no sources configured. Create a config.yaml to enable sources.");
            eprintln!("See config.example.yaml for a template.");

            // Build a server with no sources and serve via MCP
            let orchestrator = SearchOrchestrator::new(vec![], OrchestratorConfig::default());
            let server = UnifiedSearchServer::new(orchestrator, None, None, None, None);
            eprintln!("unified-search-mcp v0.1.0 -- 0 source(s) ready (no config loaded)");
            mcp::serve_stdio(server).await;
            return;
        }
    };

    // Run preflight verification if --verify was passed
    if verify {
        let ok = unified_search_mcp::verify::verify(&app_config, config_path).await;
        std::process::exit(if ok { 0 } else { 1 });
    }

    // Build sources from config
    let mut sources: Vec<Box<dyn SearchSource>> = Vec::new();
    let mut source_weights: HashMap<String, f32> = HashMap::new();

    if let Some(ref slack_cfg) = app_config.sources.slack {
        if slack_cfg.enabled {
            source_weights.insert("slack".to_string(), slack_cfg.weight);
            sources.push(Box::new(SlackSource::new(slack_cfg.config.clone())));
        }
    }

    if let Some(ref confluence_cfg) = app_config.sources.confluence {
        if confluence_cfg.enabled {
            source_weights.insert("confluence".to_string(), confluence_cfg.weight);
            sources.push(Box::new(ConfluenceSource::new(confluence_cfg.config.clone())));
        }
    }

    if let Some(ref jira_cfg) = app_config.sources.jira {
        if jira_cfg.enabled {
            source_weights.insert("jira".to_string(), jira_cfg.weight);
            sources.push(Box::new(JiraSource::new(jira_cfg.config.clone())));
        }
    }

    if let Some(ref local_cfg) = app_config.sources.local_text {
        if local_cfg.enabled {
            source_weights.insert("local_text".to_string(), local_cfg.weight);
            sources.push(Box::new(LocalTextSource::new(local_cfg.config.clone())));
        }
    }

    let source_count = sources.len();

    // Build per-source instances for get_detail lookups
    let jira_detail = app_config
        .sources
        .jira
        .as_ref()
        .filter(|c| c.enabled)
        .map(|c| JiraSource::new(c.config.clone()));
    let confluence_detail = app_config
        .sources
        .confluence
        .as_ref()
        .filter(|c| c.enabled)
        .map(|c| ConfluenceSource::new(c.config.clone()));
    let slack_detail = app_config
        .sources
        .slack
        .as_ref()
        .filter(|c| c.enabled)
        .map(|c| SlackSource::new(c.config.clone()));

    let orchestrator_config = OrchestratorConfig {
        timeout_seconds: app_config.server.timeout_seconds,
        source_weights,
        max_results: app_config.server.max_results,
    };

    let metrics_path = shellexpand::tilde("~/.unified-search/metrics.jsonl").to_string();
    let metrics = unified_search_mcp::metrics::MetricsLogger::new(std::path::PathBuf::from(metrics_path));

    let orchestrator = SearchOrchestrator::new(sources, orchestrator_config);
    let server = UnifiedSearchServer::new(orchestrator, jira_detail, confluence_detail, slack_detail, Some(metrics));

    // Stdout is now the MCP JSON-RPC channel -- all diagnostics go to stderr
    eprintln!(
        "unified-search-mcp v0.1.0 -- {} source(s) ready: {}",
        source_count, app_config.server.name,
    );

    mcp::serve_stdio(server).await;
}
