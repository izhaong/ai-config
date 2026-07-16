//! Canonical source 的只读三层 overlay resolver。
//!
//! 本模块故意不复用旧 `source::ScanResult`：旧模型把 MCP 当整份 `mcp.json`，且没有
//! Prompt 的 canonical source。这里仅处理新 projection source，legacy monolith 留给
//! migration inventory。

use std::collections::BTreeMap;
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::CoreError;
use crate::model::AssetKind;
use crate::projection::fingerprint::path_content_digest;
use crate::projection::model::{EffectiveAsset, SourceLayer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRoots {
    pub global: Utf8PathBuf,
    pub workspace: Option<Utf8PathBuf>,
    pub project: Utf8PathBuf,
}

pub fn resolve_effective_assets(roots: &OverlayRoots) -> Result<Vec<EffectiveAsset>, CoreError> {
    let mut resolved = BTreeMap::new();
    for (root, layer) in [
        (Some(&roots.global), SourceLayer::Global),
        (roots.workspace.as_ref(), SourceLayer::Workspace),
        (Some(&roots.project), SourceLayer::Project),
    ] {
        let Some(root) = root else {
            continue;
        };
        for asset in scan_layer(root, layer)? {
            resolved.insert(asset_sort_key(&asset), asset);
        }
    }
    Ok(resolved.into_values().collect())
}

fn scan_layer(root: &Utf8Path, layer: SourceLayer) -> Result<Vec<EffectiveAsset>, CoreError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    scan_directories(
        root,
        "skills",
        AssetKind::Skill,
        "SKILL.md",
        layer,
        &mut out,
    )?;
    scan_files(
        root,
        "rules",
        AssetKind::Rule,
        &["mdc", "md"],
        layer,
        &mut out,
    )?;
    scan_files(
        root,
        "mcp/servers",
        AssetKind::Mcp,
        &["json"],
        layer,
        &mut out,
    )?;
    scan_agents(root, layer, &mut out)?;
    scan_files(
        root,
        "commands",
        AssetKind::Command,
        &["md"],
        layer,
        &mut out,
    )?;
    scan_prompt(root, layer, &mut out)?;
    scan_hooks(root, layer, &mut out)?;
    Ok(out)
}

fn scan_directories(
    root: &Utf8Path,
    relative_dir: &str,
    kind: AssetKind,
    required_file: &str,
    layer: SourceLayer,
    out: &mut Vec<EffectiveAsset>,
) -> Result<(), CoreError> {
    let dir = root.join(relative_dir);
    for path in direct_entries(&dir)? {
        if path.is_dir() && path.join(required_file).is_file() {
            add_asset(out, kind, entry_name(&path)?, path, layer)?;
        }
    }
    Ok(())
}

fn scan_files(
    root: &Utf8Path,
    relative_dir: &str,
    kind: AssetKind,
    extensions: &[&str],
    layer: SourceLayer,
    out: &mut Vec<EffectiveAsset>,
) -> Result<(), CoreError> {
    let dir = root.join(relative_dir);
    for path in direct_entries(&dir)? {
        let extension = path.extension().unwrap_or_default();
        if path.is_file() && extensions.contains(&extension) {
            let name = path
                .file_stem()
                .filter(|name| is_asset_name(name))
                .ok_or_else(|| {
                    CoreError::InvalidPath(format!("invalid source asset name: {path}"))
                })?;
            add_asset(out, kind, name.to_owned(), path, layer)?;
        }
    }
    Ok(())
}

fn scan_agents(
    root: &Utf8Path,
    layer: SourceLayer,
    out: &mut Vec<EffectiveAsset>,
) -> Result<(), CoreError> {
    let dir = root.join("agents");
    for path in direct_entries(&dir)? {
        if path.is_dir() {
            add_asset(out, AssetKind::Agent, entry_name(&path)?, path, layer)?;
        } else if path.is_file() && matches!(path.extension(), Some("md" | "yaml" | "yml" | "json"))
        {
            let name = path
                .file_stem()
                .filter(|name| is_asset_name(name))
                .ok_or_else(|| {
                    CoreError::InvalidPath(format!("invalid agent asset name: {path}"))
                })?;
            add_asset(out, AssetKind::Agent, name.to_owned(), path, layer)?;
        }
    }
    Ok(())
}

