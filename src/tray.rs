//! Notification area icon, built directly on Shell_NotifyIconW.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC,
    ReleaseDC,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

use crate::logo;
use crate::render;

const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    callback_msg: u32,
    icon: Option<HICON>,
    tip: String,
}

impl Tray {
    pub fn new(hwnd: HWND, callback_msg: u32) -> Self {
        Self { hwnd, callback_msg, icon: None, tip: "reticle".into() }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    /// Adds the icon (again). Also used after Explorer restarts.
    pub fn add(&self) {
        let mut nid = self.data();
        nid.uFlags = NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = self.callback_msg;
        copy_wide(&mut nid.szTip, &self.tip);
        if let Some(icon) = self.icon {
            nid.uFlags |= NIF_ICON;
            nid.hIcon = icon;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    pub fn remove(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
        }
    }

    /// Updates icon state and tooltip.
    pub fn set_state(&mut self, shown: bool, tip: &str) {
        let icon = make_icon(shown);
        self.tip = tip.to_string();
        let mut nid = self.data();
        nid.uFlags = NIF_TIP;
        copy_wide(&mut nid.szTip, tip);
        if let Some(icon) = icon {
            nid.uFlags |= NIF_ICON;
            nid.hIcon = icon;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
        if let Some(icon) = icon
            && let Some(old) = self.icon.replace(icon) {
                unsafe {
                    let _ = DestroyIcon(old);
                }
            }
    }

    pub fn notify(&self, title: &str, text: &str, warning: bool) {
        let mut nid = self.data();
        nid.uFlags = NIF_INFO;
        copy_wide(&mut nid.szInfoTitle, title);
        copy_wide(&mut nid.szInfo, text);
        nid.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        if let Some(icon) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for unit in s.encode_utf16().take(dst.len() - 1) {
        dst[n] = unit;
        n += 1;
    }
    dst[n] = 0;
}

/// The "r." logo; the dot is green while the crosshair is visible, grey while hidden.
fn make_icon(shown: bool) -> Option<HICON> {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let pixmap = logo::render(size as u32, if shown { logo::ACCENT } else { logo::MUTED }, true);
    let pixels = render::to_bgra_straight(&pixmap);
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let screen = GetDC(None);
        let mut bits = std::ptr::null_mut();
        let color_bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, screen);
        let color_bitmap = color_bitmap.ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color_bitmap,
            ..Default::default()
        });
        let _ = DeleteObject(color_bitmap.into());
        let _ = DeleteObject(mask.into());
        icon.ok()
    }
}
