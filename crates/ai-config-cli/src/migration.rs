//! Read-only migration inventory CLI wiring.

use std::process::ExitCode;

use ai_config_core::paths;
use ai_config_core::projection::migration::{inventory, InventoryRequest};
use camino::Utf8Path;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

pub fn run_inventory(mode: OutputMode, default_root: &Utf8Path) -> ExitCode {
    let roots = paths::resolve_sync_roots(default_root);
    let request = InventoryRequest {
        asset_root: roots.asset_root,
        deploy_base: roots.deploy_base,
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
