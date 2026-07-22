pub mod bugreport;
pub mod export;
pub mod store;
pub mod tray;

use tauri::{Manager, WindowEvent};

use store::Settings;

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
        // Native drag-out (TASK-013). The export-folder picker deliberately
        // does NOT use tauri-plugin-dialog — see export::drag::pick_export_folder.
        .plugin(tauri_plugin_drag::init())
        .setup(|app| {
            tray::sync(app.handle())?;
            Ok(())
        })
        // Both arms below gate on a tray icon that ACTUALLY EXISTS, never on
        // the setting that asks for one. The setting is a wish; the icon is the
        // way back. Hiding the window without one leaves a live process with no
        // window, no taskbar entry and nothing to click — recoverable only from
        // Task Manager.
        .on_window_event(|window, event| match event {
            // Close-to-tray: hide instead of quitting, but only when asked.
            // Reading the setting at the moment of the event rather than
            // caching it means a change in Settings takes effect immediately.
            WindowEvent::CloseRequested { api, .. } => {
                if Settings::load().close_to_tray && tray::exists(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // Minimise-to-tray: let the minimise happen, then hide the window
            // so it leaves the taskbar too.
            WindowEvent::Resized(_)
                if Settings::load().minimize_to_tray
                    && window.is_minimized().unwrap_or(false)
                    && tray::exists(window.app_handle()) =>
            {
                let _ = window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            bugreport::bug_report_context,
            bugreport::bug_report_has_pending_crash,
            bugreport::bug_report_submit,
            bugreport::bug_report_clear_crash,
            bugreport::bug_report_preview,
            export::drag::drag_capability,
            export::drag::export_spike_midi,
            export::drag::drag_source_ready,
            export::drag::pick_export_folder,
            export::drag::export_to_folder,
            store::settings::settings_get,
            store::settings::settings_set,
            app_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Version and platform, for the About panel.
#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    })
}
