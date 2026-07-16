//! Source-first projection 的纯平台 target adapter。
//!
//! 本模块只声明目标与能力；目录创建、配置写入、secret 解析均属于后续 planner/executor。

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::model::{AssetKind, PlatformId};
use crate::projection::model::{
    DeploymentScope, EffectiveAsset, ProjectionMode, ProjectionSurface, ProjectionTarget, SourceRef,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetContext {
    pub scope: DeploymentScope,
    /// User scope 为 home；workspace/project scope 为对应 deploy root。
    pub deploy_base: Utf8PathBuf,
}

/// 平台实际消费的目标格式，独立于“链接/生成”的投影动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    Directory,
    DirectoryOrFile,
    Mdc,
    Markdown,
    Json,
    Toml,
    Yaml,
    None,
}

/// planner 必须在 apply 前满足的信任边界。adapter 只声明要求，不检查机器状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustRequirement {
    None,
    TrustedProject,
    TrustedProjectWithIndependentReview,
}

/// 旧路径只能供 inventory/migration 使用，不能成为普通 projection target。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyInventoryStatus {
    AlternateStillConsumed,
    DeprecatedConsumed,
    InventoryOnly,
    DifferentSemanticAsset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyInventoryPath {
    pub path: Utf8PathBuf,
    pub status: LegacyInventoryStatus,
    pub reason: &'static str,
}

/// 给 planner 的完整单资产契约。source 永远从 EffectiveAsset 取得，不能由 deploy scope 推断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityContract {
    pub source: SourceRef,
    pub deployment_scope: DeploymentScope,
    /// 同一物理 target 的全部消费者，固定排序且不重复。
    pub consumers: Vec<PlatformId>,
    pub target: Option<ProjectionTarget>,
    pub format: TargetFormat,
    pub projection_mode: Option<ProjectionMode>,
    pub trust_requirement: TrustRequirement,
    pub legacy_inventory_paths: Vec<LegacyInventoryPath>,
    /// 保留现有 facade，T009 前旧入口仍可安全共存。
    pub capability: PlatformCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCapabilityContract {
    pub source: SourceRef,
    pub deployment_scope: DeploymentScope,
    pub consumers: Vec<PlatformId>,
    pub binding_target: Option<ProjectionTarget>,
    pub binding_format: TargetFormat,
    pub binding_projection_mode: Option<ProjectionMode>,
    pub script_target: Option<ProjectionTarget>,
    pub script_format: TargetFormat,
    pub script_projection_mode: Option<ProjectionMode>,
    pub trust_requirement: TrustRequirement,
    pub legacy_inventory_paths: Vec<LegacyInventoryPath>,
    pub capability: HookCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCapability {
    DirectLink {
        target: ProjectionTarget,
        mode: ProjectionMode,
        surface: ProjectionSurface,
    },
    ExternalDirectory {
        target: ProjectionTarget,
        mode: ProjectionMode,
        surface: ProjectionSurface,
    },
    Generated {
        target: ProjectionTarget,
        mode: ProjectionMode,
        surface: ProjectionSurface,
    },
    Unsupported {
        reason: String,
    },
}

/// Hook 同时由聚合配置中的具名 binding 和可逐项链接的脚本单元组成。
///
/// 这只是纯目标描述；事件映射和配置合并将由后续 renderer/executor 处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookCapability {
    Supported {
        binding: PlatformCapability,
        script: PlatformCapability,
    },
    Unsupported {
        reason: String,
    },
}

