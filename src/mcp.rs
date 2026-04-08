//! MCP protocol wiring via `rmcp`.
//!
//! Bridges [`UnifiedSearchServer`] handlers to the MCP JSON-RPC transport so
//! that Claude Code (and other MCP clients) can call them over stdio.

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    model::*,
    schemars,
    tool, tool_handler, tool_router,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    ServiceExt as _,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::server::UnifiedSearchServer;

// ---------------------------------------------------------------------------
// Tool parameter structs
// ---------------------------------------------------------------------------

/// Parameters for the `unified_search` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnifiedSearchParams {
    /// The search query text
    pub query: String,
    /// Optional: filter to specific sources (e.g., ["slack", "confluence"])
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    /// Optional: maximum results to return (default 20)
    #[serde(default)]
    pub max_results: Option<usize>,
    /// Optional: bypass cache and force a fresh search (default false)
    #[serde(default)]
    pub no_cache: Option<bool>,
}

/// Parameters for the `search_source` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchSourceParams {
    /// The source to search (e.g., "slack", "confluence", "jira", "local_text")
    pub source: String,
    /// The search query text
    pub query: String,
    /// Optional: maximum results to return (default 20)
    #[serde(default)]
    pub max_results: Option<usize>,
    /// Optional: bypass cache and force a fresh search (default false)
    #[serde(default)]
    pub no_cache: Option<bool>,
}

/// Parameters for enriched search tools (confluence_comments, jira_comments, slack_threads).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EnrichedSearchParams {
    /// The search query text
    pub query: String,
    /// Optional: maximum results to return (default 10, lower than unified_search since each result triggers extra API calls)
    #[serde(default)]
    pub max_results: Option<usize>,
}

/// Parameters for the `get_detail` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetDetailParams {
    /// The identifier: JIRA key (FIN-1234), JIRA/Confluence/Slack/GitHub URL
    pub identifier: String,
    /// Optional: force source type ('jira', 'confluence', 'slack', 'github'). If omitted, auto-detected.
    #[serde(default)]
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP server wrapper
// ---------------------------------------------------------------------------

