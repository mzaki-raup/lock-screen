//! Small Win32 helpers shared by every window.
//!
//! Ported from `flex-taskbar/src/win/ui.rs`, trimmed to what this app uses.
//! Kept as a separate module for the same reason the sibling does: every window
//! needs the same handful of operations, and DPI scaling in particular is the
//! sort of thing that goes wrong once it is open-coded in three places.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetMonitorInfoW, HFONT, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetCursorPos, GetWindowTextLengthW, GetWindowTextW, HMENU, MB_ICONERROR, MB_OK, MessageBoxW,
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SendMessageW, SetWindowTextW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT,
};
use windows::core::PCWSTR;

/// Static-control style for labels: `SS_NOPREFIX` (show `&` literally) and
/// `SS_ENDELLIPSIS` (truncate with an ellipsis rather than clipping).
pub const SS_LABEL: WINDOW_STYLE = WINDOW_STYLE(0x0080 | 0x4000);

/// `ES_PASSWORD`: the edit control shows a bullet per character. The masking is
/// done by the control, so the plaintext never passes through our paint code.
pub const ES_PASSWORD: WINDOW_STYLE = WINDOW_STYLE(0x0020);

/// `ES_AUTOHSCROLL`: scroll horizontally rather than refusing further input.
pub const ES_AUTOHSCROLL: WINDOW_STYLE = WINDOW_STYLE(0x0080);

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

/// The DPI of the monitor a window is on, or 96 if Windows will not say.
pub fn dpi_of(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 96 } else { dpi }
}

/// Scale a 96-DPI design measurement to the given DPI.
pub fn scale(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}

/// The system message font at the given DPI.
///
/// Asking for it per-DPI rather than scaling one font is what makes text crisp
/// on a 150% monitor instead of merely bigger.
pub fn message_font(dpi: u32) -> HFONT {
    unsafe {
        let mut ncm =
            NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            ncm.cbSize,
            Some(&mut ncm as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0).0,
            dpi,
        );
        CreateFontIndirectW(&ncm.lfMessageFont)
    }
}

pub fn delete_font(font: HFONT) {
    if !font.is_invalid() {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(windows::Win32::Foundation::WPARAM(font.0 as usize)),
            Some(windows::Win32::Foundation::LPARAM(1)),
        );
    }
}

/// Create a child control. Position is set afterwards, once the layout is known.
pub fn child(parent: HWND, class: &str, text: &str, style: WINDOW_STYLE, ex_style: WINDOW_EX_STYLE, id: u16) -> HWND {
    let class_w = wide(class);
    let text_w = wide(text);
    unsafe {
        CreateWindowExW(
            ex_style,
            PCWSTR(class_w.as_ptr()),
            PCWSTR(text_w.as_ptr()),
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            None,
            None,
        )
        .unwrap_or_default()
    }
}

/// Read a control's text.
///
/// For the password field this is the one place the plaintext is materialised,
/// which is why the caller wraps it in `Zeroizing` immediately.
pub fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        let text = String::from_utf16_lossy(&buf[..n as usize]);
        // The wide buffer held the password too; clear it before it is freed.
        buf.fill(0);
        text
    }
}

pub fn set_text(hwnd: HWND, text: &str) {
    let w = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
    }
}

pub fn error(owner: Option<HWND>, text: &str) {
    let t = wide(text);
    let c = wide("Lock Screen");
    unsafe {
        MessageBoxW(owner, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONERROR);
    }
}

pub fn rect_w(r: &RECT) -> i32 {
    r.right - r.left
}

pub fn rect_h(r: &RECT) -> i32 {
    r.bottom - r.top
}

/// Work area of the monitor under the pointer.
pub fn work_area_at_cursor() -> RECT {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

/// Move a `w`x`h` window at `x`,`y` back inside the work area of the monitor it
/// lands on, so a dialog centred over a multi-monitor overlay cannot end up
/// half off the screen.
pub fn keep_on_screen(x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let centre = POINT { x: x + w / 2, y: y + h / 2 };
        let mon = MonitorFromPoint(centre, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        // A window too big for the work area starts at its top-left corner.
        (x.min(wa.right - w).max(wa.left), y.min(wa.bottom - h).max(wa.top))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_matches_the_usual_windows_steps() {
        assert_eq!(scale(100, 96), 100);
        assert_eq!(scale(100, 120), 125); // 125%
        assert_eq!(scale(100, 144), 150); // 150%
        assert_eq!(scale(100, 192), 200); // 200%
    }

    #[test]
    fn wide_strings_are_nul_terminated() {
        assert_eq!(wide("ok"), vec![b'o' as u16, b'k' as u16, 0]);
        assert_eq!(wide(""), vec![0]);
    }

    #[test]
    fn loword_takes_the_low_half() {
        assert_eq!(loword(0x0002_0001), 1);
        assert_eq!(loword(0xFFFF), 0xFFFF);
    }

    #[test]
    fn rect_dimensions_are_right_minus_left() {
        let r = RECT { left: 10, top: 20, right: 110, bottom: 220 };
        assert_eq!(rect_w(&r), 100);
        assert_eq!(rect_h(&r), 200);
    }

    #[test]
    fn a_window_larger_than_the_screen_is_pinned_to_the_top_left() {
        // Whatever this machine's monitors are, an absurdly large window must
        // come back at or before the work area's origin, never pushed negative
        // past it by the min/max ordering.
        let (x, y) = keep_on_screen(0, 0, 1 << 20, 1 << 20);
        let wa = work_area_at_cursor();
        assert_eq!((x, y), (wa.left, wa.top));
    }
}
