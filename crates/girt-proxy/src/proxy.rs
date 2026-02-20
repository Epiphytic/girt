use std::sync::Arc;

use girt_core::decision::{Decision, GateKind};
use girt_core::engine::DecisionEngine;
use girt_core::spec::{CapabilitySpec, ExecutionRequest, GateInput};
use girt_pipeline::llm::LlmClient;
use girt_pipeline::orchestrator::{Orchestrator, PipelineOutcome};
use girt_pipeline::publish::Publisher;
use girt_pipeline::types::{CapabilityRequest, RequestSource};
use rmcp::{
    ErrorData as McpError, Peer, RoleClient, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, CompleteRequestParams, CompleteResult, Content,
        GetPromptRequestParams, GetPromptResult, InitializeRequestParams, InitializeResult,
        ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, ServerCapabilities,
        ServerInfo, Tool,
    },
    service::RequestContext,
};
use tokio::sync::Mutex;

/// MCP proxy that routes agent requests through decision gates to Wassette.
///
/// Phase 1: Intercepts call_tool through Execution Gate, provides
/// request_capability tool through Creation Gate.
///
/// Phase 2: When Creation Gate allows, triggers the build pipeline
/// (Architect -> Engineer -> QA -> Red Team) and publishes the result.
pub struct GirtProxy {
    wassette: Peer<RoleClient>,
    wassette_capabilities: ServerCapabilities,
    engine: Arc<DecisionEngine>,
    llm: Arc<dyn LlmClient>,
    publisher: Arc<Publisher>,
    /// Server peer for sending notifications (set during handler calls).
    server_peer: Arc<Mutex<Option<Peer<RoleServer>>>>,
}

impl GirtProxy {
    pub fn new(
        wassette: Peer<RoleClient>,
        wassette_init: InitializeResult,
        engine: Arc<DecisionEngine>,
        llm: Arc<dyn LlmClient>,
        publisher: Arc<Publisher>,
    ) -> Self {
        Self {
            wassette,
            wassette_capabilities: wassette_init.capabilities,
            engine,
            llm,
            publisher,
            server_peer: Arc::new(Mutex::new(None)),
        }
    }
}

fn girt_info(capabilities: &ServerCapabilities) -> InitializeResult {
    InitializeResult {
        protocol_version: Default::default(),
        capabilities: capabilities.clone(),
        server_info: rmcp::model::Implementation::from_build_env(),
        instructions: Some("GIRT MCP Proxy -- Generative Isolated Runtime for Tools".into()),
    }
}

