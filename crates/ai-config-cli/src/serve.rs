//! `ai-config serve` — MCP stdio 服务器，供 IDE Agent 外部控制资产。

use std::sync::Arc;

use camino::Utf8PathBuf;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, Json, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ai_config_core::asset_ops::AssetFileDetail;
use ai_config_core::doctor::DoctorReport;
use ai_config_core::error::CoreError;

use crate::agent_api::{self, parse_kind, parse_platform, resolve_scope_root, EnvSummary};
use crate::lifecycle::{ListReport, StatusReport};
use crate::projection::LifecycleReport;

#[derive(Debug, Clone)]
pub struct AiConfigMcpServer {
    default_root: Arc<Utf8PathBuf>,
    tool_router: ToolRouter<Self>,
}

impl AiConfigMcpServer {
    pub fn new(default_root: Utf8PathBuf) -> Self {
        Self {
            default_root: Arc::new(default_root),
            tool_router: Self::tool_router(),
        }
    }

    fn root_for(&self, override_root: Option<String>) -> Utf8PathBuf {
        match override_root.filter(|s| !s.is_empty()) {
            Some(r) => resolve_scope_root(Some(&r)),
            None => (*self.default_root).clone(),
        }
    }

    fn err(e: CoreError) -> McpError {
        let data = e.hint().map(|h| serde_json::json!({ "hint": h }));
        McpError::invalid_params(e.to_string(), data)
    }

    fn err_msg(msg: String) -> McpError {
        McpError::invalid_params(msg, None)
    }
}

#[derive(Debug, Serialize, JsonSchema)]
struct OkMessage {
    ok: bool,
    message: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RootParam {
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    #[schemars(default)]
    apply: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KindNameParam {
    #[serde(default)]
    root: Option<String>,
    kind: String,
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SaveParam {
    #[serde(default)]
    root: Option<String>,
    kind: String,
    name: String,
    content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeployParam {
    #[serde(default)]
    root: Option<String>,
    kind: String,
    name: String,
    platform: String,
    #[serde(default)]
    #[schemars(default)]
    apply: bool,
}

#[tool_router]
impl AiConfigMcpServer {
    #[tool(description = "列出已纳管资产（skills / rules / agents / commands / mcp）")]
    async fn ai_config_list(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<ListReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::list(&root).map_err(Self::err)?))
    }

    #[tool(description = "各资产在各平台的链接/同步状态")]
    async fn ai_config_status(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<StatusReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::status(&root).map_err(Self::err)?))
    }

    #[tool(description = "健康检查：断链、缺 secrets、平台能力问题等")]
    async fn ai_config_doctor(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<DoctorReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::doctor(&root).map_err(Self::err)?))
    }

    #[tool(description = "生成 source-first 同步计划；仅 apply=true 才会写入目标平台")]
    async fn ai_config_sync(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<LifecycleReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::sync(&root, params.apply).map_err(Self::err)?))
    }

    #[tool(description = "读取单条资产正文与元数据")]
    async fn ai_config_show(
        &self,
        Parameters(params): Parameters<KindNameParam>,
    ) -> Result<Json<AssetFileDetail>, McpError> {
        let kind = parse_kind(&params.kind).map_err(Self::err_msg)?;
        let root = self.root_for(params.root);
        Ok(Json(
            agent_api::show(&root, kind, &params.name).map_err(Self::err)?,
        ))
    }

    #[tool(description = "保存单条资产正文到 ai-config 源目录")]
    async fn ai_config_save(
        &self,
        Parameters(params): Parameters<SaveParam>,
    ) -> Result<Json<OkMessage>, McpError> {
        let kind = parse_kind(&params.kind).map_err(Self::err_msg)?;
        let root = self.root_for(params.root);
        let msg = agent_api::save(&root, kind, &params.name, &params.content).map_err(Self::err)?;
        Ok(Json(OkMessage {
            ok: true,
            message: msg,
        }))
    }

    #[tool(description = "下发单条资产到指定平台")]
    async fn ai_config_deploy(
        &self,
        Parameters(params): Parameters<DeployParam>,
    ) -> Result<Json<OkMessage>, McpError> {
        let kind = parse_kind(&params.kind).map_err(Self::err_msg)?;
        let platform = parse_platform(&params.platform).map_err(Self::err_msg)?;
        let root = self.root_for(params.root);
        let msg = agent_api::deploy(&root, kind, &params.name, platform, params.apply)
            .map_err(Self::err)?;
        Ok(Json(OkMessage {
            ok: true,
            message: msg,
        }))
    }

    #[tool(description = "从指定平台收回单条资产副本")]
    async fn ai_config_retract(
        &self,
        Parameters(params): Parameters<DeployParam>,
    ) -> Result<Json<OkMessage>, McpError> {
        let kind = parse_kind(&params.kind).map_err(Self::err_msg)?;
        let platform = parse_platform(&params.platform).map_err(Self::err_msg)?;
        let root = self.root_for(params.root);
        let msg = agent_api::retract(&root, kind, &params.name, platform, params.apply)
            .map_err(Self::err)?;
        Ok(Json(OkMessage {
            ok: true,
            message: msg,
        }))
    }

    #[tool(description = "资产根目录扫描摘要（各类型数量）")]
    async fn ai_config_env(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<EnvSummary>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::scan_summary(&root).map_err(Self::err)?))
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "ai-config",
    instructions = "Manage ai-config assets across Cursor, Codex, Claude Code, and Hermes. Prefer ai_config_doctor before sync/deploy. Never expose secrets values."
)]
impl ServerHandler for AiConfigMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "ai-config MCP: list/show/save/deploy/retract assets; doctor/status/sync for health and distribution.",
            )
    }
}