/// 返回一个资产的 projection 能力；不创建目录，也不读取现存配置。
pub fn capability_for(
    platform: PlatformId,
    kind: AssetKind,
    name: &str,
    context: &TargetContext,
) -> PlatformCapability {
    if !is_safe_asset_name(name) {
        return PlatformCapability::Unsupported {
            reason: "asset name must be one safe path segment".to_owned(),
        };
    }

    match (platform, kind) {
        (_, AssetKind::Prompt) => PlatformCapability::Unsupported {
            reason: "Prompt requires the source-first projection planner; legacy lifecycle is disabled"
                .to_owned(),
        },
        (PlatformId::Cursor | PlatformId::Codex, AssetKind::Skill) => {
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: context.deploy_base.join(".agents/skills").join(name),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::SharedTarget {
                    key: "agents_skills".to_owned(),
                },
            }
        }
        (PlatformId::Claude, AssetKind::Skill) => PlatformCapability::DirectLink {
            target: ProjectionTarget {
                path: context.deploy_base.join(".claude/skills").join(name),
                entry_key: None,
            },
            mode: ProjectionMode::DirectLink,
            surface: ProjectionSurface::Platform(PlatformId::Claude),
        },
        (PlatformId::Hermes, AssetKind::Skill) if context.scope == DeploymentScope::User => {
            PlatformCapability::ExternalDirectory {
                target: ProjectionTarget {
                    path: context.deploy_base.join(".hermes/config.yaml"),
                    entry_key: Some("skills.external_dirs".to_owned()),
                },
                mode: ProjectionMode::ExternalDirectory,
                surface: ProjectionSurface::Platform(PlatformId::Hermes),
            }
        }
        (PlatformId::Codex, AssetKind::Rule) => PlatformCapability::Unsupported {
            reason:
                "Codex instruction rules are unsupported; use canonical AGENTS prompt, never .codex/rules"
                    .to_owned(),
        },
        (PlatformId::Cursor, AssetKind::Rule) if context.scope == DeploymentScope::User => {
            PlatformCapability::Unsupported {
                reason: "Cursor user rules have no stable file target".to_owned(),
            }
        }
        (PlatformId::Cursor, AssetKind::Rule) if context.scope == DeploymentScope::Project => {
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: context
                        .deploy_base
                        .join(".cursor/rules")
                        .join(format!("{name}.mdc")),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::SharedTarget {
                    key: "cursor_hermes_rules".to_owned(),
                },
            }
        }
        (PlatformId::Cursor, AssetKind::Rule) if context.scope == DeploymentScope::Workspace => {
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: context
                        .deploy_base
                        .join(".cursor/rules")
                        .join(format!("{name}.mdc")),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            }
        }
        (PlatformId::Hermes, AssetKind::Rule) if context.scope == DeploymentScope::Project => {
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: context
                        .deploy_base
                        .join(".cursor/rules")
                        .join(format!("{name}.mdc")),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::SharedTarget {
                    key: "cursor_hermes_rules".to_owned(),
                },
            }
        }
        (PlatformId::Hermes, AssetKind::Rule) => PlatformCapability::Unsupported {
            reason: "Hermes user rules are unsupported; do not write SOUL.md".to_owned(),
        },
        (PlatformId::Codex, AssetKind::Mcp) => PlatformCapability::Generated {
            target: ProjectionTarget {
                path: context.deploy_base.join(".codex/config.toml"),
                entry_key: Some(format!("mcp_servers.{name}")),
            },
            mode: ProjectionMode::GeneratedToml,
            surface: ProjectionSurface::Platform(PlatformId::Codex),
        },
        (PlatformId::Cursor, AssetKind::Mcp) => PlatformCapability::Generated {
            target: ProjectionTarget {
                path: context.deploy_base.join(".cursor/mcp.json"),
                entry_key: Some(format!("mcpServers.{name}")),
            },
            mode: ProjectionMode::GeneratedJson,
            surface: ProjectionSurface::Platform(PlatformId::Cursor),
        },
        (PlatformId::Claude, AssetKind::Mcp) if context.scope == DeploymentScope::User => {
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: context.deploy_base.join(".claude.json"),
                    entry_key: Some(format!("mcpServers.{name}")),
                },
                mode: ProjectionMode::GeneratedJson,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        }
        (PlatformId::Claude, AssetKind::Mcp) if context.scope != DeploymentScope::User => {
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: context.deploy_base.join(".mcp.json"),
                    entry_key: Some(format!("mcpServers.{name}")),
                },
                mode: ProjectionMode::GeneratedJson,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        }
        (PlatformId::Hermes, AssetKind::Mcp) if context.scope == DeploymentScope::User => {
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: context.deploy_base.join(".hermes/config.yaml"),
                    entry_key: Some(format!("mcp_servers.{name}")),
                },
                mode: ProjectionMode::GeneratedYaml,
                surface: ProjectionSurface::Platform(PlatformId::Hermes),
            }
        }
        (PlatformId::Hermes, AssetKind::Mcp) => PlatformCapability::Unsupported {
            reason: "Hermes project MCP is unsupported; do not modify global config.yaml".to_owned(),
        },
        (PlatformId::Codex, AssetKind::Agent) => PlatformCapability::Generated {
            target: ProjectionTarget {
                path: context
                    .deploy_base
                    .join(".codex/agents")
                    .join(format!("{name}.toml")),
                entry_key: None,
            },
            mode: ProjectionMode::GeneratedToml,
            surface: ProjectionSurface::Platform(PlatformId::Codex),
        },
        (PlatformId::Cursor, AssetKind::Agent) => PlatformCapability::Generated {
            target: ProjectionTarget {
                path: context
                    .deploy_base
                    .join(".cursor/agents")
                    .join(format!("{name}.md")),
                entry_key: None,
            },
            mode: ProjectionMode::GeneratedMarkdown,
            surface: ProjectionSurface::Platform(PlatformId::Cursor),
        },
        (PlatformId::Claude, AssetKind::Agent) => PlatformCapability::Generated {
            target: ProjectionTarget {
                path: context
                    .deploy_base
                    .join(".claude/agents")
                    .join(format!("{name}.md")),
                entry_key: None,
            },
            mode: ProjectionMode::GeneratedMarkdown,
            surface: ProjectionSurface::Platform(PlatformId::Claude),
        },
        (PlatformId::Hermes, AssetKind::Agent) => PlatformCapability::Unsupported {
            reason: "Hermes has no static agent directory".to_owned(),
        },
        (PlatformId::Codex, AssetKind::Command) => PlatformCapability::Unsupported {
            reason: "Codex commands are unsupported; legacy .codex/prompts is inventory only"
                .to_owned(),
        },
        (PlatformId::Cursor, AssetKind::Command) => PlatformCapability::DirectLink {
            target: ProjectionTarget {
                path: context
                    .deploy_base
                    .join(".cursor/commands")
                    .join(format!("{name}.md")),
                entry_key: None,
            },
            mode: ProjectionMode::DirectLink,
            surface: ProjectionSurface::Platform(PlatformId::Cursor),
        },
        (PlatformId::Claude, AssetKind::Command) => PlatformCapability::DirectLink {
            target: ProjectionTarget {
                path: context
                    .deploy_base
                    .join(".claude/commands")
                    .join(format!("{name}.md")),
                entry_key: None,
            },
            mode: ProjectionMode::DirectLink,
            surface: ProjectionSurface::Platform(PlatformId::Claude),
        },
        (PlatformId::Hermes, AssetKind::Command) => PlatformCapability::Unsupported {
            reason: "Hermes commands are unsupported; use Skills".to_owned(),
        },
        (PlatformId::Claude, AssetKind::Rule) => PlatformCapability::Generated {
            target: ProjectionTarget {
                path: context
                    .deploy_base
                    .join(".claude/rules")
                    .join(format!("{name}.md")),
                entry_key: None,
            },
            mode: ProjectionMode::GeneratedMarkdown,
            surface: ProjectionSurface::Platform(PlatformId::Claude),
        },
        _ => PlatformCapability::Unsupported {
            reason: format!(
                "projection adapter has no contract for platform {platform:?} and asset {kind:?}"
            ),
        },
    }
}

