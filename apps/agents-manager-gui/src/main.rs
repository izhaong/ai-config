//! Tauri 2 启动入口(Phase 3 启 GUI 时由 src/main.rs 调用)。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    agents_manager_gui_lib::run();
}
