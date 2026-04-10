use std::collections::HashMap;
use std::env;

use unified_search_mcp::config;
use unified_search_mcp::core::{OrchestratorConfig, SearchOrchestrator};
use unified_search_mcp::mcp;
use unified_search_mcp::server::UnifiedSearchServer;
use unified_search_mcp::sources::confluence::ConfluenceSource;
use unified_search_mcp::sources::github::GitHubSource;
use unified_search_mcp::sources::jira::JiraSource;
use unified_search_mcp::sources::local_text::LocalTextSource;
use unified_search_mcp::sources::slack::SlackSource;
use unified_search_mcp::sources::SearchSource;

#[tokio::main]
async fn main() {
    // Init tracing to stderr (stdout = MCP JSON-RPC channel).
    // Default: warn-level; override with RUST_LOG env var.
    // Example: RUST_LOG=unified_search_mcp=info for get_detail timing logs.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_target(false)
        .compact()
        .init();

    let args: Vec<String> = env::args().collect();
    let verify = args.iter().any(|a| a == "--verify");

    let help = args.iter().any(|a| a == "--help" || a == "-h");
    if help {
        eprintln!("unified-search-mcp v{}", env!("CARGO_PKG_VERSION"));
        eprintln!();
        eprintln!(
            "A unified MCP search server for Slack, Confluence, JIRA, GitHub, and local files."
        );
        eprintln!();
        eprintln!("USAGE:");
        eprintln!("  unified-search-mcp [OPTIONS]");
        eprintln!();
        eprintln!("OPTIONS:");
        eprintln!("  --config <PATH>    Config file path (default: config.yaml)");
        eprintln!("  --verify           Run preflight checks and exit");
        eprintln!("  --stats            Show adoption report and exit");
        eprintln!("  --days <N>         Days to include in stats (default: 7, use with --stats)");
        eprintln!("  --help, -h         Show this help message");
        eprintln!();
        eprintln!("Without flags, starts the MCP server on stdio.");
        std::process::exit(0);
    }

    let stats = args.iter().any(|a| a == "--stats");
    let stats_days = args
        .iter()
        .position(|a| a == "--days")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(7);

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
            if stats {
                // Fall back to default metrics path when config fails to load
                unified_search_mcp::stats::run_stats("~/.unified-search/metrics.jsonl", stats_days);
                return;
            }
            eprintln!(
                "Warning: Could not load config from '{}': {}",
                config_path, e
            );
            eprintln!(
                "Starting with no sources configured. Create a config.yaml to enable sources."
            );
            eprintln!("See config.example.yaml for a template.");

            // Build a server with no sources and serve via MCP
            let orchestrator = SearchOrchestrator::new(vec![], OrchestratorConfig::default(), 0);
            let server = UnifiedSearchServer::new(orchestrator, None, None, None, None, None);
            eprintln!(
                "unified-search-mcp v{} -- 0 source(s) ready (no config loaded)",
                env!("CARGO_PKG_VERSION")
            );
            mcp::serve_stdio(server).await;
            return;
        }
    };

    // Run stats if --stats was passed (after config is loaded so we use configured metrics_path)
    if stats {
        let metrics_path = shellexpand::tilde(&app_config.server.metrics_path).to_string();
        unified_search_mcp::stats::run_stats(&metrics_path, stats_days);
        return;
    }

    // Run preflight verification if --verify was passed
    if verify {
        let ok = unified_search_mcp::verify::verify(&app_config, config_path).await;
        std::process::exit(if ok { 0 } else { 1 });
    }

    // Build sources from config. HTTP clients are built once per source type
    // and shared between the orchestrator (search) and detail (get_detail) paths
    // to halve connection pool overhead and maximize HTTP keep-alive reuse.
    let mut sources: Vec<Box<dyn SearchSource>> = Vec::new();
    let mut source_weights: HashMap<String, f32> = HashMap::new();

    // Build shared clients + detail instances
    let mut slack_detail: Option<SlackSource> = None;
    let mut confluence_detail: Option<ConfluenceSource> = None;
    let mut jira_detail: Option<JiraSource> = None;

    if let Some(ref slack_cfg) = app_config.sources.slack {
        if slack_cfg.enabled {
            let client = SlackSource::build_client();
            source_weights.insert("slack".to_string(), slack_cfg.weight);
            sources.push(Box::new(SlackSource::new_with_client(
                slack_cfg.config.clone(),
                client.clone(),
            )));
            slack_detail = Some(SlackSource::new_with_client(
                slack_cfg.config.clone(),
                client,
            ));
        }
    }

    if let Some(ref confluence_cfg) = app_config.sources.confluence {
        if confluence_cfg.enabled {
            let client = ConfluenceSource::build_client();
            source_weights.insert("confluence".to_string(), confluence_cfg.weight);
            sources.push(Box::new(ConfluenceSource::new_with_client(
                confluence_cfg.config.clone(),
                client.clone(),
            )));
            confluence_detail = Some(ConfluenceSource::new_with_client(
                confluence_cfg.config.clone(),
                client,
            ));
        }
    }

    if let Some(ref jira_cfg) = app_config.sources.jira {
        if jira_cfg.enabled {
            let client = JiraSource::build_client();
            source_weights.insert("jira".to_string(), jira_cfg.weight);
            sources.push(Box::new(JiraSource::new_with_client(
                jira_cfg.config.clone(),
                client.clone(),
            )));
            jira_detail = Some(JiraSource::new_with_client(jira_cfg.config.clone(), client));
        }
    }

    if let Some(ref local_cfg) = app_config.sources.local_text {
        if local_cfg.enabled {
            source_weights.insert("local_text".to_string(), local_cfg.weight);
            sources.push(Box::new(LocalTextSource::new(local_cfg.config.clone())));
        }
    }

    if let Some(ref github_cfg) = app_config.sources.github {
        if github_cfg.enabled {
            source_weights.insert("github".to_string(), github_cfg.weight);
            sources.push(Box::new(GitHubSource::new(github_cfg.config.clone())));
        }
    }

    let source_count = sources.len();

    // GitHub uses CLI subprocess (no HTTP client to share)
    let github_detail = app_config
        .sources
        .github
        .as_ref()
        .filter(|c| c.enabled)
        .map(|c| GitHubSource::new(c.config.clone()));

    let orchestrator_config = OrchestratorConfig {
        timeout_seconds: app_config.server.timeout_seconds,
        source_weights,
        max_results: app_config.server.max_results,
    };

    let metrics_path = shellexpand::tilde(&app_config.server.metrics_path).to_string();
    let metrics =
        unified_search_mcp::metrics::MetricsLogger::new(std::path::PathBuf::from(metrics_path));

    let orchestrator = SearchOrchestrator::new(
        sources,
        orchestrator_config,
        app_config.server.cache_ttl_seconds,
    );
    let server = UnifiedSearchServer::new(
        orchestrator,
        jira_detail,
        confluence_detail,
        slack_detail,
        github_detail,
        Some(metrics),
    );

    // Stdout is now the MCP JSON-RPC channel -- all diagnostics go to stderr
    eprintln!(
        "unified-search-mcp v{} -- {} source(s) ready: {}",
        env!("CARGO_PKG_VERSION"),
        source_count,
        app_config.server.name,
    );

    mcp::serve_stdio(server).await;
}
