#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::ScanState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(ScanState::default())
        .invoke_handler(tauri::generate_handler![
            commands::scan_path,
            commands::cancel_scan,
            commands::delete_paths,
            commands::list_roots,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TidyTrail Desktop");
}
