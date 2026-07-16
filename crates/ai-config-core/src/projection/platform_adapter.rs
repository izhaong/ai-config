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
}
