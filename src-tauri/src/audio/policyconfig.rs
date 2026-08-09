//! Manual COM binding for the undocumented `IPolicyConfig` interface - the
//! only known way to change the system-wide default audio endpoint from code.
//!
//! GUIDs and vtable layout are long-standing, community-verified facts
//! (EarTrumpet, SoundVolumeView and related writeups since Windows Vista).
//! `SetDefaultEndpoint` sits at vtable slot 13, after 3 IUnknown methods,
//! 8 internal methods, GetPropertyValue and SetPropertyValue.

use std::ffi::c_void;
use std::ptr;

use windows::core::{Interface, IUnknown, GUID, HRESULT, PCWSTR};
use windows::Win32::Media::Audio::{eCommunications, eConsole, eMultimedia, ERole};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

// CLSID_CPolicyConfigClient
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

// IPolicyConfig IIDs differ per Windows build:
// - Win7/8 and Win10 RS1+: F8679F50-850A-41CF-9C72-430F290290C8
// - Win10 TH2:             6BE54BE8-A068-4875-A49D-0C2966473B11
// - Win10 TH1:             CA286FC3-91FD-42C3-8E9B-CAAFA66242E3
// - Vista (Vista-shaped):  568B9108-44BF-40B4-9006-86AFE5B5A620
const IID_IPOLICY_CONFIG_RS1: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
const IID_IPOLICY_CONFIG_TH2: GUID = GUID::from_u128(0x6be54be8_a068_4875_a49d_0c2966473b11);
const IID_IPOLICY_CONFIG_TH1: GUID = GUID::from_u128(0xca286fc3_91fd_42c3_8e9b_caafa66242e3);
const IID_IPOLICY_CONFIG_VISTA: GUID = GUID::from_u128(0x568b9108_44bf_40b4_9006_86afe5b5a620);

#[repr(C)]
struct IPolicyConfigVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut c_void,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
    // Opaque slots we never call - present only to keep the layout right.
    _get_mix_format: usize,        // 3
    _get_device_format: usize,     // 4
    _reset_device_format: usize,   // 5
    _set_device_format: usize,     // 6
    _get_processing_period: usize, // 7
    _set_processing_period: usize, // 8
    _get_share_mode: usize,        // 9
    _set_share_mode: usize,        // 10
    _get_property_value: usize,    // 11
    _set_property_value: usize,    // 12
    set_default_endpoint:
        unsafe extern "system" fn(this: *mut c_void, device_id: PCWSTR, role: ERole) -> HRESULT,
    _set_endpoint_visibility: usize, // 14
}

/// Thin RAII wrapper over the COM object; calls Release on drop.
struct PolicyConfig {
    raw: *mut c_void,
    vtbl: *const IPolicyConfigVtbl,
}

impl PolicyConfig {
    /// CoCreate the policy client, then QI the first IID variant that answers.
    fn create() -> windows::core::Result<Self> {
        let unknown: IUnknown =
            unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }?;

        for iid in [
            IID_IPOLICY_CONFIG_RS1,
            IID_IPOLICY_CONFIG_TH2,
            IID_IPOLICY_CONFIG_TH1,
            IID_IPOLICY_CONFIG_VISTA,
        ] {
            let mut raw: *mut c_void = ptr::null_mut();
            let hr = unsafe {
                (Interface::vtable(&unknown).QueryInterface)(unknown.as_raw(), &iid, &mut raw)
            };
            if hr.is_ok() && !raw.is_null() {
                // The vtable pointer is the first field of a COM object.
                let vtbl = unsafe { *(raw as *const *const IPolicyConfigVtbl) };
                return Ok(PolicyConfig { raw, vtbl });
            }
        }
        // E_NOINTERFACE - none of the known IID variants accepted.
        Err(windows::core::Error::from_hresult(HRESULT(0x80004002u32 as i32)))
    }

    fn set_default_endpoint(&self, device_id: &str, role: ERole) -> windows::core::Result<()> {
        let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
        let hr =
            unsafe { ((*self.vtbl).set_default_endpoint)(self.raw, PCWSTR(wide.as_ptr()), role) };
        if hr.is_ok() {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(hr))
        }
    }
}

impl Drop for PolicyConfig {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { ((*self.vtbl).release)(self.raw) };
        }
    }
}

/// Set the device as the default for Console, Multimedia and Communications.
pub(super) fn set_default_for_all_roles(device_id: &str) -> windows::core::Result<()> {
    let config = PolicyConfig::create()?;
    for role in [eConsole, eMultimedia, eCommunications] {
        config.set_default_endpoint(device_id, role)?;
    }
    Ok(())
}
