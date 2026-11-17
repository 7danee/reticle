#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod clipboard;
mod config;
mod editor;
mod hotkey;
mod logo;
mod overlay;
mod process;
mod render;
mod share;
mod theme;
mod tray;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, MB_ICONERROR, MB_OK, MessageBoxW, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::{HSTRING, PCWSTR, w};

struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Returns `None` if a mutex with this name is already held by another process.
fn single_instance(name: PCWSTR) -> Option<InstanceGuard> {
    unsafe {
        let handle = CreateMutexW(None, true, name).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(InstanceGuard(handle))
    }
}

fn error_box(text: &str) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), w!("reticle"), MB_OK | MB_ICONERROR);
    }
}

fn run_editor() {
    let Some(_guard) = single_instance(w!("Local\\reticle_editor")) else {
        // Already open: bring it to the front.
        unsafe {
            if let Ok(hwnd) = FindWindowW(PCWSTR::null(), &HSTRING::from(editor::WINDOW_TITLE)) {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            }
        }
        return;
    };
    if let Err(e) = editor::run() {
        error_box(&format!("Die Einstellungen konnten nicht geöffnet werden:\n{e}"));
    }
}

fn run_background() {
    let Some(_guard) = single_instance(w!("Local\\reticle_single_instance")) else {
        // Already running in the tray: a second start opens the settings instead.
        app::spawn_editor();
        return;
    };
    if let Err(e) = app::run() {
        error_box(&format!("reticle konnte nicht starten:\n{}", e.message()));
    }
}

fn main() {
    unsafe {
        // Physical pixels everywhere, so the crosshair lands on the exact screen center.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if std::env::args().skip(1).any(|a| a == "--editor") {
        run_editor();
    } else {
        run_background();
    }
}
