//! Windows Core Audio: enumerate playback endpoints and switch the system
//! default for all three roles (Console / Multimedia / Communications) at once.

#[cfg(target_os = "windows")]
mod policyconfig;

/// One playback endpoint on the host.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default_console: bool,
    pub is_default_multimedia: bool,
    pub is_default_communications: bool,
}

impl AudioDevice {
    /// True when this device holds all three role defaults (the app's contract).
    pub fn is_default(&self) -> bool {
        self.is_default_console && self.is_default_multimedia && self.is_default_communications
    }
}

#[cfg(target_os = "windows")]
pub fn list_devices() -> Result<Vec<AudioDevice>, String> {
    windows_impl::list_devices()
}

#[cfg(target_os = "windows")]
pub fn set_default(device_id: &str) -> Result<(), String> {
    windows_impl::set_default(device_id)
}

#[cfg(not(target_os = "windows"))]
pub fn list_devices() -> Result<Vec<AudioDevice>, String> {
    Err("AudioSwitch requires Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn set_default(_device_id: &str) -> Result<(), String> {
    Err("AudioSwitch requires Windows".to_string())
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::policyconfig;
    use std::ffi::c_void;
    use windows::core::{Error as WinError, PWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{
        eCommunications, eConsole, eMultimedia, eRender, ERole, IMMDevice,
        IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED, STGM_READ,
    };

    /// RAII guard for CoInitializeEx / CoUninitialize. Windows refcounts
    /// per-thread, so nested creation is safe — but only the call that
    /// actually initialized the apartment may uninitialize it.
    ///
    /// The main thread may already live in an apartment set up by Tauri or
    /// WebView2, in which case an MTA init fails with RPC_E_CHANGED_MODE.
    /// The existing apartment is still fully usable for Core Audio, so we
    /// reuse it instead of failing.
    pub(super) struct ComGuard {
        pub(super) owns_init: bool,
    }

    // RPC_E_CHANGED_MODE: the thread is already in a different apartment.
    pub(super) const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;

    impl ComGuard {
        pub(super) fn new() -> Result<Self, String> {
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                // S_OK means this call initialized the apartment; S_FALSE
                // means it was already initialized in this very mode.
                Ok(Self {
                    owns_init: hr.0 == 0,
                })
            } else if hr.0 == RPC_E_CHANGED_MODE {
                Ok(Self { owns_init: false })
            } else {
                Err(format!("CoInitializeEx: {}", WinError::from_hresult(hr)))
            }
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.owns_init {
                unsafe { CoUninitialize() }
            }
        }
    }

    fn create_enumerator() -> Result<IMMDeviceEnumerator, String> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|err| format!("CoCreateInstance(MMDeviceEnumerator): {err}"))
    }

    /// Owns a `PWSTR` that Windows allocated with `CoTaskMemAlloc`
    /// (`IMMDevice::GetId` hands back a raw pointer without transferring
    /// ownership — freeing it is this guard's only job).
    struct CoTaskString(PWSTR);

    impl CoTaskString {
        fn to_rust_string(&self) -> String {
            if self.0.is_null() {
                return String::new();
            }
            unsafe {
                let mut len = 0usize;
                while *self.0 .0.add(len) != 0 {
                    len += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(self.0 .0, len))
            }
        }
    }

    impl Drop for CoTaskString {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CoTaskMemFree(Some(self.0 .0 as *const c_void)) };
                self.0 = PWSTR::null();
            }
        }
    }

    fn device_id(device: &IMMDevice) -> Result<String, String> {
        let raw = CoTaskString(
            unsafe { device.GetId() }.map_err(|err| format!("IMMDevice::GetId: {err}"))?,
        );
        Ok(raw.to_rust_string())
    }

    fn friendly_name(device: &IMMDevice) -> Result<String, String> {
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }
            .map_err(|err| format!("IMMDevice::OpenPropertyStore: {err}"))?;
        let value = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
            .map_err(|err| format!("IPropertyStore::GetValue(FriendlyName): {err}"))?;
        // PROPVARIANT is opaque in windows-rs; its Display impl renders
        // VT_LPWSTR / VT_BSTR as the underlying string.
        Ok(value.to_string())
    }

    #[derive(Default)]
    struct RoleDefaults {
        console: Option<String>,
        multimedia: Option<String>,
        communications: Option<String>,
    }

    fn current_defaults(enumerator: &IMMDeviceEnumerator) -> RoleDefaults {
        fn one(enumerator: &IMMDeviceEnumerator, role: ERole) -> Option<String> {
            let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, role) }.ok()?;
            let raw = CoTaskString(unsafe { device.GetId() }.ok()?);
            Some(raw.to_rust_string())
        }
        RoleDefaults {
            console: one(enumerator, eConsole),
            multimedia: one(enumerator, eMultimedia),
            communications: one(enumerator, eCommunications),
        }
    }

    /// Enumerate active render endpoints and mark which one currently holds
    /// each role default.
    pub(super) fn list_devices() -> Result<Vec<super::AudioDevice>, String> {
        let _com = ComGuard::new()?;
        let enumerator = create_enumerator()?;

        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(|err| format!("EnumAudioEndpoints: {err}"))?;
        let count =
            unsafe { collection.GetCount() }.map_err(|err| format!("GetCount: {err}"))?;

        let defaults = current_defaults(&enumerator);

        let mut devices = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device = unsafe { collection.Item(index) }
                .map_err(|err| format!("IMMDeviceCollection::Item({index}): {err}"))?;
            let id = device_id(&device)?;
            let name = friendly_name(&device).unwrap_or_else(|_| "(unnamed device)".to_string());
            let is_role_default = |current: &Option<String>| {
                current
                    .as_deref()
                    .map(|default| default.eq_ignore_ascii_case(&id))
                    .unwrap_or(false)
            };
            devices.push(super::AudioDevice {
                is_default_console: is_role_default(&defaults.console),
                is_default_multimedia: is_role_default(&defaults.multimedia),
                is_default_communications: is_role_default(&defaults.communications),
                id,
                name,
            });
        }
        Ok(devices)
    }

    /// Switch all three role defaults to `device_id`, then re-read them and
    /// confirm each settled — a split (partial failure, or a concurrent
    /// switch) is reported as an error, never a silent success.
    pub(super) fn set_default(device_id: &str) -> Result<(), String> {
        let _com = ComGuard::new()?;
        policyconfig::set_default_for_all_roles(device_id)
            .map_err(|err| format!("IPolicyConfig::SetDefaultEndpoint: {err}"))?;
        verify_all_roles(device_id)
    }

    fn verify_all_roles(device_id: &str) -> Result<(), String> {
        let enumerator = create_enumerator()?;
        let actual = current_defaults(&enumerator);
        let settled = |current: &Option<String>| {
            current
                .as_deref()
                .map(|id| id.eq_ignore_ascii_case(device_id))
                .unwrap_or(false)
        };
        if settled(&actual.console)
            && settled(&actual.multimedia)
            && settled(&actual.communications)
        {
            Ok(())
        } else {
            Err("default endpoint roles diverged after switching".to_string())
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::windows_impl::ComGuard;
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    #[test]
    fn enumerates_active_render_endpoints() {
        let devices = list_devices().expect("should enumerate endpoints");
        assert!(!devices.is_empty(), "expected at least one playback device");
        assert!(devices.iter().any(|d| !d.name.is_empty()), "names should be readable");
    }

    #[test]
    fn exactly_one_multimedia_default() {
        let devices = list_devices().unwrap();
        let count = devices
            .iter()
            .filter(|d| d.is_default_multimedia)
            .count();
        assert_eq!(count, 1, "exactly one device should be the multimedia default");
    }

    /// Idempotent no-op switch: re-asserts the current default across all
    /// three roles. Exercises the full IPolicyConfig path (CoCreateInstance,
    /// QueryInterface, vtable call, role verification) without changing state.
    #[test]
    fn policy_config_switch_is_idempotent() {
        let devices = list_devices().unwrap();
        let current = devices
            .iter()
            .find(|d| d.is_default_multimedia)
            .expect("a multimedia default exists");
        set_default(&current.id).expect("re-asserting the current default should succeed");
        let after = list_devices().unwrap();
        assert!(after
            .iter()
            .any(|d| d.id == current.id && d.is_default()),
            "device should still hold all three role defaults");
    }

    /// Regression: the main thread may already live in an STA (set up by Tauri
    /// or WebView2), so an MTA init fails with RPC_E_CHANGED_MODE. The guard
    /// must reuse the existing apartment rather than fail.
    #[test]
    fn com_guard_reuses_existing_apartment() {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        assert!(hr.is_ok(), "test setup: STA init failed");

        let guard = ComGuard::new().expect("MTA attempt must fall back to the existing STA");
        assert!(!guard.owns_init, "guard must not uninitialize an apartment it did not create");
        drop(guard);

        unsafe { CoUninitialize() }
    }
}
