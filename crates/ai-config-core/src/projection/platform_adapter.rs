//! Source-first projection 的纯平台 target adapter。
//!
//! 本模块只声明目标与能力；目录创建、配置写入、secret 解析均属于后续 planner/executor。

use camino::Utf8PathBuf;

use crate::model::{AssetKind, PlatformId};
use crate::projection::model::{
    DeploymentScope, ProjectionMode, ProjectionSurface, ProjectionTarget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetContext {
    pub scope: DeploymentScope,
    /// User scope 为 home；workspace/project scope 为对应 deploy root。
    pub deploy_base: Utf8PathBuf,
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

/// 返回一个资产的 projection 能力；不创建目录，也不读取现存配置。
pub fn capability_for(
    platform: PlatformId,
    kind: AssetKind,
    name: &str,
    context: &TargetContext,
) -> PlatformCapability {
    match (platform, kind) {
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
        (PlatformId::Claude, AssetKind::Mcp) if context.scope == DeploymentScope::Project => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetKind, PlatformId};
    use crate::projection::model::{DeploymentScope, ProjectionMode, ProjectionSurface};
    use camino::Utf8PathBuf;

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
}