/// 以 overlay 后的 EffectiveAsset 生成完整机器可读契约。
///
/// 这是新 planner 的入口；旧 `capability_for` 仅保留给尚未迁移的兼容调用方。
pub fn capability_contract_for(
    platform: PlatformId,
    asset: &EffectiveAsset,
    context: &TargetContext,
) -> CapabilityContract {
    let capability = capability_for(platform, asset.kind, &asset.name, context);
    let (target, projection_mode) = capability_target_and_mode(&capability);
    let supported = target.is_some();
    CapabilityContract {
        source: asset.source_ref(),
        deployment_scope: context.scope,
        consumers: if supported {
            consumers_for(platform, asset.kind, context.scope)
        } else {
            Vec::new()
        },
        target,
        format: target_format_for(platform, asset.kind, projection_mode),
        projection_mode,
        trust_requirement: trust_requirement_for(platform, asset.kind, context.scope),
        legacy_inventory_paths: legacy_inventory_paths_for(
            platform,
            asset.kind,
            &asset.name,
            context,
        ),
        capability,
    }
}

/// Hook 有 binding/script 两个 target，不能将它们压缩成单一普通资产契约。
pub fn hook_capability_contract_for(
    platform: PlatformId,
    asset: &EffectiveAsset,
    context: &TargetContext,
) -> HookCapabilityContract {
    let capability = if asset.kind == AssetKind::Hook {
        hook_capability_for(platform, &asset.name, context)
    } else {
        HookCapability::Unsupported {
            reason: "hook contract requires an EffectiveAsset of kind Hook".to_owned(),
        }
    };
    let (binding_target, binding_projection_mode, script_target, script_projection_mode) =
        match &capability {
            HookCapability::Supported { binding, script } => {
                let (binding_target, binding_mode) = capability_target_and_mode(binding);
                let (script_target, script_mode) = capability_target_and_mode(script);
                (binding_target, binding_mode, script_target, script_mode)
            }
            HookCapability::Unsupported { .. } => (None, None, None, None),
        };
    let supported = binding_target.is_some() && script_target.is_some();
    HookCapabilityContract {
        source: asset.source_ref(),
        deployment_scope: context.scope,
        consumers: if supported {
            vec![platform]
        } else {
            Vec::new()
        },
        binding_target,
        binding_format: target_format_for(platform, AssetKind::Hook, binding_projection_mode),
        binding_projection_mode,
        script_target,
        script_format: target_format_for(platform, AssetKind::Hook, script_projection_mode),
        script_projection_mode,
        trust_requirement: trust_requirement_for(platform, AssetKind::Hook, context.scope),
        legacy_inventory_paths: legacy_inventory_paths_for(
            platform,
            AssetKind::Hook,
            &asset.name,
            context,
        ),
        capability,
    }
}

fn capability_target_and_mode(
    capability: &PlatformCapability,
) -> (Option<ProjectionTarget>, Option<ProjectionMode>) {
    match capability {
        PlatformCapability::DirectLink { target, mode, .. }
        | PlatformCapability::ExternalDirectory { target, mode, .. }
        | PlatformCapability::Generated { target, mode, .. } => (Some(target.clone()), Some(*mode)),
        PlatformCapability::Unsupported { .. } => (None, None),
    }
}

fn consumers_for(platform: PlatformId, kind: AssetKind, scope: DeploymentScope) -> Vec<PlatformId> {
    match (kind, platform, scope) {
        (AssetKind::Skill, PlatformId::Cursor | PlatformId::Codex, _) => {
            vec![PlatformId::Cursor, PlatformId::Codex]
        }
        (AssetKind::Rule, PlatformId::Cursor | PlatformId::Hermes, DeploymentScope::Project) => {
            vec![PlatformId::Cursor, PlatformId::Hermes]
        }
        _ => vec![platform],
    }
}

fn target_format_for(
    platform: PlatformId,
    kind: AssetKind,
    mode: Option<ProjectionMode>,
) -> TargetFormat {
    match mode {
        None => TargetFormat::None,
        Some(ProjectionMode::ExternalDirectory) => TargetFormat::Yaml,
        Some(ProjectionMode::GeneratedJson) => TargetFormat::Json,
        Some(ProjectionMode::GeneratedToml) => TargetFormat::Toml,
        Some(ProjectionMode::GeneratedYaml) => TargetFormat::Yaml,
        Some(ProjectionMode::GeneratedMarkdown) => TargetFormat::Markdown,
        Some(ProjectionMode::DirectLink) => match kind {
            AssetKind::Skill => TargetFormat::Directory,
            AssetKind::Rule if platform != PlatformId::Claude => TargetFormat::Mdc,
            AssetKind::Hook => TargetFormat::DirectoryOrFile,
            AssetKind::Rule | AssetKind::Command | AssetKind::Agent | AssetKind::Prompt => {
                TargetFormat::Markdown
            }
            AssetKind::Mcp => TargetFormat::None,
        },
        Some(ProjectionMode::CopyFallback) => TargetFormat::None,
    }
}

