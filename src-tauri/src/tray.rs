//! Tray icon: d-scribe keeps running (and auto-recording) after its window is closed.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

const TRAY_ID: &str = "main";
/// Passed by the "Start with Windows" entry, so the app starts in the tray.
pub const MINIMIZED_ARG: &str = "--minimized";

pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn build_menu(app: &AppHandle, recording: bool) -> tauri::Result<Menu<tauri::Wry>> {
    let status = MenuItem::with_id(
        app,
        "status",
        if recording { "Recording" } else { "Not recording" },
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", "Open d-scribe", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Menu::with_items(app, &[&status, &PredefinedMenuItem::separator(app)?, &open, &quit])
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("d-scribe")
        .menu(&build_menu(app, false)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => {
                if crate::is_recording() {
                    // Let the window stop and save the recording first; it calls exit_app.
                    show_main_window(app);
                    let _ = app.emit("quit-requested", ());
                } else {
                    app.exit(0);
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Shows whether a recording is running in the tray tooltip and menu.
pub fn set_recording(app: &AppHandle, recording: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let _ = tray.set_tooltip(Some(if recording { "d-scribe: recording" } else { "d-scribe" }));
    if let Ok(menu) = build_menu(app, recording) {
        let _ = tray.set_menu(Some(menu));
    }
}
