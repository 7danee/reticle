//! Finds out which program owns a window. Only the program's file name is read, through a
//! query-only handle (the same access Task Manager uses); the process itself is never touched.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

fn exe_of_pid(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

/// File name of the program that owns `hwnd`, e.g. "cs2.exe".
pub fn exe_of_window(hwnd: HWND) -> Option<String> {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then(|| exe_of_pid(pid)).flatten()
}

/// File name of reticle itself.
pub fn own_exe() -> Option<String> {
    std::env::current_exe().ok()?.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Programs that currently show a normal window, sorted, without duplicates.
pub fn running_programs() -> Vec<String> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<String>) };
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        let titled = unsafe { GetWindowTextLengthW(hwnd) } > 0;
        let tool = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW.0 != 0;
        if visible && titled && !tool
            && let Some(exe) = exe_of_window(hwnd) {
                list.push(exe);
            }
        true.into()
    }
    let mut list: Vec<String> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    let own = own_exe().unwrap_or_default().to_lowercase();
    const SKIP: &[&str] = &["explorer.exe", "applicationframehost.exe", "textinputhost.exe", "systemsettings.exe"];
    list.retain(|e| {
        let l = e.to_lowercase();
        l != own && !SKIP.contains(&l.as_str())
    });
    list.sort_by_key(|e| e.to_lowercase());
    list.dedup_by_key(|e| e.to_lowercase());
    list
}
