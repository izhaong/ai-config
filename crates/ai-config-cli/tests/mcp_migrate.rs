//! 集成测试：legacy MCP migration must remain read-only until source-first apply exists.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = "ai-config";
const FIXTURE_TEMPLATE_REL: &str = "tests/fixtures/mcp/cursor.mcp.template.json";
const ASSET_TEMPLATE_REL: &str = "mcp/cursor.mcp.template.json";

fn setup_with_template() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home tempdir");
    let root = TempDir::new().expect("root tempdir");

    let src_template = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_TEMPLATE_REL);
    let dst_template = root.path().join(ASSET_TEMPLATE_REL);
    fs::create_dir_all(dst_template.parent().unwrap()).expect("mkdir mcp/");
    fs::copy(&src_template, &dst_template).expect("copy template fixture");
    fs::create_dir_all(root.path().join("skills/_fixture")).expect("mkdir skills fixture");

    (home, root)
}

fn cmd(home: &Path, root: &Path) -> Command {
    let mut c = Command::cargo_bin(BIN).expect("binary");
    c.env("HOME", home);
    c.env("USERPROFILE", home);
    c.env_remove("AI_CONFIG_SECRETS_DIR");
    c.env_remove("HERMES_SKILLS_DIR");
    c.arg("--root").arg(root);
    c
}

#[test]
fn mcp_migrate_dry_run_creates_no_files() {
    let (home, root) = setup_with_template();
    let mcp_json = root.path().join("mcp.json");

    assert!(!mcp_json.exists(), "fresh: mcp.json must not exist yet");

    let assert = cmd(home.path(), root.path())
        .args(["mcp", "migrate", "--dry-run"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8");
    assert!(
        stdout.contains("dry-run") && stdout.contains("已迁移 3"),
        "expected dry-run report, got: {stdout}"
    );
    assert!(
        !mcp_json.exists(),
        "dry-run must not create mcp.json at {}",
        mcp_json.display()
    );
}

#[test]
fn mcp_migrate_refuses_legacy_write_and_preserves_the_source_tree() {
    let (home, root) = setup_with_template();
    let mcp_json = root.path().join("mcp.json");
    let template = root.path().join(ASSET_TEMPLATE_REL);
    let before = fs::read(&template).expect("read source template before migration");

    let assert = cmd(home.path(), root.path())
        .args(["mcp", "migrate"])
        .assert()
        .failure()
        .code(2);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8");
    assert!(
        stderr.contains("source-first") && stderr.contains("只读"),
        "expected a source-first read-only refusal, got: {stderr}"
    );

    assert!(!mcp_json.exists(), "refusal must not create mcp.json");
    assert!(
        root.path().join("mcp").is_dir(),
        "refusal must not delete the source mcp/ directory"
    );
    assert_eq!(
        fs::read(&template).expect("read source template after migration"),
        before,
        "refusal must not mutate canonical source input"
    );
}
