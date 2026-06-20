//! Tauri command 与 `ai-config-core::asset_ops` 之间的薄桥。

use ai_config_core::asset_ops::{self, AssetFileDetail, ScopeRoots};
use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, PlatformId};
use camino::Utf8PathBuf;

pub async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| e.to_string())
}

pub fn scope_roots<'a>(
    default_root: &'a Utf8PathBuf,
    asset_root: &'a Utf8PathBuf,
    deploy_base: &'a Utf8PathBuf,
) -> ScopeRoots<'a> {
    ScopeRoots {
        default_root,
        asset_root,
        deploy_base,
    }
}

pub async fn deploy(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
    plat: PlatformId,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::deploy(&scope, kind, &name, plat)
    })
    .await
}

pub async fn retract(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
    plat: PlatformId,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::retract(&scope, kind, &name, plat)
    })
    .await
}

pub async fn get_detail(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
) -> Result<AssetFileDetail, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::get_detail(&scope, kind, &name)
    })
    .await
}

pub async fn save_content(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
    content: String,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::save_content(&scope, kind, &name, &content)
    })
    .await
}

pub async fn delete_source(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::delete_source(&scope, kind, &name)
    })
    .await
}

pub async fn read_platform_preview(
    kind: AssetKind,
    path: String,
    name: String,
) -> Result<AssetFileDetail, String> {
    let platform_path = camino::Utf8PathBuf::from(path);
    blocking(move || asset_ops::read_platform_preview(kind, &platform_path, &name)).await
}
