use std::collections::HashMap;
use std::env;

use unified_search_mcp::config;
use unified_search_mcp::core::{OrchestratorConfig, SearchOrchestrator};
use unified_search_mcp::server::UnifiedSearchServer;
use unified_search_mcp::sources::confluence::ConfluenceSource;
use unified_search_mcp::sources::jira::JiraSource;
use unified_search_mcp::sources::local_text::LocalTextSource;
use unified_search_mcp::sources::slack::SlackSource;
use unified_search_mcp::sources::SearchSource;

fn main() {
    // Determine config path: CLI arg or default
    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "./config.yaml".to_string());

    println!("unified-search-mcp v0.1.0");

    // Attempt to load config
    let app_config = match config::load(&config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Warning: Could not load config from '{}': {}", config_path, e);
            eprintln!("Starting with no sources configured. Create a config.yaml to enable sources.");
            eprintln!("See config.example.yaml for a template.");

            // Build a server with no sources
            let orchestrator = SearchOrchestrator::new(vec![], OrchestratorConfig::default());
            let _server = UnifiedSearchServer::new(orchestrator);
            println!("Server ready with 0 sources (no config loaded).");
            println!("MCP stdio transport not yet wired — exiting.");
            return;
        }
    };

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

    let orchestrator_config = OrchestratorConfig {
        timeout_seconds: app_config.server.timeout_seconds,
        source_weights,
        max_results: app_config.server.max_results,
    };

    let orchestrator = SearchOrchestrator::new(sources, orchestrator_config);
    let _server = UnifiedSearchServer::new(orchestrator);

    println!(
        "Server ready with {} source(s): {}",
        source_count,
        app_config.server.name,
    );
    println!("MCP stdio transport not yet wired — exiting.");
}