/// 启动 MCP stdio 服务（阻塞直到客户端断开）。
pub fn run(default_root: Utf8PathBuf) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ai_config_cli=info,rmcp=warn".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init()
        .ok();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let server = AiConfigMcpServer::new(default_root);
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ai_config_core::paths;
    use tempfile::TempDir;

    use super::*;

    fn projection_fixture() -> TempDir {
        let repo = TempDir::new().expect("temporary project");
        let source = repo.path().join(".ai-config");
        fs::create_dir_all(source.join("skills/demo")).expect("skill parent");
        fs::write(source.join("skills/demo/SKILL.md"), "# canonical demo\n")
            .expect("canonical skill");
        fs::create_dir_all(source.join("prompts")).expect("prompt parent");
        fs::write(source.join("prompts/AGENTS.md"), "canonical instructions\n")
            .expect("canonical prompt");
        repo
    }

    #[test]
    fn tool_output_schemas_are_objects() {
        let server = AiConfigMcpServer::new(paths::discover_global_asset_root());
        let tools = server.tool_router.list_all();
        assert_eq!(tools.len(), 9);
        for tool in tools {
            let schema = tool
                .output_schema
                .as_ref()
                .expect("tool should have output_schema");
            assert_eq!(
                schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "tool `{}` output_schema must be object",
                tool.name
            );
        }
    }

    #[test]
    fn mutation_tools_require_explicit_apply_and_sync_returns_projection_plan() {
        let server = AiConfigMcpServer::new(paths::discover_global_asset_root());
        let tools = server.tool_router.list_all();
        for name in ["ai_config_sync", "ai_config_deploy", "ai_config_retract"] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("missing mutation tool {name}"));
            let apply = tool
                .input_schema
                .get("properties")
                .and_then(|properties| properties.get("apply"))
                .unwrap_or_else(|| panic!("{name} must expose an apply parameter"));
            assert_eq!(
                apply.get("default").and_then(|value| value.as_bool()),
                Some(false),
                "{name} must be plan-only unless apply=true is explicit"
            );
        }

        let sync = tools
            .iter()
            .find(|tool| tool.name == "ai_config_sync")
            .expect("sync tool");
        let output = sync.output_schema.as_ref().expect("sync output schema");
        let plan = output
            .get("properties")
            .and_then(|properties| properties.get("plan"))
            .expect("sync must return the exact projection plan");
        assert!(
            plan.get("properties")
                .and_then(|properties| properties.get("schema_version"))
                .is_some()
                && plan
                    .get("properties")
                    .and_then(|properties| properties.get("plan_digest"))
                    .is_some(),
            "agent and CLI must receive the same plan identity"
        );
        assert!(
            output
                .get("properties")
                .and_then(|properties| properties.get("apply"))
                .is_some(),
            "apply=true must return an apply report without hiding the plan"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sync_handler_is_plan_only_by_default_and_foreign_guard_blocks_apply() {
        let repo = projection_fixture();
        let root = Utf8PathBuf::from_path_buf(repo.path().to_path_buf()).expect("utf8 repo");
        let server = AiConfigMcpServer::new(root.clone());

        let plan_only = server
            .ai_config_sync(Parameters(RootParam {
                root: None,
                apply: false,
            }))
            .await
            .expect("plan-only MCP sync")
            .0;
        assert!(plan_only.apply.is_none(), "default MCP sync must not apply");
        assert!(
            !repo.path().join(".agents/skills/demo").exists(),
            "plan-only MCP sync must not create a target"
        );
        for operation in ["deploy", "retract"] {
            let params = DeployParam {
                root: None,
                kind: "skill".to_owned(),
                name: "demo".to_owned(),
                platform: "cursor".to_owned(),
                apply: true,
            };
            let result = if operation == "deploy" {
                server.ai_config_deploy(Parameters(params)).await
            } else {
                server.ai_config_retract(Parameters(params)).await
            };
            let error = match result {
                Ok(_) => panic!("single-item MCP {operation} must fail closed"),
                Err(error) => error,
            };
            assert!(
                error.message.contains("source-first projection"),
                "single-item MCP {operation} must fail closed until it has a plan adapter"
            );
        }
        assert!(
            !repo.path().join(".cursor/skills/demo/SKILL.md").exists(),
            "refused single-item MCP operations must never use the legacy write path"
        );

        let foreign = repo.path().join("AGENTS.md");
        fs::write(&foreign, "foreign instructions\n").expect("foreign project entry");
        let foreign_before = fs::read(&foreign).expect("read foreign entry");
        let blocked = server
            .ai_config_sync(Parameters(RootParam {
                root: None,
                apply: true,
            }))
            .await
            .expect("foreign guard is a report, not a retryable write")
            .0;
        assert!(
            blocked.blocking_reason.is_some(),
            "apply=true must surface the foreign ownership guard"
        );
        assert_eq!(fs::read(&foreign).unwrap(), foreign_before);
        assert!(
            !repo.path().join(".agents/skills/demo").exists(),
            "foreign guard must prevent all plan actions, not only the conflicting prompt"
        );
    }
}
