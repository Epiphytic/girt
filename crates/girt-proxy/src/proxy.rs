use std::future::Future;

use rmcp::{
    ErrorData as McpError, Peer, RoleClient, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, CompleteRequestParams, CompleteResult,
        GetPromptRequestParams, GetPromptResult, InitializeRequestParams, InitializeResult,
        ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, ServerCapabilities,
        ServerInfo,
    },
    service::RequestContext,
};

/// MCP proxy that forwards all requests from the agent to Wassette.
///
/// Phase 0: Pure pass-through. No interception, no decision gates.
/// Future phases will add Creation Gate and Execution Gate here.
pub struct GirtProxy {
    /// The Wassette MCP client peer — used to forward requests
    wassette: Peer<RoleClient>,
    /// Capabilities reported by Wassette (cached after connection)
    wassette_capabilities: ServerCapabilities,
}

impl GirtProxy {
    pub fn new(wassette: Peer<RoleClient>, wassette_init: InitializeResult) -> Self {
        Self {
            wassette,
            wassette_capabilities: wassette_init.capabilities,
        }
    }
}

/// Return GIRT server info, forwarding Wassette's capabilities.
fn girt_info(capabilities: &ServerCapabilities) -> InitializeResult {
    InitializeResult {
        protocol_version: Default::default(),
        capabilities: capabilities.clone(),
        server_info: rmcp::model::Implementation::from_build_env(),
        instructions: Some(
            "GIRT MCP Proxy — Generative Isolated Runtime for Tools".into(),
        ),
    }
}

impl ServerHandler for GirtProxy {
    fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        async { Ok(girt_info(&self.wassette_capabilities)) }
    }

    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying list_tools");
            self.wassette
                .list_tools(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            tracing::info!(tool = %request.name, "Proxying call_tool");
            self.wassette
                .call_tool(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying list_resources");
            self.wassette
                .list_resources(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying read_resource");
            self.wassette
                .read_resource(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying list_prompts");
            self.wassette
                .list_prompts(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying get_prompt");
            self.wassette
                .get_prompt(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying list_resource_templates");
            self.wassette
                .list_resource_templates(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, McpError>> + Send + '_ {
        async move {
            tracing::debug!("Proxying complete");
            self.wassette
                .complete(request)
                .await
                .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
        }
    }

    fn get_info(&self) -> ServerInfo {
        let result = girt_info(&self.wassette_capabilities);
        ServerInfo {
            protocol_version: result.protocol_version,
            capabilities: result.capabilities,
            server_info: result.server_info,
            instructions: result.instructions,
        }
    }
}
