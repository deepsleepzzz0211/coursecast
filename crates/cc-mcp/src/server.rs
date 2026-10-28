//! ServerHandler wiring: the 16-tool face of PRD §7.2 over stdio. Handler bodies
//! delegate to `tools::call_tool`; everything here is protocol, nothing is business.

use crate::tools::{call_tool, tool_specs, ToolContext};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorData, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::{service::RequestContext, RoleServer, ServerHandler};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct CourseCastMcp {
    ctx: Arc<ToolContext>,
}

impl CourseCastMcp {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        CourseCastMcp {
            ctx: Arc::new(ToolContext { data_dir }),
        }
    }
}

impl ServerHandler for CourseCastMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("coursecast", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "课影 CourseCast：课件 + 逐字稿 + 出镜视频 → 讲解视频。先 project_status 看缺什么，\
                 按返回的 next 建议走 slide_render → script_set → voice_synthesize → \
                 person_matte（可选）→ layout_set → render；每步工具幂等，重跑只补缺。",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools: Vec<Tool> = tool_specs()
            .into_iter()
            .map(|spec| {
                Tool::new(
                    spec.name,
                    spec.description,
                    Arc::new(
                        spec.schema
                            .as_object()
                            .expect("tool schemas are objects")
                            .clone(),
                    ),
                )
            })
            .collect();
        Ok(ListToolsResult {
            tools,
            ..ListToolsResult::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments: JsonObject = request.arguments.unwrap_or_default();
        let outcome = call_tool(&self.ctx, &request.name, &arguments);
        let text = serde_json::to_string_pretty(&outcome.value)
            .expect("serializing a JSON value cannot fail");
        let result = if outcome.is_error {
            CallToolResult::structured_error(outcome.value.clone())
        } else {
            CallToolResult::structured(outcome.value.clone())
        };
        // content text mirrors structuredContent for clients that only render text
        let mut result = result;
        result.content = vec![rmcp::model::ContentBlock::text(text)];
        Ok(CallToolResponse::Complete(result))
    }
}
