mod audio;

use std::sync::Mutex;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};
use tauri_plugin_updater::UpdaterExt;

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
    let Ok(separator) = PredefinedMenuItem::separator(app) else {
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
            let id = format!("{DEVICE_PREFIX}{}", device.id);
            if let Ok(item) =
                CheckMenuItem::with_id(app, id, device.name.clone(), true, device.is_default(), None::<&str>)
            {
                let _ = menu.append(&item);
            }
        }
    }

    let _ = menu.append(&separator);
    let _ = menu.append(&refresh);
    let _ = menu.append(&quit);

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some(device_id) = id.strip_prefix(DEVICE_PREFIX) {
        let state = app.state::<AudioState>();
        let _guard = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = audio::set_default(device_id);
        rebuild_tray_menu(app);
        return;
    }
    match id {
        MENU_REFRESH => rebuild_tray_menu(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

/// Check for updates in the background shortly after launch. When one is
/// found, it downloads and installs it, then exits so the new version takes
/// over (the MSI installs in passive mode with no prompts).
fn spawn_update_check(app: AppHandle) {
    let Ok(updater) = app.updater() else {
        eprintln!("AudioSwitch: updater is not configured");
        return;
    };
    tauri::async_runtime::spawn(async move {
        match updater.check().await {
            Ok(Some(update)) => {
                eprintln!("AudioSwitch: update {} available; installing", update.version);
                match update.download_and_install(|_, _| {}, || {}).await {
                    Ok(_) => {
                        eprintln!("AudioSwitch: update installed; exiting to apply");
                        app.exit(0);
                    }
                    Err(err) => eprintln!("AudioSwitch: update install failed: {err}"),
                }
            }
            Ok(None) => eprintln!("AudioSwitch: up to date"),
            Err(err) => eprintln!("AudioSwitch: update check failed: {err}"),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            spawn_update_check(app.handle().clone());
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
