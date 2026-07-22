pub mod bugreport;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The Havoc-standard update check — the only network the app does.
        // See EULA.md § 5 and HAVOC-STANDARD-bug-report-and-updater.md Part 2.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Needed to relaunch after an update on macOS and Linux. On Windows,
        // NSIS restarts the app itself and this path is never reached.
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            bugreport::bug_report_context,
            bugreport::bug_report_has_pending_crash,
            bugreport::bug_report_submit,
            bugreport::bug_report_clear_crash
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
