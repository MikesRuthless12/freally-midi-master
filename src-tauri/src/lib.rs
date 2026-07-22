pub mod bugreport;
pub mod export;
pub mod store;
pub mod tray;

use tauri::utils::config::Color;
use tauri::{Manager, Theme, WindowEvent};

use store::{settings::ThemePreference, Settings};

/// `--color-bg` in src/styles/tokens.css, for each theme.
///
/// Duplicated from the stylesheet on purpose: this has to be known before any
/// stylesheet exists. `background_matches_the_design_tokens` in this file reads
/// tokens.css and fails if the two ever drift.
const DARK_BG: Color = Color(0x0b, 0x0c, 0x10, 255);
const LIGHT_BG: Color = Color(0xfa, 0xfa, 0xfc, 255);

/// Paint the window its own background before the WebView has anything to show.
///
/// Measured on Windows: the window appears ~50 ms after launch and the UI paints
/// ~800 ms later. Almost all of that gap is WebView2 starting, which is not ours
/// to speed up — but what fills the gap is. The default is a white rectangle,
/// which against the dark theme reads as a flash and makes a fast app feel slow.
/// Painting the real background colour makes the wait look like the window is
/// simply already there.
fn paint_window_background(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let dark = match Settings::load().theme {
        ThemePreference::Dark => true,
        ThemePreference::Light => false,
        // No stored choice: ask the OS. Falling back to dark matches tokens.css,
        // which treats dark as the default when nothing says otherwise.
        ThemePreference::System => window.theme().map(|t| t == Theme::Dark).unwrap_or(true),
    };
    let _ = window.set_background_color(Some(if dark { DARK_BG } else { LIGHT_BG }));
}

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
            paint_window_background(app.handle());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The window background is a copy of a value that really lives in CSS.
    /// If they drift, the launch flash comes back — and it is invisible in
    /// review, because both files look correct on their own.
    #[test]
    fn background_matches_the_design_tokens() {
        let css = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("src")
                .join("styles")
                .join("tokens.css"),
        )
        .expect("tokens.css must be readable");

        let hex = |color: Color| format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2);

        let find = |name: &str| {
            css.lines()
                .find_map(|line| line.trim().strip_prefix(name)?.split(';').next())
                .map(|v| v.trim().to_lowercase())
                .unwrap_or_else(|| panic!("{name} not found in tokens.css"))
        };

        assert_eq!(find("--color-bg:"), hex(DARK_BG), "dark background drifted");
        assert_eq!(
            find("--light-bg:"),
            hex(LIGHT_BG),
            "light background drifted"
        );
    }
}
