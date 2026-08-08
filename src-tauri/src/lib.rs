mod audio;

use std::sync::Mutex;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

const MENU_REFRESH: &str = "refresh";
const MENU_QUIT: &str = "quit";
const DEVICE_PREFIX: &str = "device::";
const TRAY_ID: &str = "main";

/// Serializes audio switches so interleaved `SetDefaultEndpoint` calls cannot
/// leave the three roles pointing at different devices.
struct AudioState(Mutex<()>);

fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    let Ok(menu) = Menu::new(app) else {
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
                .on_menu_event(handle_menu_event);
            // No on_tray_icon_event handler: the native menu opens on both
            // clicks (Windows default), and rebuilding the menu from tray
            // events would replace the HMENU while Windows is tracking the
            // popup, making it flash open and close. The menu is built at
            // startup and refreshed only from menu events (Refresh devices,
            // device switches) — moments when no popup is displayed.

            if let Ok(initial) = Menu::new(app) {
                builder = builder.menu(&initial);
            }
            builder.build(app)?;

            rebuild_tray_menu(app.app_handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // The app lives in the tray; the Quit menu item exits with a code, which
    // is allowed through. Any uncoded exit request is prevented.
    app.run(|_, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
