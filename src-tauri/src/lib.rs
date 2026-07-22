pub mod bugreport;
pub mod export;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The Havoc-standard update check — the only network the app does.
        // See EULA.md § 5 and HAVOC-STANDARD-bug-report-and-updater.md Part 2.
        // removed
        // Needed to relaunch after an update on macOS and Linux. On Windows,
        // NSIS restarts the app itself and this path is never reached.
        .plugin(tauri_plugin_process::init())
        // Choosing the export folder. The dialog is also what grants the fs
        // scope for writing there — nothing else may pick a path.
        // removed
        // Native drag-out (TASK-013).
        // removed
        .invoke_handler(tauri::generate_handler![
            bugreport::bug_report_context,
            bugreport::bug_report_has_pending_crash,
            bugreport::bug_report_submit,
            bugreport::bug_report_clear_crash,
            export::drag::drag_capability,
            export::drag::export_spike_midi,
            export::drag::drag_source_ready,
            export::drag::pick_export_folder,
            export::drag::export_to_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
