//! Repository dogfood guard for the source-first project fixture.
//!
//! This test deliberately inspects tracked paths rather than the working tree so local,
//! generated platform projections can stay available without becoming another source of truth.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn tracked_paths(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["ls-files"])
        .current_dir(root)
        .output()
        .expect("git ls-files must run");
    assert!(output.status.success(), "git ls-files failed");
    String::from_utf8(output.stdout)
        .expect("git paths are UTF-8")
        .lines()
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn repository_tracks_only_canonical_source_assets() {
    let root = repository_root();
    let paths = tracked_paths(&root);

    assert!(
        root.join(".agent-manager/prompts/AGENTS.md").is_file(),
        "canonical project prompt is required"
    );
    assert_eq!(
        std::fs::read_link(root.join("AGENTS.md"))
            .expect("root AGENTS.md must be a tracked source link"),
        Path::new(".agent-manager/prompts/AGENTS.md")
    );
    assert!(
        !paths.iter().any(|path| path == ".agent-manager/mcp.json"),
        "monolithic MCP source is forbidden; use .agent-manager/mcp/servers/<name>.json"
    );

    let forbidden_prefixes = [
        ".cursor/skills/",
        ".codex/skills/",
        ".claude/skills/",
        ".cursor/hooks/",
        ".codex/hooks/",
        ".claude/hooks/",
    ];
    let forbidden_exact = [
        ".cursor/hooks.json",
        ".codex/hooks.json",
        ".claude/settings.json",
    ];
    let offenders = paths
        .iter()
        .filter(|path| {
            forbidden_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix))
                || forbidden_exact.contains(&path.as_str())
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "tracked platform projections must not be canonical copies: {offenders:?}"
    );
}
