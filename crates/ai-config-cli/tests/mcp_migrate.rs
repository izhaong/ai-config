//! 集成测试:`mcp migrate` — legacy `mcp/servers/` 或模板 → 单一 `mcp.json`。

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
fn mcp_migrate_writes_servers_into_mcp_json() {
    let (home, root) = setup_with_template();
    let mcp_json = root.path().join("mcp.json");

    let assert = cmd(home.path(), root.path())
        .args(["mcp", "migrate"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8");
    assert!(
        stdout.contains("已迁移 3 个 / 跳过 0 个 / 失败 0 个"),
        "expected 18/0/0 report, got: {stdout}"
    );

    assert!(mcp_json.is_file(), "mcp.json should exist");
    assert!(
        !root.path().join("mcp").exists(),
        "legacy mcp/ directory should be removed"
    );

    let body = fs::read_to_string(&mcp_json).expect("read mcp.json");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let servers = v["mcpServers"].as_object().expect("mcpServers object");
    assert_eq!(servers.len(), 3);

    assert!(servers.contains_key("example-http"));
    let http = servers.get("example-http").expect("example-http entry");
    assert!(http.get("url").is_some());
}
