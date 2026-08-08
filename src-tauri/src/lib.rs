mod audio;

use std::sync::Mutex;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

const MENU_OPEN: &str = "open";
const MENU_REFRESH: &str = "refresh";
const MENU_QUIT: &str = "quit";
const DEVICE_PREFIX: &str = "device::";
const TRAY_ID: &str = "main";

/// Serializes audio switches so interleaved `SetDefaultEndpoint` calls cannot
/// leave the three roles pointing at different devices.
struct AudioState(Mutex<()>);

#[tauri::command]
fn list_devices() -> Result<Vec<audio::AudioDevice>, String> {
    audio::list_devices()
}

#[tauri::command]
fn set_default(state: tauri::State<'_, AudioState>, device_id: String) -> Result<(), String> {
    let _guard = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    audio::set_default(&device_id)
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    let Ok(menu) = Menu::new(app) else {
        return;
    };
    let Ok(open) = MenuItem::with_id(app, MENU_OPEN, "Open AudioSwitch", true, None::<&str>)
    else {
        return;
    };
    let Ok(separator_a) = PredefinedMenuItem::separator(app) else {
        return;
    };
    let Ok(devices_sub) = Submenu::with_id(app, "devices", "Devices", true) else {
        return;
    };
    let Ok(separator_b) = PredefinedMenuItem::separator(app) else {
        return;
    };
    let Ok(refresh) = MenuItem::with_id(app, MENU_REFRESH, "Refresh devices", true, None::<&str>)
    else {
        return;
    };
    let Ok(quit) = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>) else {
        return;
    };

    let devices = audio::list_devices().unwrap_or_default();
    let devices: Vec<_> = devices
        .into_iter()
        .filter(|d| d.state != audio::DeviceState::Disabled)
        .collect();
    if devices.is_empty() {
        if let Ok(placeholder) = MenuItem::with_id(
            app,
            "no-devices",
            "No playback devices found",
            false,
            None::<&str>,
        ) {
            let _ = devices_sub.append(&placeholder);
        }
    } else {
        for device in &devices {
            let label = if device.state == audio::DeviceState::Active {
                device.name.clone()
            } else {
                format!("{} ({})", device.name, device.state.as_str())
            };
            let id = format!("{DEVICE_PREFIX}{}", device.id);
            if let Ok(item) =
                CheckMenuItem::with_id(app, id, label, true, device.is_default(), None::<&str>)
            {
                let _ = devices_sub.append(&item);
            }
        }
    }

    let _ = menu.append(&open);
    let _ = menu.append(&separator_a);
    let _ = menu.append(&devices_sub);
    let _ = menu.append(&separator_b);
    let _ = menu.append(&refresh);
    let _ = menu.append(&quit);

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some(device_id) = id.strip_prefix(DEVICE_PREFIX) {
        {
            let state = app.state::<AudioState>();
            let _guard = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = audio::set_default(device_id);
        }
        rebuild_tray_menu(app);
        return;
    }
    match id {
        MENU_OPEN => show_main_window(app),
        MENU_REFRESH => rebuild_tray_menu(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(AudioState(Mutex::new(())));

            let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("AudioSwitch")
                .on_menu_event(handle_menu_event)
                .on_tray_icon_event(|tray, event| {
                    match event {
                        // Rebuild the menu on every click so the device list is
                        // fresh when the menu is shown.
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => rebuild_tray_menu(tray.app_handle()),
                        TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } => show_main_window(tray.app_handle()),
                        _ => {}
                    }
                });

            if let Ok(initial) = Menu::new(app) {
                builder = builder.menu(&initial);
            }
            builder.build(app)?;

            rebuild_tray_menu(app.app_handle());
            Ok(())
        })
        // Closing the window keeps the tray app alive; quit via the tray menu.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![list_devices, set_default])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
