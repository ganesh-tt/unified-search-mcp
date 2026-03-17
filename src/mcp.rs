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
    #[tool(description = "Search across all enabled sources in parallel. Returns a ranked Markdown table.")]
    async fn unified_search(
        &self,
        Parameters(params): Parameters<UnifiedSearchParams>,
    ) -> String {
        self.server
            .handle_unified_search(params.query, params.sources, params.max_results)
            .await
    }

    /// Search a single named source.
    /// Returns results as a JSON array.
    #[tool(description = "Search a single named source. Returns results as a JSON array.")]
    async fn search_source(
        &self,
        Parameters(params): Parameters<SearchSourceParams>,
    ) -> String {
        self.server
            .handle_search_source(params.source, params.query, params.max_results)
            .await
    }

    /// List all configured sources with their health status.
    #[tool(description = "List all configured sources with their health status.")]
    async fn list_sources(&self) -> String {
        self.server.handle_list_sources().await
    }

    /// Index local files for vector search (not yet available).
    #[tool(description = "Index local files for vector search (not yet available).")]
    async fn index_local(&self) -> String {
        self.server.handle_index_local().await
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Unified search across Slack, Confluence, JIRA, and local files. \
                 Use unified_search for cross-source queries or search_source for \
                 a single source.",
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
