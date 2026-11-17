//! "Start with Windows" via the per-user Run key. No admin rights needed.

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::w;

fn command() -> Option<String> {
    Some(format!("\"{}\"", std::env::current_exe().ok()?.display()))
}

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), Some(0), access, &mut key)
    };
    status.is_ok().then_some(key)
}

/// True if reticle (this exe) starts with Windows.
pub fn is_enabled() -> bool {
    let Some(key) = open(KEY_QUERY_VALUE) else { return false };
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let mut kind = REG_VALUE_TYPE::default();
    let status = unsafe {
        RegQueryValueExW(key, w!("reticle"), None, Some(&mut kind), Some(buf.as_mut_ptr() as *mut u8), Some(&mut size))
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status.is_err() || kind != REG_SZ {
        return false;
    }
    let value = String::from_utf16_lossy(&buf[..(size as usize / 2)]);
    command().is_some_and(|c| value.trim_end_matches('\0').eq_ignore_ascii_case(&c))
}

pub fn set(enabled: bool) -> Result<(), String> {
    let key = open(KEY_SET_VALUE).ok_or("Autostart-Eintrag konnte nicht geöffnet werden")?;
    let status = unsafe {
        if enabled {
            let cmd = command().ok_or("Programmpfad unbekannt")?;
            let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(key, w!("reticle"), Some(0), REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(key, w!("reticle"))
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status.is_ok() || !enabled {
        Ok(())
    } else {
        Err(format!("Autostart konnte nicht gesetzt werden ({})", status.0))
    }
}