fn scan_prompt(
    root: &Utf8Path,
    layer: SourceLayer,
    out: &mut Vec<EffectiveAsset>,
) -> Result<(), CoreError> {
    let prompt = root.join("prompts/AGENTS.md");
    if prompt.is_file() {
        add_asset(out, AssetKind::Prompt, "AGENTS".to_owned(), prompt, layer)?;
    }
    Ok(())
}

fn scan_hooks(
    root: &Utf8Path,
    layer: SourceLayer,
    out: &mut Vec<EffectiveAsset>,
) -> Result<(), CoreError> {
    for path in direct_entries(&root.join("hooks"))? {
        if path.is_file() || path.is_dir() {
            add_asset(out, AssetKind::Hook, entry_name(&path)?, path, layer)?;
        }
    }
    Ok(())
}

fn direct_entries(dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir.as_std_path()).map_err(CoreError::Io)? {
        let entry = entry.map_err(CoreError::Io)?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
        if path.file_name().is_some_and(is_asset_name) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn entry_name(path: &Utf8Path) -> Result<String, CoreError> {
    path.file_name()
        .filter(|name| is_asset_name(name))
        .map(str::to_owned)
        .ok_or_else(|| CoreError::InvalidPath(format!("invalid source asset name: {path}")))
}

fn is_asset_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name != "README" && name != "README.md"
}

fn add_asset(
    out: &mut Vec<EffectiveAsset>,
    kind: AssetKind,
    name: String,
    source_path: Utf8PathBuf,
    layer: SourceLayer,
) -> Result<(), CoreError> {
    out.push(EffectiveAsset {
        kind,
        name,
        fingerprint: path_content_digest(&source_path)?,
        source_path,
        layer,
    });
    Ok(())
}

