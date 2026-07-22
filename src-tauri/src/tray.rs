//! The system tray icon and its menu.
//!
//! Only meaningful because of the settings it serves: minimise-to-tray and
//! close-to-tray. Both default to **off** — a window that disappears from the
//! taskbar when the user minimises it is alarming if they never asked for it,
//! and "close" should mean close until told otherwise.

use tauri::{
    menu::{Menu, MenuEvent, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::store::Settings;

const SHOW: &str = "show";
const QUIT: &str = "quit";

/// Bring the main window back and focus it.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Build the tray icon. Returns `Ok(())` when the tray is not wanted.
pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if !Settings::load().show_tray_icon {
        return Ok(());
    }

    let show = MenuItem::with_id(app, SHOW, "Show Freally MIDI Master", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("no default window icon to use for the tray".into())
        })?)
        .tooltip("Freally MIDI Master")
        .menu(&menu)
        // The menu must NOT open on a left click, or the click-to-restore
        // gesture below never fires — the menu swallows it.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event: MenuEvent| match event.id().as_ref() {
            SHOW => show_main_window(app),
            QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left click restores, which is what every tray app does and what a
            // user who just minimised here will try first.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
