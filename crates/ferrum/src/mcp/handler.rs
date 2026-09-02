use crate::auth::Caller;
use crate::routes::error::ApiError;
use crate::server::AppState;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde::Serialize;

pub const READ_ONLY: &str = "That API token is read-only.";

const INSTRUCTIONS: &str = "Ferrum runs the applications and databases on one Ubuntu server. \
The read tools report on applications, deploys, logs, metrics, certificates, databases and the \
host; the write tools set environment variables, edit custom nginx directives, restart, deploy, \
roll back, create databases, add domains and adjust resource limits. Deleting anything, managing \
people and hardening the host stay in the panel.";

pub struct Ferrum {
    pub(super) state: AppState,
    tools: ToolRouter<Ferrum>,
}

pub(super) type ToolResult = Result<CallToolResult, McpError>;

impl Ferrum {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tools: Self::read_router() + Self::write_router(),
        }
    }
}

fn caller(ctx: &RequestContext<RoleServer>) -> Result<Caller, McpError> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<Caller>())
        .cloned()
        .ok_or_else(|| McpError::internal_error("The request carried no caller.", None))
}

fn is_read_tool(tool: &Tool) -> bool {
    tool.annotations
        .as_ref()
        .and_then(|a| a.read_only_hint)
        .unwrap_or(false)
}

pub(super) fn refusal(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

/// What a route would answer with 4xx is a tool error the agent can read; a 5xx is a protocol
/// error, as it is for the panel.
pub(super) fn finish<T: Serialize>(outcome: Result<T, ApiError>) -> ToolResult {
    match outcome {
        Ok(value) => {
            let json = serde_json::to_value(value)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            Ok(CallToolResult::structured(json))
        }
        Err(e) if e.status.is_server_error() => Err(McpError::internal_error(e.message, None)),
        Err(e) => Ok(refusal(e.message)),
    }
}

impl ServerHandler for Ferrum {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        let mut implementation = Implementation::default();
        implementation.name = "ferrum".into();
        implementation.version = crate::cli::VERSION.into();
        info.server_info = implementation;
        info.instructions = Some(INSTRUCTIONS.into());
        info
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let read_only = caller(&ctx)?.is_read_only();
        let tools = self
            .tools
            .list_all()
            .into_iter()
            .filter(|tool| !read_only || is_read_tool(tool))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let write_tool = self
            .tools
            .get(&request.name)
            .is_some_and(|tool| !is_read_tool(tool));
        if write_tool && caller(&ctx)?.is_read_only() {
            return Ok(refusal(READ_ONLY));
        }
        self.tools
            .call(ToolCallContext::new(self, request, ctx))
            .await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.get(name).cloned()
    }
}