fn trust_requirement_for(
    platform: PlatformId,
    kind: AssetKind,
    scope: DeploymentScope,
) -> TrustRequirement {
    match (platform, kind, scope) {
        (
            PlatformId::Codex,
            AssetKind::Mcp,
            DeploymentScope::Workspace | DeploymentScope::Project,
        ) => TrustRequirement::TrustedProject,
        (
            PlatformId::Codex,
            AssetKind::Hook,
            DeploymentScope::Workspace | DeploymentScope::Project,
        ) => TrustRequirement::TrustedProjectWithIndependentReview,
        _ => TrustRequirement::None,
    }
}

fn legacy_inventory_paths_for(
    platform: PlatformId,
    kind: AssetKind,
    name: &str,
    context: &TargetContext,
) -> Vec<LegacyInventoryPath> {
    let entry = |path: Utf8PathBuf, status, reason| LegacyInventoryPath {
        path,
        status,
        reason,
    };
    match (platform, kind) {
        (PlatformId::Cursor, AssetKind::Skill) => vec![entry(
            context.deploy_base.join(".cursor/skills").join(name),
            LegacyInventoryStatus::AlternateStillConsumed,
            "Cursor alternate skill location; never a new default target",
        )],
        (PlatformId::Codex, AssetKind::Skill) => vec![entry(
            context.deploy_base.join(".codex/skills").join(name),
            LegacyInventoryStatus::InventoryOnly,
            "legacy Codex skill location",
        )],
        (PlatformId::Codex, AssetKind::Rule) => vec![entry(
            context
                .deploy_base
                .join(".codex/rules")
                .join(format!("{name}.rules")),
            LegacyInventoryStatus::DifferentSemanticAsset,
            "Codex execution policy, not an instruction rule target",
        )],
        (PlatformId::Codex, AssetKind::Mcp) => vec![entry(
            context.deploy_base.join(".codex/mcp.json"),
            LegacyInventoryStatus::InventoryOnly,
            "legacy MCP JSON is not a Codex projection target",
        )],
        (PlatformId::Codex, AssetKind::Agent) => vec![entry(
            context.deploy_base.join(".codex/subagents").join(name),
            LegacyInventoryStatus::InventoryOnly,
            "legacy subagents directory",
        )],
        (PlatformId::Claude, AssetKind::Agent) => vec![entry(
            context.deploy_base.join(".claude/subagents").join(name),
            LegacyInventoryStatus::InventoryOnly,
            "legacy subagents directory",
        )],
        (PlatformId::Codex, AssetKind::Command) => vec![entry(
            context
                .deploy_base
                .join(".codex/prompts")
                .join(format!("{name}.md")),
            LegacyInventoryStatus::DeprecatedConsumed,
            "deprecated prompt inventory; recommend migration to Skill",
        )],
        (PlatformId::Codex, AssetKind::Hook) => vec![entry(
            context.deploy_base.join(".codex/config.toml"),
            LegacyInventoryStatus::InventoryOnly,
            "inline TOML hooks are inventory-only; new projection uses hooks.json",
        )],
        _ => Vec::new(),
    }
}

/// 返回 Hook 的 binding 与脚本单元目标，避免出现“只写配置”或“只放脚本”的半投影。
pub fn hook_capability_for(
    platform: PlatformId,
    script_name: &str,
    context: &TargetContext,
) -> HookCapability {
    if !is_safe_asset_name(script_name) {
        return HookCapability::Unsupported {
            reason: "asset name must be one safe path segment".to_owned(),
        };
    }

    let (config_path, hooks_dir, mode) = match platform {
        PlatformId::Cursor => (
            context.deploy_base.join(".cursor/hooks.json"),
            context.deploy_base.join(".cursor/hooks"),
            ProjectionMode::GeneratedJson,
        ),
        PlatformId::Codex => (
            context.deploy_base.join(".codex/hooks.json"),
            context.deploy_base.join(".codex/hooks"),
            ProjectionMode::GeneratedJson,
        ),
        PlatformId::Claude => (
            context.deploy_base.join(".claude/settings.json"),
            context.deploy_base.join(".claude/hooks"),
            ProjectionMode::GeneratedJson,
        ),
        PlatformId::Hermes if context.scope == DeploymentScope::User => (
            context.deploy_base.join(".hermes/config.yaml"),
            context.deploy_base.join(".hermes/hooks"),
            ProjectionMode::GeneratedYaml,
        ),
        PlatformId::Hermes => {
            return HookCapability::Unsupported {
                reason: "Hermes project Hook is unsupported; do not modify global config.yaml"
                    .to_owned(),
            };
        }
        PlatformId::AiConfig => {
            return HookCapability::Unsupported {
                reason: "ai-config is canonical source, not a projection target".to_owned(),
            };
        }
    };

    HookCapability::Supported {
        binding: PlatformCapability::Generated {
            target: ProjectionTarget {
                path: config_path,
                entry_key: Some(format!("hooks.{script_name}")),
            },
            mode,
            surface: ProjectionSurface::Platform(platform),
        },
        script: PlatformCapability::DirectLink {
            target: ProjectionTarget {
                path: hooks_dir.join(script_name),
                entry_key: None,
            },
            mode: ProjectionMode::DirectLink,
            surface: ProjectionSurface::Platform(platform),
        },
    }
}