/// Build the JSON schema for the request_capability tool.
fn request_capability_tool() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "A descriptive snake_case name for the tool"
            },
            "description": {
                "type": "string",
                "description": "What this tool does and why it is needed"
            },
            "inputs": {
                "type": "object",
                "description": "Input parameter schema"
            },
            "outputs": {
                "type": "object",
                "description": "Expected output schema"
            },
            "constraints": {
                "type": "object",
                "description": "Security constraints (network hosts, storage paths, secrets)",
                "properties": {
                    "network": { "type": "array", "items": { "type": "string" } },
                    "storage": { "type": "array", "items": { "type": "string" } },
                    "secrets": { "type": "array", "items": { "type": "string" } }
                }
            }
        },
        "required": ["name", "description"]
    });

    Tool {
        name: "request_capability".into(),
        title: None,
        description: Some(
            "Request a new capability/tool to be built. \
             Provide a JSON specification describing what the tool should do, \
             its inputs, outputs, and security constraints."
                .into(),
        ),
        input_schema: schema.as_object().cloned().unwrap_or_default().into(),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

/// Format a decision into MCP-compatible content.
fn decision_to_content(decision: &Decision) -> Vec<Content> {
    let json = match decision {
        Decision::Allow => serde_json::json!({
            "status": "allowed",
            "message": "Request approved"
        }),
        Decision::Deny { reason } => serde_json::json!({
            "status": "denied",
            "reason": reason
        }),
        Decision::Defer { target } => serde_json::json!({
            "status": "deferred",
            "target": target
        }),
        Decision::Ask { prompt, context } => serde_json::json!({
            "status": "ask",
            "prompt": prompt,
            "context": context
        }),
    };

    vec![Content::text(json.to_string())]
}

fn make_tool_result(content: Vec<Content>, is_error: bool) -> CallToolResult {
    CallToolResult {
        content,
        structured_content: None,
        is_error: Some(is_error),
        meta: None,
    }
}

impl ServerHandler for GirtProxy {
    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        // Capture the server peer for later notifications
        let mut peer_lock = self.server_peer.lock().await;
        *peer_lock = Some(context.peer.clone());
        Ok(girt_info(&self.wassette_capabilities))
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        tracing::debug!("Proxying list_tools");

        // Get tools from Wassette
        let mut result = self
            .wassette
            .list_tools(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))?;

        // Add GIRT's own tools
        result.tools.push(request_capability_tool());

        // Add cached tools (built by pipeline)
        if let Ok(cached_names) = self.publisher.cache().list().await {
            for name in cached_names {
                if let Ok(Some(artifact)) = self.publisher.cache().get(&name).await {
                    let tool = Tool {
                        name: artifact.spec.name.into(),
                        title: None,
                        description: Some(artifact.spec.description.into()),
                        input_schema: artifact
                            .spec
                            .inputs
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    };
                    result.tools.push(tool);
                }
            }
        }

        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_name: &str = &request.name;

        // Handle GIRT built-in tools
        if tool_name == "request_capability" {
            return self.handle_request_capability(request).await;
        }

        // Run Execution Gate on all other tool calls
        let exec_input = GateInput::Execution(ExecutionRequest {
            tool_name: tool_name.to_string(),
            arguments: request
                .arguments
                .as_ref()
                .map(|args| serde_json::to_value(args).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null),
        });

        tracing::info!(tool = %tool_name, "Evaluating tool call through Execution Gate");

        let gate_result = self
            .engine
            .evaluate(GateKind::Execution, &exec_input)
            .await
            .map_err(|e| McpError::internal_error(format!("Decision engine error: {e}"), None))?;

        tracing::info!(
            tool = %tool_name,
            decision = ?gate_result.decision,
            layer = %gate_result.layer,
            "Execution Gate decision"
        );

        match &gate_result.decision {
            Decision::Allow => {
                tracing::info!(tool = %tool_name, "Proxying allowed call_tool to Wassette");
                self.wassette
                    .call_tool(request)
                    .await
                    .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
            }
            Decision::Deny { .. } => {
                tracing::warn!(tool = %tool_name, "Tool call denied");
                Ok(make_tool_result(
                    decision_to_content(&gate_result.decision),
                    true,
                ))
            }
            _ => Ok(make_tool_result(
                decision_to_content(&gate_result.decision),
                false,
            )),
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        tracing::debug!("Proxying list_resources");
        self.wassette
            .list_resources(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        tracing::debug!("Proxying read_resource");
        self.wassette
            .read_resource(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        tracing::debug!("Proxying list_prompts");
        self.wassette
            .list_prompts(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        tracing::debug!("Proxying get_prompt");
        self.wassette
            .get_prompt(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        tracing::debug!("Proxying list_resource_templates");
        self.wassette
            .list_resource_templates(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
    }

    async fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        tracing::debug!("Proxying complete");
        self.wassette
            .complete(request)
            .await
            .map_err(|e| McpError::internal_error(format!("Wassette error: {e}"), None))
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

impl GirtProxy {
    async fn handle_request_capability(
        &self,
        request: CallToolRequestParams,
    ) -> Result<CallToolResult, McpError> {
        let spec: CapabilitySpec = request
            .arguments
            .as_ref()
            .map(|args| serde_json::from_value(serde_json::to_value(args).unwrap_or_default()))
            .transpose()
            .map_err(|e| McpError::invalid_params(format!("Invalid capability spec: {e}"), None))?
            .unwrap_or_else(|| CapabilitySpec {
                name: "unknown".into(),
                description: "No description provided".into(),
                inputs: serde_json::Value::Null,
                outputs: serde_json::Value::Null,
                constraints: Default::default(),
            });

        tracing::info!(
            name = %spec.name,
            "Evaluating capability request through Creation Gate"
        );

        let input = GateInput::Creation(spec.clone());

        let gate_result = self
            .engine
            .evaluate(GateKind::Creation, &input)
            .await
            .map_err(|e| McpError::internal_error(format!("Decision engine error: {e}"), None))?;

        tracing::info!(
            decision = ?gate_result.decision,
            layer = %gate_result.layer,
            "Creation Gate decision"
        );

        match &gate_result.decision {
            Decision::Allow => {
                // Creation allowed -- trigger build pipeline
                self.trigger_build(spec).await
            }
            Decision::Deny { .. } => Ok(make_tool_result(
                decision_to_content(&gate_result.decision),
                true,
            )),
            _ => Ok(make_tool_result(
                decision_to_content(&gate_result.decision),
                false,
            )),
        }
    }

    /// Trigger the build pipeline for an approved capability request.
    async fn trigger_build(&self, spec: CapabilitySpec) -> Result<CallToolResult, McpError> {
        let cap_request = CapabilityRequest::new(spec, RequestSource::Operator);
        let tool_name = cap_request.spec.name.clone();

        tracing::info!(
            id = %cap_request.id,
            tool = %tool_name,
            "Triggering build pipeline"
        );

        let orchestrator = Orchestrator::new(self.llm.as_ref());
        let outcome = orchestrator.run(&cap_request).await;

        match outcome {
            PipelineOutcome::Built(artifact) => {
                tracing::info!(
                    tool = %tool_name,
                    iterations = artifact.build_iterations,
                    "Build pipeline succeeded"
                );

                // Publish to local cache
                match self.publisher.publish(&artifact).await {
                    Ok(result) => {
                        tracing::info!(
                            tool = %result.tool_name,
                            path = %result.local_path.display(),
                            "Tool published"
                        );

                        // Send tools/list_changed notification
                        self.notify_tools_changed().await;

                        let response = serde_json::json!({
                            "status": "built",
                            "tool_name": result.tool_name,
                            "build_iterations": artifact.build_iterations,
                            "tests_run": artifact.qa_result.tests_run,
                            "tests_passed": artifact.qa_result.tests_passed,
                            "exploits_attempted": artifact.security_result.exploits_attempted,
                            "exploits_succeeded": artifact.security_result.exploits_succeeded,
                        });
                        Ok(make_tool_result(
                            vec![Content::text(response.to_string())],
                            false,
                        ))
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to publish artifact");
                        Ok(make_tool_result(
                            vec![Content::text(format!(
                                "{{\"status\": \"publish_failed\", \"error\": \"{e}\"}}"
                            ))],
                            true,
                        ))
                    }
                }
            }
            PipelineOutcome::RecommendExtend { target, features } => {
                tracing::info!(
                    tool = %tool_name,
                    target = %target,
                    "Architect recommends extending existing tool"
                );
                let response = serde_json::json!({
                    "status": "recommend_extend",
                    "target_tool": target,
                    "features": features,
                    "message": format!("Consider extending '{}' instead of building a new tool", target),
                });
                Ok(make_tool_result(
                    vec![Content::text(response.to_string())],
                    false,
                ))
            }
            PipelineOutcome::Failed(e) => {
                tracing::error!(
                    tool = %tool_name,
                    error = %e,
                    "Build pipeline failed"
                );
                let response = serde_json::json!({
                    "status": "build_failed",
                    "error": e.to_string(),
                });
                Ok(make_tool_result(
                    vec![Content::text(response.to_string())],
                    true,
                ))
            }
        }
    }

    /// Send a tools/list_changed notification to the connected agent.
    async fn notify_tools_changed(&self) {
        let peer_lock = self.server_peer.lock().await;
        if let Some(peer) = peer_lock.as_ref() {
            if let Err(e) = peer.notify_tool_list_changed().await {
                tracing::warn!(error = %e, "Failed to send tools/list_changed notification");
            } else {
                tracing::info!("Sent tools/list_changed notification");
            }
        } else {
            tracing::warn!("No server peer available for tools/list_changed notification");
        }
    }
}
