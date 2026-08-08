mod audio;

use std::sync::Mutex;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_updater::UpdaterExt;

const MENU_AUTOSTART: &str = "autostart";
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
    let Ok(autostart) =
        CheckMenuItem::with_id(app, MENU_AUTOSTART, "Start with Windows", true, false, None::<&str>)
    else {
        return;
    };
    let _ = autostart.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
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

    let _ = menu.append(&autostart);
    let _ = menu.append(&separator);

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
            // '&' starts a menu mnemonic in Win32 menus; escape it so device
            // names render literally.
            let label = device.name.replace('&', "&&");
            if let Ok(item) =
                CheckMenuItem::with_id(app, id, label, true, device.is_default(), None::<&str>)
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
        MENU_AUTOSTART => {
            let autolaunch = app.autolaunch();
            let enabled = autolaunch.is_enabled().unwrap_or(false);
            let result = if enabled {
                autolaunch.disable()
            } else {
                autolaunch.enable()
            };
            match result {
                Ok(_) => log_line(&format!("autostart: {}", if enabled { "disabled" } else { "enabled" })),
                Err(err) => log_line(&format!(
                    "autostart: {} failed: {err}",
                    if enabled { "disable" } else { "enable" }
                )),
            }
            rebuild_tray_menu(app);
        }
        MENU_REFRESH => rebuild_tray_menu(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

/// Append a timestamped line to the update log in the app data directory.
/// Release builds have no console, so this is the only way to see what the
/// updater and autostart are doing.
fn log_line(message: &str) {
    let Ok(dir) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let dir = std::path::Path::new(&dir).join("com.audioswitch.app");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("app.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        use std::io::Write;
        let _ = writeln!(f, "{} {message}", chrono_now());
    }
}

/// Best-effort local timestamp via GetLocalTime (no extra dependencies).
fn chrono_now() -> String {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let st: SYSTEMTIME = unsafe { GetLocalTime() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        "????-??-?? ??:??:??".to_string()
    }
}

/// Check for updates in the background shortly after launch. When one is
/// found, it downloads and installs it, then exits so the new version takes
/// over (the MSI installs in passive mode with no prompts).
fn spawn_update_check(app: AppHandle) {
    let Ok(updater) = app.updater() else {
        log_line("update: updater is not configured");
        return;
    };
    log_line("update: checking for updates");
    tauri::async_runtime::spawn(async move {
        match updater.check().await {
            Ok(Some(update)) => {
                log_line(&format!("update: {} available; installing", update.version));
                match update.download_and_install(|_, _| {}, || {}).await {
                    Ok(_) => {
                        log_line("update: installed; exiting to apply");
                        app.exit(0);
                    }
                    Err(err) => log_line(&format!("update: install failed: {err}")),
                }
            }
            Ok(None) => log_line("update: up to date"),
            Err(err) => log_line(&format!("update: check failed: {err}")),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(AudioState(Mutex::new(())));

            let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("Windows Audio Switcher")
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
