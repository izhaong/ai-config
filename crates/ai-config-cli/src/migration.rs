//! Read-only migration inventory CLI wiring.

use std::process::ExitCode;

use ai_config_core::paths;
use ai_config_core::projection::migration::{
    inventory, CanonicalLayerRoot, InventoryRequest, InventoryScope,
};
use ai_config_core::projection::model::SourceLayer;
use camino::Utf8Path;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

pub fn run_inventory(mode: OutputMode, default_root: &Utf8Path) -> ExitCode {
    let roots = paths::resolve_sync_roots(default_root);
    let project_scope = paths::is_project_deploy_base(&roots.deploy_base);
    let (canonical_layers, scope) = if project_scope {
        let canonical_layers = if roots.global_default == roots.asset_root {
            vec![CanonicalLayerRoot {
                layer: SourceLayer::Project,
                asset_root: roots.asset_root,
            }]
        } else {
            vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: roots.global_default,
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Project,
                    asset_root: roots.asset_root,
                },
            ]
        };
        (canonical_layers, InventoryScope::Project)
    } else {
        (
            vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: roots.asset_root,
            }],
            InventoryScope::Global,
        )
    };
    let request = InventoryRequest {
        canonical_layers,
        deploy_base: roots.deploy_base,
        scope,
    };

    match inventory(&request) {
        Ok(report) => {
            if mode.is_json() {
                emit_json(mode, &report);
            } else if mode.is_human() {
                let blocking = report.entries.iter().filter(|entry| entry.blocking).count();
                emit_line(
                    mode,
                    format!(
                        "migration inventory: {} entries, {blocking} blocking, digest {}",
                        report.entries.len(),
                        report.plan_digest
                    ),
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = error.exit_code();
            emit_error_envelope(mode, code, &error.to_string(), error.hint());
            ExitCode::from(code)
        }
    }
}