fn asset_sort_key(asset: &EffectiveAsset) -> (u8, String) {
    let order = match asset.kind {
        AssetKind::Skill => 0,
        AssetKind::Rule => 1,
        AssetKind::Mcp => 2,
        AssetKind::Agent => 3,
        AssetKind::Command => 4,
        AssetKind::Prompt => 5,
        AssetKind::Hook => 6,
    };
    (order, asset.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AssetKind;
    use crate::projection::model::SourceLayer;
    use camino::Utf8Path;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Utf8Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        fs::write(path.as_std_path(), content).unwrap();
    }

    fn roots(tmp: &TempDir) -> OverlayRoots {
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        OverlayRoots {
            global: root.join("global"),
            workspace: Some(root.join("workspace")),
            project: root.join("project"),
        }
    }

    #[test]
    fn overlay_keeps_provenance_and_uses_global_workspace_project_precedence() {
        let tmp = TempDir::new().unwrap();
        let roots = roots(&tmp);
        write(&roots.global.join("skills/shared/SKILL.md"), "global");
        write(&roots.global.join("skills/global-only/SKILL.md"), "global");
        write(
            &roots
                .workspace
                .as_ref()
                .unwrap()
                .join("skills/shared/SKILL.md"),
            "workspace",
        );
        write(
            &roots
                .workspace
                .as_ref()
                .unwrap()
                .join("skills/workspace-only/SKILL.md"),
            "workspace",
        );
        write(&roots.project.join("skills/shared/SKILL.md"), "project");
        write(
            &roots.project.join("skills/project-only/SKILL.md"),
            "project",
        );

        let assets = resolve_effective_assets(&roots).unwrap();

        let locate = |name: &str| {
            assets
                .iter()
                .find(|asset| asset.kind == AssetKind::Skill && asset.name == name)
                .unwrap()
        };
        assert_eq!(locate("global-only").layer, SourceLayer::Global);
        assert_eq!(locate("workspace-only").layer, SourceLayer::Workspace);
        assert_eq!(locate("project-only").layer, SourceLayer::Project);
        assert_eq!(locate("shared").layer, SourceLayer::Project);
        assert!(locate("shared").source_path.starts_with(&roots.project));
    }

    #[test]
    fn overlay_overrides_mcp_per_server_and_prompt_and_hook_independently() {
        let tmp = TempDir::new().unwrap();
        let roots = roots(&tmp);
        write(
            &roots.global.join("mcp/servers/catalog.json"),
            "{\"version\":1}",
        );
        write(
            &roots
                .workspace
                .as_ref()
                .unwrap()
                .join("mcp/servers/catalog.json"),
            "{\"version\":2}",
        );
        write(
            &roots.project.join("mcp/servers/project.json"),
            "{\"version\":3}",
        );
        write(&roots.global.join("prompts/AGENTS.md"), "global prompt");
        write(&roots.project.join("prompts/AGENTS.md"), "project prompt");
        write(&roots.global.join("hooks/check/run.sh"), "global hook");
        write(&roots.project.join("hooks/check/run.sh"), "project hook");

        let assets = resolve_effective_assets(&roots).unwrap();

        let locate = |kind: AssetKind, name: &str| {
            assets
                .iter()
                .find(|asset| asset.kind == kind && asset.name == name)
                .unwrap()
        };
        assert_eq!(
            locate(AssetKind::Mcp, "catalog").layer,
            SourceLayer::Workspace
        );
        assert_eq!(
            locate(AssetKind::Mcp, "project").layer,
            SourceLayer::Project
        );
        assert_eq!(
            locate(AssetKind::Prompt, "AGENTS").layer,
            SourceLayer::Project
        );
        assert_eq!(locate(AssetKind::Hook, "check").layer, SourceLayer::Project);
    }

    #[test]
    fn overlay_merges_rules_agents_and_commands_without_losing_unrelated_workspace_assets() {
        let tmp = TempDir::new().unwrap();
        let roots = roots(&tmp);
        write(&roots.global.join("rules/shared.mdc"), "global rule");
        write(
            &roots.workspace.as_ref().unwrap().join("rules/shared.mdc"),
            "workspace rule",
        );
        write(
            &roots
                .workspace
                .as_ref()
                .unwrap()
                .join("rules/workspace.mdc"),
            "workspace rule",
        );
        write(&roots.project.join("agents/reviewer.md"), "project agent");
        write(
            &roots
                .workspace
                .as_ref()
                .unwrap()
                .join("agents/researcher.md"),
            "workspace agent",
        );
        write(&roots.global.join("commands/check.md"), "global command");
        write(&roots.project.join("commands/check.md"), "project command");

        let assets = resolve_effective_assets(&roots).unwrap();
        let locate = |kind: AssetKind, name: &str| {
            assets
                .iter()
                .find(|asset| asset.kind == kind && asset.name == name)
                .unwrap()
        };

        assert_eq!(
            locate(AssetKind::Rule, "shared").layer,
            SourceLayer::Workspace
        );
        assert_eq!(
            locate(AssetKind::Rule, "workspace").layer,
            SourceLayer::Workspace
        );
        assert_eq!(
            locate(AssetKind::Agent, "researcher").layer,
            SourceLayer::Workspace
        );
        assert_eq!(
            locate(AssetKind::Agent, "reviewer").layer,
            SourceLayer::Project
        );
        assert_eq!(
            locate(AssetKind::Command, "check").layer,
            SourceLayer::Project
        );
    }

    #[test]
    fn resolver_is_read_only_across_repeated_runs() {
        let tmp = TempDir::new().unwrap();
        let roots = roots(&tmp);
        write(&roots.global.join("skills/demo/SKILL.md"), "demo");
        let before = crate::projection::fingerprint::directory_digest(
            Utf8Path::from_path(tmp.path()).unwrap(),
        )
        .unwrap();

        for _ in 0..100 {
            resolve_effective_assets(&roots).unwrap();
        }

        let after = crate::projection::fingerprint::directory_digest(
            Utf8Path::from_path(tmp.path()).unwrap(),
        )
        .unwrap();
        assert_eq!(after, before, "resolver must not mutate the source tree");
    }
}
