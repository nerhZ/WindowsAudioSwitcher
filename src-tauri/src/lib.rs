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

/// Whether one of our own Win32 popup menus (class "#32768", our process) is
/// showing. Used to defer tray-menu rebuilds while a popup is open - calling
/// `set_menu` mid-popup dismisses it.
#[cfg(target_os = "windows")]
fn own_popup_menu_open() -> bool {
    own_popup_menu_hwnd().is_some()
}

/// Handle of our own popup menu window, if one is showing. See
/// [`own_popup_menu_open`].
#[cfg(target_os = "windows")]
fn own_popup_menu_hwnd() -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId,
    };

    struct Ctx {
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let mut buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut buf);
        if len > 0 && String::from_utf16_lossy(&buf[..len as usize]) == "#32768" {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == std::process::id() {
                ctx.found = Some(hwnd);
                return BOOL(0); // stop enumerating
            }
        }
        BOOL(1) // continue
    }

    let mut ctx = Ctx { found: None };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut ctx as *mut Ctx as isize));
    };
    ctx.found
}

#[cfg(not(target_os = "windows"))]
fn own_popup_menu_open() -> bool {
    false
}

/// Keep our tray popup menu inside the work area (screen minus taskbar).
///
/// The menu opens at the cursor, which sits inside the taskbar, so its bottom
/// strip overlaps it; behind a fullscreen app the taskbar covers that strip,
/// and z-order tricks (topmost on the menu or its owner) can't fix it. Moving
/// the menu into the work area the moment it opens makes the overlap
/// impossible.
///
/// Events, not clicks: tray-icon opens the menu with a blocking
/// TrackPopupMenu before any click event is delivered, so a WinEvent hook on
/// EVENT_SYSTEM_MENUPOPUPSTART is the only way to catch the menu while it is
/// still open. Only our own process's menus are touched.
#[cfg(target_os = "windows")]
fn spawn_menu_work_area_clamp() {
    std::thread::spawn(|| {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, GetWindowRect, GetWindowThreadProcessId, SetWindowPos,
            SystemParametersInfoW, TranslateMessage, EVENT_SYSTEM_MENUPOPUPSTART, MSG,
            SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WINEVENT_OUTOFCONTEXT,
        };

        /// Move a window so it fits entirely inside the work area (the screen
        /// minus the taskbar). The tray menu opens at the cursor - inside the
        /// taskbar - so it needs lifting by exactly the overlap amount.
        unsafe fn clamp_to_work_area(hwnd: HWND) {
            let mut menu_rect = RECT::default();
            if GetWindowRect(hwnd, &mut menu_rect).is_err() {
                return;
            }
            let width = menu_rect.right - menu_rect.left;
            let height = menu_rect.bottom - menu_rect.top;
            if width <= 0 || height <= 0 {
                return;
            }
            let mut work = RECT::default();
            if SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut work as *mut RECT as *mut core::ffi::c_void),
                Default::default(),
            )
            .is_err()
            {
                return;
            }
            let mut x = menu_rect.left;
            let mut y = menu_rect.top;
            if x + width > work.right {
                x = work.right - width;
            }
            if y + height > work.bottom {
                y = work.bottom - height;
            }
            if x < work.left {
                x = work.left;
            }
            if y < work.top {
                y = work.top;
            }
            if x != menu_rect.left || y != menu_rect.top {
                match SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
                ) {
                    Ok(_) => log_line(&format!(
                        "devices: menu moved from ({},{}) to ({x},{y}) size {width}x{height}",
                        menu_rect.left, menu_rect.top
                    )),
                    Err(err) => log_line(&format!("devices: menu move failed: {err}")),
                }
            }
        }

        unsafe extern "system" fn hook_proc(
            _hook: HWINEVENTHOOK,
            event: u32,
            hwnd: HWND,
            _id_object: i32,
            _id_child: i32,
            _event_thread: u32,
            _event_time: u32,
        ) {
            if event != EVENT_SYSTEM_MENUPOPUPSTART {
                return;
            }
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if pid != std::process::id() {
                return;
            }
            log_line(&format!("devices: menu popup start hwnd={hwnd:?}"));
            unsafe { clamp_to_work_area(hwnd) };
            // The rect may not be final at EVENT_SYSTEM_MENUPOPUPSTART (still
            // zero-size or mid-layout); one delayed pass catches that. HWND
            // is not Send, so the handle travels as a raw usize.
            let hwnd = hwnd.0 as usize;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(15));
                unsafe { clamp_to_work_area(HWND(hwnd as *mut core::ffi::c_void)) };
            });
        }

        unsafe {
            let hook = SetWinEventHook(
                EVENT_SYSTEM_MENUPOPUPSTART,
                EVENT_SYSTEM_MENUPOPUPSTART,
                None,
                Some(hook_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if hook.is_invalid() {
                log_line("devices: work-area hook failed - menu may overlap the taskbar");
                return;
            }
            let mut msg = MSG::default();
            // GetMessageW returns -1 on error; only nonzero-positive means a
            // message was retrieved (0 = WM_QUIT).
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn spawn_menu_work_area_clamp() {}

/// Rebuild the tray menu, deferring while a popup menu is open: replacing the
/// HMENU mid-popup dismisses it (the flash bug). Waits on a background thread
/// so menu clicks keep working, then rebuilds on the main thread.
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

/// Move our autostart Run entry to the front of the key.
///
/// Winlogon launches Run values in the order they appear in the registry,
/// which is insertion order - so a value deleted and re-created by every
/// enable/disable cycle always ends up last. Reordering to the front once per
/// launch (values update in place afterwards) keeps the tray appearing early
/// in the logon sequence. No-op when autostart is disabled or already first.
#[cfg(target_os = "windows")]
fn reorder_autostart_to_front(app_name: &str) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::transaction::Transaction;
    use winreg::{RegKey, RegValue};

    const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // No API can insert a value at the front of a key (the value list is
    // append-only), so reordering means rewriting the key. The rewrite runs
    // inside a TxR transaction: a crash or power loss mid-write rolls the
    // key back instead of leaving startup entries half-deleted.
    let Ok(t) = Transaction::new() else {
        log_line("autostart: reorder skipped - TxR transaction unavailable");
        return;
    };
    let Ok(run) = hkcu.open_subkey_transacted_with_flags(RUN_KEY, &t, KEY_READ | KEY_WRITE) else {
        log_line("autostart: reorder skipped - cannot open Run key");
        return;
    };
    // enum_values yields values in registry order (= the launch order).
    let Ok(mut items) = run.enum_values().collect::<Result<Vec<(String, RegValue)>, _>>() else {
        log_line("autostart: reorder skipped - cannot enumerate Run values");
        return;
    };
    let Some(own_index) = items.iter().position(|(n, _)| n.eq_ignore_ascii_case(app_name)) else {
        return; // autostart not enabled - leave the key alone
    };
    if own_index == 0 {
        return; // already first
    }
    let own = items.remove(own_index);
    // Rewrite in one burst: our value first, then everything else in its
    // original order, with original types and raw bytes preserved. Any
    // failure aborts the whole rewrite - never commit a partial Run key.
    // Names are already UTF-16-safe here: winreg's enum_values uses strict
    // from_utf16, so a lone-surrogate name aborts the enumeration above with
    // ERROR_INVALID_DATA before any write happens - reorder is just skipped.
    let rewrite = (|| -> Result<(), ()> {
        for (name, _) in &items {
            run.delete_value(name).map_err(|_| ())?;
        }
        run.set_raw_value(&own.0, &own.1).map_err(|_| ())?;
        for (name, value) in &items {
            run.set_raw_value(name, value).map_err(|_| ())?;
        }
        Ok(())
    })();
    if rewrite.is_err() {
        let _ = t.rollback();
        log_line("autostart: reorder aborted - Run key rewrite failed, rolled back");
        return;
    }
    if t.commit().is_err() {
        log_line("autostart: Run reorder transaction failed - rolled back");
        return;
    }
    log_line("autostart: Run entry moved to front for earlier launch");
}

#[cfg(not(target_os = "windows"))]
fn reorder_autostart_to_front(_app_name: &str) {}

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
                Ok(_) => {
                    log_line(&format!("autostart: {}", if enabled { "disabled" } else { "enabled" }));
                    // enable() appends the Run value to the end of the key, so
                    // move it to the front right away - otherwise the very
                    // next boot would still start last (the startup self-heal
                    // only fixes the boot after that).
                    if !enabled {
                        reorder_autostart_to_front(app.package_info().name.as_str());
                    }
                }
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

/// App data directory: `%LOCALAPPDATA%\com.audioswitch.app`, where the log and
/// the settings file live. `None` when the environment has no LOCALAPPDATA.
fn app_data_dir() -> Option<std::path::PathBuf> {
    Some(
        std::path::Path::new(&std::env::var("LOCALAPPDATA").ok()?).join("com.audioswitch.app"),
    )
}

/// Append a timestamped line to the update log in the app data directory.
/// Release builds have no console, so this is the only way to see what the
/// updater, autostart and device watcher are doing. The log rotates when it
/// exceeds [`LOG_MAX_BYTES`].
pub(crate) fn log_line(message: &str) {
    let Some(dir) = app_data_dir() else {
        return;
    };
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
    Some(app_data_dir()?.join("settings.txt"))
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

            // Winlogon launches Run entries in insertion order; every
            // enable/disable cycle re-appends ours, so it would start last.
            // Nudge it to the front once per launch (no-op unless enabled
            // and not already first).
            reorder_autostart_to_front(app.package_info().name.as_str());

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
            // clicks, and rebuilding it from tray events would replace the
            // HMENU mid-popup, making it flash. It is rebuilt only from menu
            // events (Refresh, device switches) - moments when no popup is
            // open. Keeping the popup in the work area is handled by
            // `spawn_menu_work_area_clamp`, not click events (which are
            // delivered only after the popup closes).
            spawn_menu_work_area_clamp();
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
