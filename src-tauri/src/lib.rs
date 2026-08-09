mod audio;

use std::sync::Mutex;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_updater::UpdaterExt;

const MENU_CHECK_UPDATES: &str = "check-updates";
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
    let Ok(check_updates) = CheckMenuItem::with_id(
        app,
        MENU_CHECK_UPDATES,
        "Check for updates on startup",
        true,
        update_check_enabled(),
        None::<&str>,
    ) else {
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
    let _ = menu.append(&check_updates);
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

/// Whether our own Win32 popup menu (e.g. the open tray menu) is currently
/// showing. Popup menus are top-level windows of class "#32768"; we only match
/// windows owned by this process, so other applications' open menus are
/// ignored. (The foreground-window check cannot be used: tray-icon foregrounds
/// its hidden window, not the menu.)
#[cfg(target_os = "windows")]
fn own_popup_menu_open() -> bool {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId,
    };

    struct Ctx {
        found: bool,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let mut buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut buf);
        if len > 0 && String::from_utf16_lossy(&buf[..len as usize]) == "#32768" {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == std::process::id() {
                ctx.found = true;
                return BOOL(0); // stop enumerating
            }
        }
        BOOL(1) // continue
    }

    let mut ctx = Ctx { found: false };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut ctx as *mut Ctx as isize));
    };
    ctx.found
}

#[cfg(not(target_os = "windows"))]
fn own_popup_menu_open() -> bool {
    false
}

/// Rebuild the tray menu, deferring while a popup menu is open: calling
/// `set_menu` mid-popup dismisses it (the flash bug). When a popup is open the
/// wait happens on a background thread - the main thread must stay free so
/// menu clicks keep working - and the rebuild is dispatched back once the
/// popup closes (50ms intervals, 10s cap). The open menu may show stale
/// devices until it is dismissed.
fn rebuild_tray_menu_when_closed(app: AppHandle) {
    if !own_popup_menu_open() {
        rebuild_tray_menu(&app);
        return;
    }
    log_line("devices: changed - refreshing tray menu (deferred while open)");
    std::thread::spawn(move || {
        let mut waited = 0u32;
        while own_popup_menu_open() && waited < 10_000 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 50;
        }
        let cb = app.clone();
        let _ = app.run_on_main_thread(move || {
            rebuild_tray_menu(&cb);
        });
    });
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
        MENU_CHECK_UPDATES => {
            let next = !update_check_enabled();
            set_update_check_enabled(next);
            log_line(&format!(
                "update check on startup: {}",
                if next { "enabled" } else { "disabled" }
            ));
            rebuild_tray_menu(app);
        }
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
/// updater, autostart and device watcher are doing. The log rotates when it
/// exceeds [`LOG_MAX_BYTES`].
pub(crate) fn log_line(message: &str) {
    let Ok(dir) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let dir = std::path::Path::new(&dir).join("com.audioswitch.app");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("app.log");
    // Rotate before appending: the previous log becomes app.log.old. Ignore
    // failures (e.g. another thread holds the file open) - the next write
    // retries the rotation.
    if std::fs::metadata(&file)
        .map(|m| m.len() > LOG_MAX_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&file, dir.join("app.log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        use std::io::Write;
        let _ = writeln!(f, "{} {message}", chrono_now());
    }
}

/// Maximum app.log size before it rotates to app.log.old.
const LOG_MAX_BYTES: u64 = 256 * 1024;

/// The settings file lives next to the log in the app data directory.
/// Returns `None` when the app data directory is unavailable, mirroring
/// `log_line`'s early return.
fn settings_path() -> Option<std::path::PathBuf> {
    let dir = std::env::var("LOCALAPPDATA").ok()?;
    Some(std::path::Path::new(&dir).join("com.audioswitch.app").join("settings.txt"))
}

/// Whether to check for updates on startup. Defaults to enabled; a missing or
/// unreadable settings file is treated as enabled.
fn update_check_enabled() -> bool {
    let Some(path) = settings_path() else {
        return true;
    };
    std::fs::read_to_string(path)
        .map(|s| s.trim() == "true")
        .unwrap_or(true)
}

fn set_update_check_enabled(enabled: bool) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, if enabled { "true" } else { "false" }) {
        log_line(&format!("update check setting write failed: {err}"));
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
/// over (the NSIS installer relaunches the app in silent mode).
fn spawn_update_check(app: AppHandle) {
    if !update_check_enabled() {
        log_line("update: skipped (disabled in settings)");
        return;
    }
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

            // A failure here degrades gracefully: audio ops run inline and the
            // app still works, just without auto-refresh.
            if let Err(err) = audio::init_audio_core({
                let app = app.handle().clone();
                move || {
                    let app = app.clone();
                    let cb_app = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        log_line("devices: changed - refreshing tray menu");
                        rebuild_tray_menu_when_closed(cb_app);
                    });
                }
            }) {
                log_line(&format!("devices: audio core failed to start: {err}"));
            }

            let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("Windows Audio Switcher")
                .on_menu_event(handle_menu_event);
            // No on_tray_icon_event handler: the native menu opens on both
            // clicks (Windows default), and rebuilding the menu from tray
            // events would replace the HMENU while Windows is tracking the
            // popup, making it flash open and close. The menu is built at
            // startup and refreshed only from menu events (Refresh devices,
            // device switches) - moments when no popup is displayed.

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
