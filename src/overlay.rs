//! The crosshair window: layered, click-through, topmost, never activated.

use std::ffi::c_void;

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW,
    HDC, HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow, ReleaseDC, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowRect, HTTRANSPARENT, HWND_TOPMOST, MONITORINFOF_PRIMARY, RegisterClassW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowDisplayAffinity, SetWindowPos,
    ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_NCHITTEST, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, Result, w};

use crate::render;

pub struct Overlay {
    hwnd: HWND,
    shown: bool,
}

impl Overlay {
    pub fn create(instance: HINSTANCE) -> Result<Self> {
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance,
                lpszClassName: w!("reticle_overlay"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("reticle_overlay"),
                w!("reticle"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )?;
            Ok(Self { hwnd, shown: false })
        }
    }

    /// Uploads the pixmap and moves the window so the canvas center sits at `(cx, cy)`.
    pub fn update(&mut self, pixmap: &Pixmap, cx: i32, cy: i32) -> Result<()> {
        let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
        let pixels = render::to_bgra(pixmap);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let result = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0).and_then(|bitmap| {
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
                let previous = SelectObject(mem, bitmap.into());
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let result = UpdateLayeredWindow(
                    self.hwnd,
                    Some(screen),
                    Some(&POINT { x: cx - w / 2, y: cy - h / 2 }),
                    Some(&SIZE { cx: w, cy: h }),
                    Some(mem),
                    Some(&POINT { x: 0, y: 0 }),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(mem, previous);
                let _ = DeleteObject(bitmap.into());
                result
            });
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            result
        }
    }

    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        unsafe {
            let _ = ShowWindow(self.hwnd, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        if shown {
            self.raise();
        }
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// Puts the window back on top of the topmost band (some games push themselves above).
    pub fn raise(&self) {
        if !self.shown {
            return;
        }
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// Hides the window from screenshots, recordings and streams (Windows 10 2004+).
    pub fn set_capture_excluded(&self, excluded: bool) -> Result<()> {
        unsafe { SetWindowDisplayAffinity(self.hwnd, if excluded { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE }) }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub rect: RECT,
    pub primary: bool,
}

impl Monitor {
    /// Center point as pixel boundary, the same spot games use for their crosshair.
    pub fn center(&self) -> (i32, i32) {
        let r = self.rect;
        (r.left + (r.right - r.left) / 2, r.top + (r.bottom - r.top) / 2)
    }
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            list.push(Monitor { rect: info.rcMonitor, primary: info.dwFlags & MONITORINFOF_PRIMARY != 0 });
        }
        true.into()
    }
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

/// Picks the configured monitor, falling back to the primary one.
pub fn pick_monitor(index: Option<usize>) -> Option<Monitor> {
    let list = monitors();
    index
        .and_then(|i| list.get(i).copied())
        .or_else(|| list.iter().find(|m| m.primary).copied())
        .or_else(|| list.first().copied())
}

/// The monitor that shows most of `hwnd`.
pub fn monitor_of_window(hwnd: HWND) -> Option<Monitor> {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(monitor, &mut info)
            .as_bool()
            .then_some(Monitor { rect: info.rcMonitor, primary: info.dwFlags & MONITORINFOF_PRIMARY != 0 })
    }
}

/// True if `hwnd` fills its whole monitor, like a game in borderless or fullscreen mode.
pub fn fills_monitor(hwnd: HWND, monitor: &Monitor) -> bool {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
        && rect.left <= monitor.rect.left
        && rect.top <= monitor.rect.top
        && rect.right >= monitor.rect.right
        && rect.bottom >= monitor.rect.bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    #[test]
    fn desktop_window_fills_its_monitor() {
        let desktop = unsafe { GetDesktopWindow() };
        let monitor = monitor_of_window(desktop).expect("desktop has a monitor");
        assert!(monitors().contains(&monitor));
        assert!(fills_monitor(desktop, &monitor));
    }
}
