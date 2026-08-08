// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Only one instance may run at a time. A named kernel mutex is the guard:
/// creating it when an instance already holds it reports
/// ERROR_ALREADY_EXISTS, and the second process exits immediately. The
/// handle is leaked for the process lifetime - dropping it would destroy
/// the mutex and let a second instance start.
#[cfg(target_os = "windows")]
fn ensure_single_instance() {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, BOOL};
    use windows::Win32::System::Threading::CreateMutexW;

    const NAME: &str = "Local\\com.audioswitch.app.single-instance";
    let wide: Vec<u16> = NAME.encode_utf16().chain(std::iter::once(0)).collect();

    let handle = unsafe { CreateMutexW(None, BOOL(0), PCWSTR(wide.as_ptr())) };
    match handle {
        Ok(mutex) => {
            // GetLastError is meaningful even on success (already-exists case).
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                std::process::exit(0);
            }
            // HANDLE is Copy, so mem::forget is a no-op - leaking a Box keeps
            // the kernel object alive for the whole process lifetime.
            Box::leak(Box::new(mutex));
        }
        Err(_) => {
            // Fail open: if the mutex cannot be created, run without the guard.
            eprintln!("Windows Audio Switcher: single-instance guard unavailable");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn ensure_single_instance() {}

fn main() {
    ensure_single_instance();
    tauri_app_lib::run()
}