fn is_safe_asset_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetKind, PlatformId};
    use crate::projection::model::{
        DeploymentScope, EffectiveAsset, ProjectionMode, ProjectionSurface, SourceLayer,
    };
    use camino::Utf8PathBuf;

    #[test]
    fn prompt_is_explicitly_reserved_for_the_source_first_planner() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/tmp/projection-contract"),
        };
        let capability = capability_for(PlatformId::Cursor, AssetKind::Prompt, "review", &context);
        assert!(matches!(
            capability,
            PlatformCapability::Unsupported { ref reason }
                if reason.contains("source-first projection planner")
        ));
    }

    #[test]
    fn contract_keeps_global_provenance_separate_from_project_target_and_shared_consumers() {
        let asset = EffectiveAsset {
            kind: AssetKind::Skill,
            name: "demo".to_owned(),
            source_path: Utf8PathBuf::from("/home/user/.ai-config/skills/demo"),
            layer: SourceLayer::Global,
            fingerprint: "source-v1".to_owned(),
        };
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        let contract = capability_contract_for(PlatformId::Cursor, &asset, &context);

        assert_eq!(contract.source, asset.source_ref());
        assert_eq!(contract.deployment_scope, DeploymentScope::Project);
        assert_eq!(
            contract.consumers,
            vec![PlatformId::Cursor, PlatformId::Codex]
        );
        assert_eq!(contract.trust_requirement, TrustRequirement::None);
        assert_eq!(contract.format, TargetFormat::Directory);
        assert_eq!(contract.projection_mode, Some(ProjectionMode::DirectLink));
        assert_eq!(
            contract.target.as_ref().map(|target| target.path.as_str()),
            Some("/repo/.agents/skills/demo")
        );
        assert_eq!(contract.legacy_inventory_paths.len(), 1);
        assert_eq!(
            contract.legacy_inventory_paths[0].path,
            Utf8PathBuf::from("/repo/.cursor/skills/demo")
        );
        assert_eq!(
            contract.legacy_inventory_paths[0].status,
            LegacyInventoryStatus::AlternateStillConsumed
        );
    }

    #[test]
    fn codex_project_mcp_contract_requires_trusted_project() {
        let asset = EffectiveAsset {
            kind: AssetKind::Mcp,
            name: "catalog".to_owned(),
            source_path: Utf8PathBuf::from("/home/user/.ai-config/mcp/servers/catalog.json"),
            layer: SourceLayer::Global,
            fingerprint: "source-v1".to_owned(),
        };
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        let contract = capability_contract_for(PlatformId::Codex, &asset, &context);

        assert_eq!(contract.trust_requirement, TrustRequirement::TrustedProject);
        assert_eq!(contract.format, TargetFormat::Toml);
        assert_eq!(
            contract.projection_mode,
            Some(ProjectionMode::GeneratedToml)
        );
        assert_eq!(contract.consumers, vec![PlatformId::Codex]);
        assert_eq!(contract.legacy_inventory_paths.len(), 1);
        assert_eq!(
            contract.legacy_inventory_paths[0].path,
            Utf8PathBuf::from("/repo/.codex/mcp.json")
        );
        assert_eq!(
            contract.legacy_inventory_paths[0].status,
            LegacyInventoryStatus::InventoryOnly
        );
    }

    #[test]
    fn codex_project_hook_contract_requires_independent_trust_review() {
        let asset = EffectiveAsset {
            kind: AssetKind::Hook,
            name: "format.sh".to_owned(),
            source_path: Utf8PathBuf::from("/home/user/.ai-config/hooks/format.sh"),
            layer: SourceLayer::Global,
            fingerprint: "source-v1".to_owned(),
        };
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        let contract = hook_capability_contract_for(PlatformId::Codex, &asset, &context);

        assert_eq!(contract.source, asset.source_ref());
        assert_eq!(
            contract.trust_requirement,
            TrustRequirement::TrustedProjectWithIndependentReview
        );
        assert_eq!(contract.deployment_scope, DeploymentScope::Project);
        assert_eq!(contract.consumers, vec![PlatformId::Codex]);
        assert_eq!(contract.binding_format, TargetFormat::Json);
        assert_eq!(contract.script_format, TargetFormat::DirectoryOrFile);
    }

    #[test]
    fn workspace_cursor_rule_does_not_claim_hermes_as_a_shared_consumer() {
        let asset = EffectiveAsset {
            kind: AssetKind::Rule,
            name: "review".to_owned(),
            source_path: Utf8PathBuf::from("/home/user/.ai-config/rules/review.mdc"),
            layer: SourceLayer::Global,
            fingerprint: "source-v1".to_owned(),
        };
        let context = TargetContext {
            scope: DeploymentScope::Workspace,
            deploy_base: Utf8PathBuf::from("/workspace"),
        };

        let contract = capability_contract_for(PlatformId::Cursor, &asset, &context);

        assert_eq!(contract.consumers, vec![PlatformId::Cursor]);
        assert!(matches!(
            contract.capability,
            PlatformCapability::DirectLink {
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
                ..
            }
        ));
    }

    #[test]
    fn codex_user_mcp_contract_has_no_project_trust_requirement() {
        let asset = EffectiveAsset {
            kind: AssetKind::Mcp,
            name: "catalog".to_owned(),
            source_path: Utf8PathBuf::from("/repo/.ai-config/mcp/servers/catalog.json"),
            layer: SourceLayer::Project,
            fingerprint: "source-v1".to_owned(),
        };
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/user"),
        };

        let contract = capability_contract_for(PlatformId::Codex, &asset, &context);

        assert_eq!(contract.source.layer, SourceLayer::Project);
        assert_eq!(contract.deployment_scope, DeploymentScope::User);
        assert_eq!(contract.trust_requirement, TrustRequirement::None);
        assert_eq!(
            contract.target.as_ref().map(|target| target.path.as_str()),
            Some("/home/user/.codex/config.toml")
        );
    }

    #[test]
    fn codex_command_legacy_prompt_path_is_inventory_only_not_a_target() {
        let asset = EffectiveAsset {
            kind: AssetKind::Command,
            name: "review".to_owned(),
            source_path: Utf8PathBuf::from("/home/user/.ai-config/commands/review.md"),
            layer: SourceLayer::Global,
            fingerprint: "source-v1".to_owned(),
        };
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/user"),
        };

        let contract = capability_contract_for(PlatformId::Codex, &asset, &context);

        assert!(contract.target.is_none());
        assert_eq!(contract.projection_mode, None);
        assert_eq!(contract.format, TargetFormat::None);
        assert_eq!(contract.legacy_inventory_paths.len(), 1);
        assert_eq!(
            contract.legacy_inventory_paths[0].path,
            Utf8PathBuf::from("/home/user/.codex/prompts/review.md")
        );
        assert_eq!(
            contract.legacy_inventory_paths[0].status,
            LegacyInventoryStatus::DeprecatedConsumed
        );
    }

    #[test]
    fn codex_user_skills_use_agents_skills() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/codex-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Skill, "demo", &context),
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/home/codex-user/.agents/skills/demo"),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::SharedTarget {
                    key: "agents_skills".to_owned(),
                },
            }
        );
    }

    #[test]
    fn cursor_and_codex_skills_share_one_physical_target() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Skill, "demo", &context),
            capability_for(PlatformId::Codex, AssetKind::Skill, "demo", &context)
        );
    }

    #[test]
    fn claude_skills_use_its_native_directory() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Skill, "demo", &context),
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.claude/skills/demo"),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn hermes_user_skills_use_external_directory_configuration() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/hermes-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Hermes, AssetKind::Skill, "demo", &context),
            PlatformCapability::ExternalDirectory {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/home/hermes-user/.hermes/config.yaml"),
                    entry_key: Some("skills.external_dirs".to_owned()),
                },
                mode: ProjectionMode::ExternalDirectory,
                surface: ProjectionSurface::Platform(PlatformId::Hermes),
            }
        );
    }

    #[test]
    fn hermes_project_skills_are_unsupported_without_global_target() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert!(matches!(
            capability_for(PlatformId::Hermes, AssetKind::Skill, "demo", &context),
            PlatformCapability::Unsupported { .. }
        ));
    }

    #[test]
    fn codex_instruction_rules_never_target_execution_rules_directory() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Rule, "review", &context),
            PlatformCapability::Unsupported {
                reason: "Codex instruction rules are unsupported; use canonical AGENTS prompt, never .codex/rules".to_owned(),
            }
        );
    }

    #[test]
    fn cursor_user_rules_are_unsupported_without_a_stable_file_target() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/cursor-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Rule, "review", &context),
            PlatformCapability::Unsupported {
                reason: "Cursor user rules have no stable file target".to_owned(),
            }
        );
    }

    #[test]
    fn cursor_project_rules_use_native_mdc_target() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Rule, "review", &context),
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.cursor/rules/review.mdc"),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::SharedTarget {
                    key: "cursor_hermes_rules".to_owned(),
                },
            }
        );
    }

    #[test]
    fn cursor_and_hermes_project_rules_share_one_physical_target() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Rule, "review", &context),
            capability_for(PlatformId::Hermes, AssetKind::Rule, "review", &context)
        );
    }

    #[test]
    fn claude_project_rules_require_generated_markdown() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Rule, "review", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.claude/rules/review.md"),
                    entry_key: None,
                },
                mode: ProjectionMode::GeneratedMarkdown,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn claude_user_rules_require_generated_markdown() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/claude-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Rule, "review", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/home/claude-user/.claude/rules/review.md"),
                    entry_key: None,
                },
                mode: ProjectionMode::GeneratedMarkdown,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn hermes_user_rules_are_unsupported() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/hermes-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Hermes, AssetKind::Rule, "review", &context),
            PlatformCapability::Unsupported {
                reason: "Hermes user rules are unsupported; do not write SOUL.md".to_owned(),
            }
        );
    }

    #[test]
    fn codex_mcp_uses_config_toml() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.codex/config.toml"),
                    entry_key: Some("mcp_servers.catalog".to_owned()),
                },
                mode: ProjectionMode::GeneratedToml,
                surface: ProjectionSurface::Platform(PlatformId::Codex),
            }
        );
    }

    #[test]
    fn cursor_mcp_uses_json_server_entry() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.cursor/mcp.json"),
                    entry_key: Some("mcpServers.catalog".to_owned()),
                },
                mode: ProjectionMode::GeneratedJson,
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            }
        );
    }

    #[test]
    fn claude_user_mcp_uses_claude_json() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/claude-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/home/claude-user/.claude.json"),
                    entry_key: Some("mcpServers.catalog".to_owned()),
                },
                mode: ProjectionMode::GeneratedJson,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn claude_project_mcp_uses_project_dot_mcp_json() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.mcp.json"),
                    entry_key: Some("mcpServers.catalog".to_owned()),
                },
                mode: ProjectionMode::GeneratedJson,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn hermes_user_mcp_uses_yaml_server_entry() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/hermes-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Hermes, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/home/hermes-user/.hermes/config.yaml"),
                    entry_key: Some("mcp_servers.catalog".to_owned()),
                },
                mode: ProjectionMode::GeneratedYaml,
                surface: ProjectionSurface::Platform(PlatformId::Hermes),
            }
        );
    }

    #[test]
    fn hermes_project_mcp_is_unsupported_without_global_configuration() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Hermes, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Unsupported {
                reason: "Hermes project MCP is unsupported; do not modify global config.yaml"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn codex_agents_use_toml_not_subagents() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Agent, "reviewer", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.codex/agents/reviewer.toml"),
                    entry_key: None,
                },
                mode: ProjectionMode::GeneratedToml,
                surface: ProjectionSurface::Platform(PlatformId::Codex),
            }
        );
    }

    #[test]
    fn cursor_agents_require_generated_markdown() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Agent, "reviewer", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.cursor/agents/reviewer.md"),
                    entry_key: None,
                },
                mode: ProjectionMode::GeneratedMarkdown,
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            }
        );
    }

    #[test]
    fn claude_agents_require_generated_markdown() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Agent, "reviewer", &context),
            PlatformCapability::Generated {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.claude/agents/reviewer.md"),
                    entry_key: None,
                },
                mode: ProjectionMode::GeneratedMarkdown,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn hermes_agents_are_unsupported_without_static_directory() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Hermes, AssetKind::Agent, "reviewer", &context),
            PlatformCapability::Unsupported {
                reason: "Hermes has no static agent directory".to_owned(),
            }
        );
    }

    #[test]
    fn codex_commands_are_unsupported_not_codex_commands() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Command, "review", &context),
            PlatformCapability::Unsupported {
                reason: "Codex commands are unsupported; legacy .codex/prompts is inventory only"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn cursor_project_commands_are_direct_links() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Command, "review", &context),
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/repo/.cursor/commands/review.md"),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            }
        );
    }

    #[test]
    fn claude_user_commands_are_direct_links() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/claude-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Command, "review", &context),
            PlatformCapability::DirectLink {
                target: ProjectionTarget {
                    path: Utf8PathBuf::from("/home/claude-user/.claude/commands/review.md"),
                    entry_key: None,
                },
                mode: ProjectionMode::DirectLink,
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            }
        );
    }

    #[test]
    fn hermes_commands_are_unsupported_with_skill_guidance() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Hermes, AssetKind::Command, "review", &context),
            PlatformCapability::Unsupported {
                reason: "Hermes commands are unsupported; use Skills".to_owned(),
            }
        );
    }

    #[test]
    fn cursor_project_hooks_pair_generated_binding_with_linked_script_unit() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            hook_capability_for(PlatformId::Cursor, "format.sh", &context),
            HookCapability::Supported {
                binding: PlatformCapability::Generated {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/repo/.cursor/hooks.json"),
                        entry_key: Some("hooks.format.sh".to_owned()),
                    },
                    mode: ProjectionMode::GeneratedJson,
                    surface: ProjectionSurface::Platform(PlatformId::Cursor),
                },
                script: PlatformCapability::DirectLink {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/repo/.cursor/hooks/format.sh"),
                        entry_key: None,
                    },
                    mode: ProjectionMode::DirectLink,
                    surface: ProjectionSurface::Platform(PlatformId::Cursor),
                },
            }
        );
    }

    #[test]
    fn codex_hooks_use_json_binding_and_linked_script_not_inline_toml() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            hook_capability_for(PlatformId::Codex, "format.sh", &context),
            HookCapability::Supported {
                binding: PlatformCapability::Generated {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/repo/.codex/hooks.json"),
                        entry_key: Some("hooks.format.sh".to_owned()),
                    },
                    mode: ProjectionMode::GeneratedJson,
                    surface: ProjectionSurface::Platform(PlatformId::Codex),
                },
                script: PlatformCapability::DirectLink {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/repo/.codex/hooks/format.sh"),
                        entry_key: None,
                    },
                    mode: ProjectionMode::DirectLink,
                    surface: ProjectionSurface::Platform(PlatformId::Codex),
                },
            }
        );
    }

    #[test]
    fn claude_user_hooks_keep_settings_and_script_unit_separate() {
        let context = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/claude-user"),
        };

        assert_eq!(
            hook_capability_for(PlatformId::Claude, "format.sh", &context),
            HookCapability::Supported {
                binding: PlatformCapability::Generated {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/home/claude-user/.claude/settings.json"),
                        entry_key: Some("hooks.format.sh".to_owned()),
                    },
                    mode: ProjectionMode::GeneratedJson,
                    surface: ProjectionSurface::Platform(PlatformId::Claude),
                },
                script: PlatformCapability::DirectLink {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/home/claude-user/.claude/hooks/format.sh"),
                        entry_key: None,
                    },
                    mode: ProjectionMode::DirectLink,
                    surface: ProjectionSurface::Platform(PlatformId::Claude),
                },
            }
        );
    }

    #[test]
    fn hermes_user_hooks_use_yaml_and_project_hooks_are_rejected() {
        let user = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/hermes-user"),
        };
        let project = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            hook_capability_for(PlatformId::Hermes, "format.sh", &user),
            HookCapability::Supported {
                binding: PlatformCapability::Generated {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/home/hermes-user/.hermes/config.yaml"),
                        entry_key: Some("hooks.format.sh".to_owned()),
                    },
                    mode: ProjectionMode::GeneratedYaml,
                    surface: ProjectionSurface::Platform(PlatformId::Hermes),
                },
                script: PlatformCapability::DirectLink {
                    target: ProjectionTarget {
                        path: Utf8PathBuf::from("/home/hermes-user/.hermes/hooks/format.sh"),
                        entry_key: None,
                    },
                    mode: ProjectionMode::DirectLink,
                    surface: ProjectionSurface::Platform(PlatformId::Hermes),
                },
            }
        );
        assert_eq!(
            hook_capability_for(PlatformId::Hermes, "format.sh", &project),
            HookCapability::Unsupported {
                reason: "Hermes project Hook is unsupported; do not modify global config.yaml"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn asset_name_cannot_escape_the_projection_scope() {
        let context = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(
                PlatformId::Cursor,
                AssetKind::Command,
                "../../outside",
                &context
            ),
            PlatformCapability::Unsupported {
                reason: "asset name must be one safe path segment".to_owned(),
            }
        );
        assert_eq!(
            hook_capability_for(PlatformId::Cursor, "../outside.sh", &context),
            HookCapability::Unsupported {
                reason: "asset name must be one safe path segment".to_owned(),
            }
        );
    }

    #[test]
    fn user_scope_mcp_targets_use_each_platforms_documented_container() {
        let cursor = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/cursor-user"),
        };
        let codex = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/codex-user"),
        };
        let claude = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/claude-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Mcp, "catalog", &cursor),
            generated(
                "/home/cursor-user/.cursor/mcp.json",
                "mcpServers.catalog",
                ProjectionMode::GeneratedJson,
                PlatformId::Cursor
            ),
        );
        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Mcp, "catalog", &codex),
            generated(
                "/home/codex-user/.codex/config.toml",
                "mcp_servers.catalog",
                ProjectionMode::GeneratedToml,
                PlatformId::Codex
            ),
        );
        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Mcp, "catalog", &claude),
            generated(
                "/home/claude-user/.claude.json",
                "mcpServers.catalog",
                ProjectionMode::GeneratedJson,
                PlatformId::Claude
            ),
        );
    }

    #[test]
    fn user_scope_agents_use_native_generated_formats() {
        let cursor = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/cursor-user"),
        };
        let codex = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/codex-user"),
        };
        let claude = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/claude-user"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Agent, "reviewer", &cursor),
            generated(
                "/home/cursor-user/.cursor/agents/reviewer.md",
                "",
                ProjectionMode::GeneratedMarkdown,
                PlatformId::Cursor
            ),
        );
        assert_eq!(
            capability_for(PlatformId::Codex, AssetKind::Agent, "reviewer", &codex),
            generated(
                "/home/codex-user/.codex/agents/reviewer.toml",
                "",
                ProjectionMode::GeneratedToml,
                PlatformId::Codex
            ),
        );
        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Agent, "reviewer", &claude),
            generated(
                "/home/claude-user/.claude/agents/reviewer.md",
                "",
                ProjectionMode::GeneratedMarkdown,
                PlatformId::Claude
            ),
        );
    }

    #[test]
    fn cursor_and_claude_commands_cover_both_user_and_project_scopes() {
        let cursor = TargetContext {
            scope: DeploymentScope::User,
            deploy_base: Utf8PathBuf::from("/home/cursor-user"),
        };
        let claude = TargetContext {
            scope: DeploymentScope::Project,
            deploy_base: Utf8PathBuf::from("/repo"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Command, "review", &cursor),
            direct(
                "/home/cursor-user/.cursor/commands/review.md",
                PlatformId::Cursor
            ),
        );
        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Command, "review", &claude),
            direct("/repo/.claude/commands/review.md", PlatformId::Claude),
        );
    }

    #[test]
    fn workspace_scope_uses_workspace_root_without_falling_back_to_home() {
        let context = TargetContext {
            scope: DeploymentScope::Workspace,
            deploy_base: Utf8PathBuf::from("/workspace"),
        };

        assert_eq!(
            capability_for(PlatformId::Cursor, AssetKind::Rule, "review", &context),
            direct("/workspace/.cursor/rules/review.mdc", PlatformId::Cursor,)
        );
        assert_eq!(
            capability_for(PlatformId::Claude, AssetKind::Mcp, "catalog", &context),
            generated(
                "/workspace/.mcp.json",
                "mcpServers.catalog",
                ProjectionMode::GeneratedJson,
                PlatformId::Claude,
            ),
        );
        assert!(matches!(
            capability_for(PlatformId::Hermes, AssetKind::Mcp, "catalog", &context),
            PlatformCapability::Unsupported { .. }
        ));
        assert!(matches!(
            hook_capability_for(PlatformId::Hermes, "format.sh", &context),
            HookCapability::Unsupported { .. }
        ));
    }

    fn direct(path: &str, platform: PlatformId) -> PlatformCapability {
        PlatformCapability::DirectLink {
            target: ProjectionTarget {
                path: Utf8PathBuf::from(path),
                entry_key: None,
            },
            mode: ProjectionMode::DirectLink,
            surface: ProjectionSurface::Platform(platform),
        }
    }

    fn generated(
        path: &str,
        entry_key: &str,
        mode: ProjectionMode,
        platform: PlatformId,
    ) -> PlatformCapability {
        PlatformCapability::Generated {
            target: ProjectionTarget {
                path: Utf8PathBuf::from(path),
                entry_key: (!entry_key.is_empty()).then(|| entry_key.to_owned()),
            },
            mode,
            surface: ProjectionSurface::Platform(platform),
        }
    }
}