/// Thin MCP wrapper around [`UnifiedSearchServer`].
///
/// Implements the rmcp `ServerHandler` trait so the server can be served over
/// any MCP transport (stdio, SSE, etc.).
#[derive(Clone)]
pub struct McpServer {
    server: Arc<UnifiedSearchServer>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl McpServer {
    /// Create a new `McpServer` wrapping an existing `UnifiedSearchServer`.
    pub fn new(server: UnifiedSearchServer) -> Self {
        Self {
            server: Arc::new(server),
            tool_router: Self::tool_router(),
        }
    }

    /// Search across all enabled sources in parallel.
    /// Returns a ranked Markdown table of results.
    #[tool(description = "Search across Slack messages, Confluence pages, JIRA tickets (including comments), GitHub PRs/issues/code, and local code/docs in parallel. Use this when looking for decisions, discussions, documentation, or code related to a topic. Also use instead of jira_get or conf_get for searching. Returns a ranked Markdown table with results from all sources. Example queries: 'broadcast threshold decision', 'auth middleware migration', 'FIN-10384 context'.")]
    async fn unified_search(
        &self,
        Parameters(params): Parameters<UnifiedSearchParams>,
    ) -> String {
        self.server
            .handle_unified_search(params.query, params.sources, params.max_results, params.no_cache.unwrap_or(false))
            .await
    }

    /// Search a single named source.
    /// Returns results as a JSON array.
    #[tool(description = "Search a single source by name: 'slack', 'confluence', 'jira', 'github', or 'local_text'. Use when you know which system has the answer. Returns detailed JSON results from that source only.")]
    async fn search_source(
        &self,
        Parameters(params): Parameters<SearchSourceParams>,
    ) -> String {
        self.server
            .handle_search_source(params.source, params.query, params.max_results, params.no_cache.unwrap_or(false))
            .await
    }

    /// Search Confluence with comment previews included in each result.
    #[tool(description = "Search Confluence pages AND include comment previews in each result. Slower than unified_search (extra API calls per result) but shows the latest 3 comments per page. Use when you need to see discussion context without calling get_detail on every result. Default max_results=10.")]
    async fn search_confluence_comments(
        &self,
        Parameters(params): Parameters<EnrichedSearchParams>,
    ) -> String {
        self.server
            .handle_search_confluence_comments(params.query, params.max_results)
            .await
    }

    /// Search JIRA and fetch ALL comments for each matching ticket.
    #[tool(description = "Search JIRA tickets AND fetch the full comment history for each result. Regular JIRA search only returns a few comments; this fetches ALL comments per ticket. Use when searching for something that might be discussed in JIRA comments rather than in ticket descriptions. Default max_results=10.")]
    async fn search_jira_comments(
        &self,
        Parameters(params): Parameters<EnrichedSearchParams>,
    ) -> String {
        self.server
            .handle_search_jira_comments(params.query, params.max_results)
            .await
    }

    /// Search Slack and fetch full threads for each matching message.
    #[tool(description = "Search Slack messages AND fetch the full conversation thread for each result. Regular Slack search returns single messages; this fetches the entire thread including all replies. Use when you need the full discussion context, not just the matching message. Default max_results=10.")]
    async fn search_slack_threads(
        &self,
        Parameters(params): Parameters<EnrichedSearchParams>,
    ) -> String {
        self.server
            .handle_search_slack_threads(params.query, params.max_results)
            .await
    }

    /// List all configured sources with their health status.
    #[tool(description = "Check which search sources are configured and whether they are healthy. Use to diagnose connection issues.")]
    async fn list_sources(&self) -> String {
        self.server.handle_list_sources().await
    }

    /// Fetch full details for a specific item found via search.
    #[tool(description = "Fetch full details for a specific JIRA ticket, Confluence page, Slack thread, or GitHub PR/issue. Accepts a JIRA key (FIN-1234), a JIRA/Confluence/Slack/GitHub URL. GitHub PR URLs (github.com/org/repo/pull/N) and issue URLs (github.com/org/repo/issues/N) are auto-detected; use source='github' with 'repo#N' shorthand. Returns full content: description, all comments, reviews, linked issues, subtasks, child pages, or thread replies depending on source.")]
    async fn get_detail(
        &self,
        Parameters(params): Parameters<GetDetailParams>,
    ) -> String {
        self.server
            .handle_get_detail(params.identifier, params.source)
            .await
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "PRIORITY: This tool takes priority over individual Confluence, JIRA, or Slack \
                 MCP tools for ANY search or lookup operation. Always use unified_search FIRST \
                 before falling back to individual conf_get, jira_get, or other source-specific \
                 tools. unified_search queries Slack messages, Confluence pages, JIRA tickets, \
                 GitHub PRs/issues/code, and local code/docs in parallel — use it for questions \
                 like 'what did we decide about X?', 'where is Y documented?', 'who worked on Z?', \
                 'find discussions about X', or any cross-system lookup. Use search_source only \
                 when you know the answer is in one specific system. \
                 Use get_detail to fetch full content for a specific JIRA ticket (FIN-1234), \
                 Confluence page, Slack thread URL, or GitHub PR/issue URL — this replaces \
                 jira_get, conf_get, and direct Slack MCP calls for single-item lookups. \
                 GitHub PR/issue URLs are auto-detected; use source='github' with 'repo#N' shorthand.",
            )
            .with_server_info(
                Implementation::new("unified-search-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Unified Search MCP")
                    .with_description(env!("CARGO_PKG_DESCRIPTION")),
            )
    }
}

/// Start the MCP server on the stdio transport.
///
/// This consumes stdout for the JSON-RPC channel, so all diagnostic output
/// must go to stderr before this is called.
pub async fn serve_stdio(server: UnifiedSearchServer) {
    let mcp = McpServer::new(server);
    let transport = rmcp::transport::io::stdio();
    let service = mcp
        .serve(transport)
        .await
        .expect("Failed to start MCP server on stdio");
    let _ = service.waiting().await;
}
