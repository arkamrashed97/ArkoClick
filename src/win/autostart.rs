//! "Launch with Windows" via the per-user Run registry key. No admin rights needed.

use super::to_wide;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "ArkoClick";

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let path = to_wide(RUN_KEY);
    let mut key = HKEY::default();
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, access, &mut key) };
    r.is_ok().then_some(key)
}

/// The command we register: our own exe plus a flag so the app knows it was auto-launched.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --autostart", exe.display()))
}

pub fn is_enabled() -> bool {
    let Some(key) = open(KEY_QUERY_VALUE) else { return false };
    let name = to_wide(VALUE_NAME);
    let r = unsafe { RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, None, None) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r.is_ok()
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let key = open(KEY_SET_VALUE).ok_or("cannot open HKCU Run key")?;
    let name = to_wide(VALUE_NAME);
    let result = unsafe {
        if enabled {
            let cmd = command().ok_or("cannot resolve exe path")?;
            let wide = to_wide(&cmd);
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)).ok().map_err(|e| e.to_string())
        } else {
            let r = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
            // Deleting a value that is already gone is not an error for us.
            if r.is_ok() || !is_enabled_with(key) { Ok(()) } else { Err(r.to_hresult().to_string()) }
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn is_enabled_with(key: HKEY) -> bool {
    let name = to_wide(VALUE_NAME);
    unsafe { RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, None, None).is_ok() }
}
