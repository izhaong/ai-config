//! 全平台 Hook 生命周期目录与 GUI 切换（canonical 键名并集，按当前浏览平台标记 supported/active）。

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::hook::{self, HookBinding};
use crate::hook_adapter;
use crate::model::PlatformId;

const DEPLOY_PLATFORMS: [PlatformId; 4] = [
    PlatformId::Cursor,
    PlatformId::Codex,
    PlatformId::Claude,
    PlatformId::Hermes,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookLifecycleGroup {
    Agent,
    Tab,
    Workspace,
}

#[derive(Debug, Clone, Copy)]
pub struct LifecycleDef {
    pub id: &'static str,
    pub group: HookLifecycleGroup,
    pub label: &'static str,
    pub short_label: &'static str,
}

/// Cursor 官方生命周期（canonical `hooks.json` 键名，亦作为跨平台统一 ID）。
pub const CURSOR_LIFECYCLES: &[LifecycleDef] = &[
    LifecycleDef {
        id: "sessionStart",
        group: HookLifecycleGroup::Agent,
        label: "sessionStart",
        short_label: "St",
    },
    LifecycleDef {
        id: "sessionEnd",
        group: HookLifecycleGroup::Agent,
        label: "sessionEnd",
        short_label: "En",
    },
    LifecycleDef {
        id: "preToolUse",
        group: HookLifecycleGroup::Agent,
        label: "preToolUse",
        short_label: "pre",
    },
    LifecycleDef {
        id: "postToolUse",
        group: HookLifecycleGroup::Agent,
        label: "postToolUse",
        short_label: "post",
    },
    LifecycleDef {
        id: "postToolUseFailure",
        group: HookLifecycleGroup::Agent,
        label: "postToolUseFailure",
        short_label: "fail",
    },
    LifecycleDef {
        id: "subagentStart",
        group: HookLifecycleGroup::Agent,
        label: "subagentStart",
        short_label: "sub+",
    },
    LifecycleDef {
        id: "subagentStop",
        group: HookLifecycleGroup::Agent,
        label: "subagentStop",
        short_label: "sub-",
    },
    LifecycleDef {
        id: "beforeShellExecution",
        group: HookLifecycleGroup::Agent,
        label: "beforeShellExecution",
        short_label: "bSh",
    },
    LifecycleDef {
        id: "afterShellExecution",
        group: HookLifecycleGroup::Agent,
        label: "afterShellExecution",
        short_label: "aSh",
    },
    LifecycleDef {
        id: "beforeMCPExecution",
        group: HookLifecycleGroup::Agent,
        label: "beforeMCPExecution",
        short_label: "bMC",
    },
    LifecycleDef {
        id: "afterMCPExecution",
        group: HookLifecycleGroup::Agent,
        label: "afterMCPExecution",
        short_label: "aMC",
    },
    LifecycleDef {
        id: "beforeReadFile",
        group: HookLifecycleGroup::Agent,
        label: "beforeReadFile",
        short_label: "bRd",
    },
    LifecycleDef {
        id: "afterFileEdit",
        group: HookLifecycleGroup::Agent,
        label: "afterFileEdit",
        short_label: "aEd",
    },
    LifecycleDef {
        id: "beforeSubmitPrompt",
        group: HookLifecycleGroup::Agent,
        label: "beforeSubmitPrompt",
        short_label: "bPr",
    },
    LifecycleDef {
        id: "preCompact",
        group: HookLifecycleGroup::Agent,
        label: "preCompact",
        short_label: "cmp",
    },
    LifecycleDef {
        id: "stop",
        group: HookLifecycleGroup::Agent,
        label: "stop",
        short_label: "stop",
    },
    LifecycleDef {
        id: "afterAgentResponse",
        group: HookLifecycleGroup::Agent,
        label: "afterAgentResponse",
        short_label: "aRsp",
    },
    LifecycleDef {
        id: "afterAgentThought",
        group: HookLifecycleGroup::Agent,
        label: "afterAgentThought",
        short_label: "aTh",
    },
    LifecycleDef {
        id: "beforeTabFileRead",
        group: HookLifecycleGroup::Tab,
        label: "beforeTabFileRead",
        short_label: "bTab",
    },
    LifecycleDef {
        id: "afterTabFileEdit",
        group: HookLifecycleGroup::Tab,
        label: "afterTabFileEdit",
        short_label: "aTab",
    },
    LifecycleDef {
        id: "workspaceOpen",
        group: HookLifecycleGroup::Workspace,
        label: "workspaceOpen",
        short_label: "ws",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookLifecycleView {
    pub lifecycle: String,
    pub group: HookLifecycleGroup,
    pub label: String,
    pub short_label: String,
    pub active: bool,
    pub supported: bool,
    /// 支持该生命周期的 IDE 平台（并集成员）
    pub supported_platforms: Vec<PlatformId>,
}

/// 各平台支持的生命周期并集（canonical ID 列表，保持 Cursor 目录顺序）。
pub fn lifecycle_catalog() -> &'static [LifecycleDef] {
    CURSOR_LIFECYCLES
}

pub fn lifecycle_supported_on_any_deploy_platform(lifecycle: &str) -> bool {
    DEPLOY_PLATFORMS
        .iter()
        .any(|plat| lifecycle_supported(*plat, lifecycle))
}

pub fn supported_deploy_platforms(lifecycle: &str) -> Vec<PlatformId> {
    DEPLOY_PLATFORMS
        .iter()
        .copied()
        .filter(|plat| lifecycle_supported(*plat, lifecycle))
        .collect()
}

pub fn lifecycle_supported(plat: PlatformId, lifecycle: &str) -> bool {
    match plat {
        PlatformId::AiConfig | PlatformId::Cursor => true,
        PlatformId::Codex | PlatformId::Claude => matches!(
            lifecycle,
            "preToolUse"
                | "postToolUse"
                | "postToolUseFailure"
                | "beforeShellExecution"
                | "afterShellExecution"
        ),
        PlatformId::Hermes => matches!(
            lifecycle,
            "preToolUse" | "postToolUse" | "beforeShellExecution" | "afterShellExecution"
        ),
    }
}

fn command_for_lifecycle(
    script_filename: &str,
    lifecycle: &str,
    bindings: &[HookBinding],
) -> String {
    hook::infer_binding_command(script_filename, lifecycle, bindings)
}

fn source_bindings(asset_root: &Utf8Path, script_filename: &str) -> Vec<HookBinding> {
    hook::load_spec(asset_root, script_filename)
        .map(|s| s.bindings)
        .unwrap_or_default()
}

fn platform_bindings(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
) -> Vec<HookBinding> {
    hook_adapter::read_platform_bindings(deploy_base, plat, script_filename).unwrap_or_default()
}

pub fn list_lifecycle_views(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    browse_plat: PlatformId,
    script_filename: &str,
) -> Vec<HookLifecycleView> {
    let source = source_bindings(asset_root, script_filename);
    let platform = if browse_plat == PlatformId::AiConfig {
        Vec::new()
    } else {
        platform_bindings(deploy_base, browse_plat, script_filename)
    };
    let active_bindings = if browse_plat == PlatformId::AiConfig {
        &source
    } else {
        &platform
    };

    let active_plat = if browse_plat == PlatformId::AiConfig {
        PlatformId::Cursor
    } else {
        browse_plat
    };

    lifecycle_catalog()
        .iter()
        .filter(|def| {
            browse_plat == PlatformId::AiConfig
                || lifecycle_supported_on_any_deploy_platform(def.id)
        })
        .map(|def| {
            let active = active_bindings.iter().any(|b| {
                hook::canonical_lifecycle_id(&b.lifecycle, b.matcher.as_deref(), active_plat)
                    == def.id
            });
            let supported_platforms = supported_deploy_platforms(def.id);
            HookLifecycleView {
                lifecycle: def.id.to_string(),
                group: def.group,
                label: def.label.to_string(),
                short_label: def.short_label.to_string(),
                active,
                supported: lifecycle_supported(browse_plat, def.id),
                supported_platforms,
            }
        })
        .collect()
}

pub fn toggle_lifecycle(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    browse_plat: PlatformId,
    script_filename: &str,
    lifecycle: &str,
    enabled: bool,
) -> Result<(), CoreError> {
    if !lifecycle_supported(browse_plat, lifecycle) {
        return Err(CoreError::UnsupportedAsset {
            platform: browse_plat,
            asset: crate::model::AssetKind::Hook,
            hint: format!("平台不支持生命周期 `{lifecycle}`"),
        });
    }
    if browse_plat == PlatformId::AiConfig {
        toggle_source_lifecycle(asset_root, script_filename, lifecycle, enabled)
    } else {
        hook_adapter::toggle_platform_lifecycle(
            asset_root,
            deploy_base,
            browse_plat,
            script_filename,
            lifecycle,
            enabled,
        )
    }
}

fn toggle_source_lifecycle(
    asset_root: &Utf8Path,
    script_filename: &str,
    lifecycle: &str,
    enabled: bool,
) -> Result<(), CoreError> {
    if !hook::script_path(asset_root, script_filename).is_file() {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Hook,
            name: script_filename.into(),
            hint: format!("缺少脚本 hooks/{script_filename}"),
        });
    }
    if enabled {
        let bindings = source_bindings(asset_root, script_filename);
        let binding = HookBinding {
            lifecycle: lifecycle.to_string(),
            matcher: bindings
                .iter()
                .find(|b| b.lifecycle == lifecycle)
                .and_then(|b| b.matcher.clone()),
            command: command_for_lifecycle(script_filename, lifecycle, &bindings),
        };
        hook::merge_bindings_into_manifest(asset_root, script_filename, &[binding])
    } else {
        hook::remove_binding_for_script_lifecycle(asset_root, script_filename, lifecycle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn list_views_reflects_source_bindings() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("hooks")).unwrap();
        fs::write(
            root.join("hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./hooks/demo.py sessionStart"}]}}"#,
        )
        .unwrap();
        fs::write(root.join("hooks/demo.py"), "#!/usr/bin/env python3\n").unwrap();
        let views = list_lifecycle_views(&root, &root, PlatformId::AiConfig, "demo.py");
        let st = views
            .iter()
            .find(|v| v.lifecycle == "sessionStart")
            .unwrap();
        assert!(st.active);
        assert!(st.supported);
        assert!(st.supported_platforms.contains(&PlatformId::Cursor));
        let stop = views.iter().find(|v| v.lifecycle == "stop").unwrap();
        assert!(!stop.active);
    }

    #[test]
    fn codex_post_tool_use_maps_to_canonical() {
        use crate::hook::canonical_lifecycle_id;
        assert_eq!(
            canonical_lifecycle_id("PostToolUse", None, PlatformId::Codex),
            "postToolUse"
        );
        assert_eq!(
            canonical_lifecycle_id("PostToolUse", Some("Bash"), PlatformId::Codex),
            "afterShellExecution"
        );
        assert_eq!(
            canonical_lifecycle_id("pre_tool_call", None, PlatformId::Hermes),
            "beforeShellExecution"
        );
    }

    #[test]
    fn codex_list_views_detects_bash_post_tool_use_as_shell_after() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let codex_hooks = root.join(".codex/hooks");
        fs::create_dir_all(&codex_hooks).unwrap();
        fs::create_dir_all(root.join("hooks")).unwrap();
        let script = codex_hooks.join("demo.py");
        fs::write(&script, "#!/usr/bin/env python3\n").unwrap();
        fs::write(root.join("hooks/demo.py"), "#!/usr/bin/env python3\n").unwrap();
        fs::write(root.join("hooks.json"), r#"{"version":1,"hooks":{}}"#).unwrap();
        let cmd = script.as_str();
        fs::write(
            root.join(".codex/hooks.json"),
            format!(
                r#"{{
  "hooks": {{
    "PostToolUse": [{{
      "matcher": "Bash",
      "hooks": [{{
        "type": "command",
        "command": "{cmd}",
        "managedBy": "ai-config",
        "hook": "demo.py"
      }}]
    }}]
  }}
}}"#
            ),
        )
        .unwrap();

        let views = list_lifecycle_views(&root, &root, PlatformId::Codex, "demo.py");
        let shell_after = views
            .iter()
            .find(|v| v.lifecycle == "afterShellExecution")
            .unwrap();
        assert!(shell_after.active);
        assert!(shell_after.supported);
        let tool_post = views.iter().find(|v| v.lifecycle == "postToolUse").unwrap();
        assert!(!tool_post.active);
        let session = views
            .iter()
            .find(|v| v.lifecycle == "sessionStart")
            .unwrap();
        assert!(!session.supported);
        assert!(session.supported_platforms.contains(&PlatformId::Cursor));
    }

    #[test]
    fn toggle_source_lifecycle_updates_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("hooks")).unwrap();
        fs::write(root.join("hooks.json"), r#"{"version":1,"hooks":{}}"#).unwrap();
        fs::write(root.join("hooks/demo.py"), "#!/usr/bin/env python3\n").unwrap();

        toggle_lifecycle(&root, &root, PlatformId::AiConfig, "demo.py", "stop", true).unwrap();

        let views = list_lifecycle_views(&root, &root, PlatformId::AiConfig, "demo.py");
        let stop = views.iter().find(|v| v.lifecycle == "stop").unwrap();
        assert!(stop.active);

        toggle_lifecycle(&root, &root, PlatformId::AiConfig, "demo.py", "stop", false).unwrap();
        let views = list_lifecycle_views(&root, &root, PlatformId::AiConfig, "demo.py");
        let stop = views.iter().find(|v| v.lifecycle == "stop").unwrap();
        assert!(!stop.active);
    }

    #[test]
    fn lifecycle_union_includes_cursor_only_on_cursor_platform() {
        let session = supported_deploy_platforms("sessionStart");
        assert_eq!(session, vec![PlatformId::Cursor]);
        let shell = supported_deploy_platforms("beforeShellExecution");
        assert!(shell.contains(&PlatformId::Cursor));
        assert!(shell.contains(&PlatformId::Codex));
        assert!(shell.contains(&PlatformId::Hermes));
    }

    #[test]
    fn cursor_toggle_platform_lifecycle_updates_platform_views() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let asset_root = root.join(".ai-config");
        fs::create_dir_all(asset_root.join("hooks")).unwrap();
        fs::write(
            asset_root.join("hooks.json"),
            r#"{"version":1,"hooks":{"beforeTabFileRead":[{"command":"./hooks/demo.py beforeTabFileRead"}]}}"#,
        )
        .unwrap();
        fs::write(asset_root.join("hooks/demo.py"), "#!/usr/bin/env python3\n").unwrap();

        // project-scope cursor deploy base
        fs::create_dir_all(root.join(".cursor/hooks")).unwrap();
        fs::write(
            root.join(".cursor/hooks/demo.py"),
            "#!/usr/bin/env python3\n",
        )
        .unwrap();
        fs::write(
            root.join(".cursor/hooks.json"),
            r#"{"version":1,"hooks":{"beforeTabFileRead":[{"command":".cursor/hooks/demo.py beforeTabFileRead"}]}}"#,
        )
        .unwrap();

        let before = list_lifecycle_views(&asset_root, &root, PlatformId::Cursor, "demo.py");
        assert!(before
            .iter()
            .any(|v| v.lifecycle == "beforeTabFileRead" && v.active));

        toggle_lifecycle(
            &asset_root,
            &root,
            PlatformId::Cursor,
            "demo.py",
            "beforeTabFileRead",
            false,
        )
        .unwrap();

        let after = list_lifecycle_views(&asset_root, &root, PlatformId::Cursor, "demo.py");
        assert!(after
            .iter()
            .any(|v| v.lifecycle == "beforeTabFileRead" && !v.active));
    }
}
