//! `agents-manager serve` — MCP stdio 服务器，供 IDE Agent 外部控制资产。

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

use agent_manager_core::asset_ops::AssetFileDetail;
use agent_manager_core::doctor::DoctorReport;
use agent_manager_core::error::CoreError;

use crate::agent_api::{self, parse_kind, parse_platform, resolve_scope_root, EnvSummary};
use crate::lifecycle::{ListReport, StatusReport};
use crate::projection::LifecycleReport;

#[derive(Debug, Clone)]
pub struct AgentManagerMcpServer {
    default_root: Arc<Utf8PathBuf>,
    tool_router: ToolRouter<Self>,
}

impl AgentManagerMcpServer {
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
impl AgentManagerMcpServer {
    #[tool(description = "列出已纳管资产（skills / rules / agents / commands / mcp）")]
    async fn agent_manager_list(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<ListReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::list(&root).map_err(Self::err)?))
    }

    #[tool(description = "各资产在各平台的链接/同步状态")]
    async fn agent_manager_status(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<StatusReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::status(&root).map_err(Self::err)?))
    }

    #[tool(description = "健康检查：断链、缺 secrets、平台能力问题等")]
    async fn agent_manager_doctor(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<DoctorReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::doctor(&root).map_err(Self::err)?))
    }

    #[tool(description = "生成 source-first 同步计划；仅 apply=true 才会写入目标平台")]
    async fn agent_manager_sync(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<LifecycleReport>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(
            agent_api::sync(&root, params.apply).map_err(Self::err)?,
        ))
    }

    #[tool(description = "读取单条资产正文与元数据")]
    async fn agent_manager_show(
        &self,
        Parameters(params): Parameters<KindNameParam>,
    ) -> Result<Json<AssetFileDetail>, McpError> {
        let kind = parse_kind(&params.kind).map_err(Self::err_msg)?;
        let root = self.root_for(params.root);
        Ok(Json(
            agent_api::show(&root, kind, &params.name).map_err(Self::err)?,
        ))
    }

    #[tool(description = "保存单条资产正文到 agents-manager 源目录")]
    async fn agent_manager_save(
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
    async fn agent_manager_deploy(
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
    async fn agent_manager_retract(
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
    async fn agent_manager_env(
        &self,
        Parameters(params): Parameters<RootParam>,
    ) -> Result<Json<EnvSummary>, McpError> {
        let root = self.root_for(params.root);
        Ok(Json(agent_api::scan_summary(&root).map_err(Self::err)?))
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "agents-manager",
    instructions = "Manage agents-manager assets across Cursor, Codex, Claude Code, and Hermes. Prefer agent_manager_doctor before sync/deploy. Never expose secrets values."
)]
impl ServerHandler for AgentManagerMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "agents-manager MCP: list/show/save/deploy/retract assets; doctor/status/sync for health and distribution.",
            )
    }
}

/// 启动 MCP stdio 服务（阻塞直到客户端断开）。
pub fn run(default_root: Utf8PathBuf) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agent_manager_cli=info,rmcp=warn".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init()
        .ok();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let server = AgentManagerMcpServer::new(default_root);
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use agent_manager_core::paths;
    use tempfile::TempDir;

    use super::*;

    const MISSING_SECRET_KEY: &str = "T009_MISSING_CATALOG_TOKEN";
    const SECRET_VALUE_SENTINEL: &str = "t009-mcp-secret-value-must-not-leak";

    static MCP_SYNC_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn projection_fixture() -> TempDir {
        let repo = TempDir::new().expect("temporary project");
        let source = repo.path().join(".agents-manager");
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
        let server = AgentManagerMcpServer::new(paths::discover_global_asset_root());
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
        let server = AgentManagerMcpServer::new(paths::discover_global_asset_root());
        let tools = server.tool_router.list_all();
        for name in ["agent_manager_sync", "agent_manager_deploy", "agent_manager_retract"] {
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
            .find(|tool| tool.name == "agent_manager_sync")
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
        let server = AgentManagerMcpServer::new(root.clone());

        let plan_only = server
            .agent_manager_sync(Parameters(RootParam {
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
                server.agent_manager_deploy(Parameters(params)).await
            } else {
                server.agent_manager_retract(Parameters(params)).await
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
            .agent_manager_sync(Parameters(RootParam {
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

    #[tokio::test(flavor = "current_thread")]
    async fn sync_handler_missing_mcp_secret_returns_a_safe_partial_report_and_applies_direct_assets(
    ) {
        let _lock = MCP_SYNC_ENV_LOCK.lock().await;
        let home = TempDir::new().expect("temporary HOME");
        let _home = EnvVarGuard::set("HOME", home.path());
        let _asset_root = EnvVarGuard::remove("AGENT_MANAGER_ROOT");
        let secrets = TempDir::new().expect("temporary secret directory");
        let secret_path = Utf8PathBuf::from_path_buf(secrets.path().join("secrets.env"))
            .expect("temporary secret path is UTF-8");
        agent_manager_core::secrets::save_to(
            &[(
                "UNRELATED_TEST_SECRET".to_owned(),
                SECRET_VALUE_SENTINEL.to_owned(),
            )],
            &secret_path,
        )
        .expect("seed strict temporary secret store");
        let _secret_dir = EnvVarGuard::set("AGENT_MANAGER_SECRETS_DIR", secrets.path());

        let repo = projection_fixture();
        let source = repo.path().join(".agents-manager/mcp/servers/catalog.json");
        fs::create_dir_all(source.parent().expect("MCP source parent")).expect("MCP source parent");
        fs::write(
            source,
            format!(
                r#"{{
  "enabled": true,
  "targets": ["cursor"],
  "config": {{
    "command": "catalog-mcp",
    "env": {{ "{MISSING_SECRET_KEY}": "${{{MISSING_SECRET_KEY}}}" }}
  }}
}}"#
            ),
        )
        .expect("write missing-secret canonical MCP source");
        let root = Utf8PathBuf::from_path_buf(repo.path().to_path_buf()).expect("UTF-8 repo");
        let server = AgentManagerMcpServer::new(root);

        let report = server
            .agent_manager_sync(Parameters(RootParam {
                root: None,
                apply: true,
            }))
            .await
            .expect("MCP sync must return a partial report, not a retryable error")
            .0;
        let report_json = serde_json::to_value(&report).expect("serialize MCP lifecycle report");
        let serialized = serde_json::to_string(&report_json).expect("serialize report text");

        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("mcp_missing_secret_keys"),
            "MCP missing-secret state must take precedence over unrelated report-only actions: {report_json:?}"
        );
        assert!(
            report_json["apply"]["skipped"].as_u64().unwrap_or_default() > 0,
            "MCP sync must report the missing server as skipped: {report_json:?}"
        );
        assert_eq!(
            report_json["apply"]["mcp_skipped_members"][0]["missing_secret_keys"],
            serde_json::json!([MISSING_SECRET_KEY]),
            "MCP reports may name unavailable keys but not their values"
        );
        assert!(
            !serialized.contains(SECRET_VALUE_SENTINEL),
            "MCP lifecycle report must not leak values from the caller-owned secret store"
        );
        assert!(
            repo.path().join(".agents/skills/demo").exists(),
            "a skipped MCP member must not prevent direct assets from applying"
        );
        assert!(
            !repo.path().join(".cursor/mcp.json").exists(),
            "a missing-secret MCP member must not render a platform container"
        );
    }
}
