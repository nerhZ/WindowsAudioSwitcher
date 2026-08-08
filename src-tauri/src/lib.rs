mod audio;

use std::sync::Mutex;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder,
};

const MENU_OPEN: &str = "open";
const MENU_REFRESH: &str = "refresh";
const MENU_QUIT: &str = "quit";
const DEVICE_PREFIX: &str = "device::";
const TRAY_ID: &str = "main";
const MAIN_WINDOW: &str = "main";

/// Serializes audio switches so interleaved `SetDefaultEndpoint` calls cannot
/// leave the three roles pointing at different devices.
struct AudioState(Mutex<()>);

#[tauri::command]
fn list_devices() -> Result<Vec<audio::AudioDevice>, String> {
    audio::list_devices()
}

#[tauri::command]
fn set_default(
    state: tauri::State<'_, AudioState>,
    device_id: String,
    roles: Vec<audio::Role>,
) -> Result<(), String> {
    let _guard = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    audio::set_default(&device_id, &roles)
}

/// The dashboard window is created on demand and fully destroyed on close, so
/// the WebView2 runtime only exists while the dashboard is open.
fn open_dashboard<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("index.html".into()))
        .title("AudioSwitch")
        .inner_size(460.0, 620.0)
        .build();
    if let Err(err) = result {
        eprintln!("failed to open dashboard window: {err}");
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

    let _ = menu.append(&open);
    let _ = menu.append(&separator_a);

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
            let _ = menu.append(&placeholder);
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
                let _ = menu.append(&item);
            }
        }
    }

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
            let _ = audio::set_default(device_id, &audio::ALL_ROLES);
        }
        rebuild_tray_menu(app);
        return;
    }
    match id {
        MENU_OPEN => open_dashboard(app),
        MENU_REFRESH => rebuild_tray_menu(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            app.manage(AudioState(Mutex::new(())));

            let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("AudioSwitch")
                // Menu on right click only — a left click opens the dashboard.
                .show_menu_on_left_click(false)
                .on_menu_event(handle_menu_event)
                .on_tray_icon_event(|tray, event| {
                    // Never rebuild the menu from tray icon events: replacing
                    // the HMENU while Windows is about to track the popup makes
                    // it flash open and close. The menu is built at startup and
                    // refreshed only from menu events (Refresh devices, device
                    // switches) — moments when no popup is being displayed.
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        open_dashboard(tray.app_handle());
                    }
                });

            if let Ok(initial) = Menu::new(app) {
                builder = builder.menu(&initial);
            }
            builder.build(app)?;

            rebuild_tray_menu(app.app_handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![list_devices, set_default])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Default close destroys the dashboard window (freeing the WebView2
    // runtime). That makes the event loop request an exit with no code;
    // prevent it so the tray app keeps running. The Quit menu item exits
    // with a code, which is allowed through.
    app.run(|_, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
