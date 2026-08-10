//! A dedicated STA thread owns ALL Core Audio work.
//!
//! - The thread runs a real Win32 message pump (`PeekMessageW`/`DispatchMessageW`),
//!   which is what delivers `IMMNotificationClient` callbacks into the process.
//! - The notification client is registered on this same thread, and every audio
//!   operation (listing, switching) is executed here too - a single apartment,
//!   which MMDevApi requires (mixed apartments crash it).
//! - The app's threads send jobs over a channel and wait for the reply.

use std::sync::mpsc::{sync_channel, Receiver, Sender, SyncSender};
use std::sync::OnceLock;

use windows::core::{implement, PCWSTR, Result as CoreResult};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, EDataFlow, ERole, IMMNotificationClient, IMMNotificationClient_Impl,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
};

enum AudioJob {
    ListDevices(SyncSender<Result<Vec<super::AudioDevice>, String>>),
    SetDefault(String, SyncSender<Result<(), String>>),
}

struct Executor {
    jobs: Sender<AudioJob>,
}

static EXECUTOR: OnceLock<Executor> = OnceLock::new();

/// COM object handed to the endpoint enumerator; the `#[implement]` macro
/// generates the vtable and refcounting around this struct.
#[implement(IMMNotificationClient)]
struct NotificationClient {
    events: Sender<()>,
}

impl NotificationClient_Impl {
    fn notify(&self) -> CoreResult<()> {
        let _ = self.events.send(());
        Ok(())
    }
}

impl IMMNotificationClient_Impl for NotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> CoreResult<()> {
        self.notify()
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> CoreResult<()> {
        self.notify()
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> CoreResult<()> {
        self.notify()
    }

    fn OnDefaultDeviceChanged(&self, _flow: EDataFlow, _role: ERole, _id: &PCWSTR) -> CoreResult<()> {
        self.notify()
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> CoreResult<()> {
        self.notify()
    }
}

/// Starts the audio core thread and the change-notification consumer. `on_change`
/// is invoked (from a background thread, after a short debounce) whenever the
/// device list or the default endpoint may have changed. Safe to call once;
/// subsequent calls are no-ops.
pub fn init_audio_core(on_change: impl Fn() + Send + 'static) -> Result<(), String> {
    if EXECUTOR.get().is_some() {
        return Ok(());
    }

    let (job_tx, job_rx) = std::sync::mpsc::channel::<AudioJob>();
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    std::thread::Builder::new()
        .name("audio-core".to_string())
        .spawn(move || audio_core_thread(job_rx, notify_tx, ready_tx))
        .map_err(|err| format!("failed to spawn audio core thread: {err}"))?;

    // Only publish the executor once the core thread confirms COM is up -
    // otherwise a dead channel would poison every later audio call. The
    // notification client registration happens after this handshake and is
    // best-effort: the thread always runs its message pump, so all audio
    // operations stay on the single STA thread (never the inline fallback),
    // keeping the MMDevApi single-apartment guarantee regardless.
    match ready_rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(())) => {
            let _ = EXECUTOR.set(Executor { jobs: job_tx });
            spawn_consumer(notify_rx, on_change);
            Ok(())
        }
        // The thread died before any notification client could be registered,
        // so inline fallback is safe (no registered client exists).
        Ok(Err(err)) => {
            crate::log_line(&format!(
                "devices: watcher unavailable ({err}); continuing without auto-refresh"
            ));
            Ok(())
        }
        // Practically unreachable: ready is sent immediately after CoInit.
        Err(_) => {
            crate::log_line("devices: watcher startup timed out; continuing without auto-refresh");
            Ok(())
        }
    }
}

fn audio_core_thread(
    jobs: Receiver<AudioJob>,
    notify: Sender<()>,
    ready: Sender<Result<(), String>>,
) {
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        let _ = ready.send(Err(format!("CoInitializeEx failed: {hr}")));
        return;
    }
    let _ = ready.send(Ok(()));

    // Notification client registration is best-effort: on failure the app
    // runs without auto-refresh, but the pump loop below still serves audio
    // operations on this thread.
    let client: IMMNotificationClient = NotificationClient { events: notify }.into();
    let setup = super::windows_impl::create_enumerator().and_then(|enumerator| {
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
            .map_err(|err| format!("RegisterEndpointNotificationCallback: {err}"))
            .map(|_| enumerator)
    });
    match &setup {
        Ok(_) => crate::log_line("devices: watcher started"),
        Err(err) => crate::log_line(&format!("devices: watcher failed: {err}")),
    }
    let _keep = (setup.ok(), client);

    // Message pump + job loop. Notifications arrive via DispatchMessageW; jobs
    // are drained each iteration.
    let mut msg = MSG::default();
    loop {
        while let Ok(job) = jobs.try_recv() {
            execute(job);
        }
        let has_msg = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) };
        if has_msg.as_bool() {
            if msg.message == WM_QUIT {
                break;
            }
            unsafe { let _ = TranslateMessage(&msg); };
            unsafe { DispatchMessageW(&msg) };
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    unsafe { CoUninitialize() };
}

fn execute(job: AudioJob) {
    match job {
        AudioJob::ListDevices(reply) => {
            let _ = reply.send(super::windows_impl::list_devices());
        }
        AudioJob::SetDefault(id, reply) => {
            let _ = reply.send(super::windows_impl::set_default(&id));
        }
    }
}

/// Route a job through the audio core thread. Falls back to running inline
/// when the core has not been started (e.g. tests).
fn via_core<T>(
    make_job: impl FnOnce(SyncSender<Result<T, String>>) -> AudioJob,
    fallback: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let Some(executor) = EXECUTOR.get() else {
        return fallback();
    };
    let (tx, rx) = sync_channel(1);
    executor
        .jobs
        .send(make_job(tx))
        .map_err(|err| format!("audio core unavailable: {err}"))?;
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|err| format!("audio core unresponsive: {err}"))?
}

pub fn list_devices() -> Result<Vec<super::AudioDevice>, String> {
    via_core(AudioJob::ListDevices, super::windows_impl::list_devices)
}

pub fn set_default(device_id: &str) -> Result<(), String> {
    via_core(
        |tx| AudioJob::SetDefault(device_id.to_string(), tx),
        || super::windows_impl::set_default(device_id),
    )
}

/// Coalescing window for device-change bursts (Windows fires several events
/// per plug/unplug within a few milliseconds). Kept tiny (~20ms) so the menu
/// refresh always finishes before a human could possibly open the tray menu -
/// the menu never shows a device that is "still waiting to appear".
const CHANGE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(20);

fn spawn_consumer(rx: Receiver<()>, on_change: impl Fn() + Send + 'static) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            while rx.try_recv().is_ok() {}
            std::thread::sleep(CHANGE_DEBOUNCE);
            while rx.try_recv().is_ok() {}
            crate::log_line("devices: change event received");
            on_change();
        }
    })
}
