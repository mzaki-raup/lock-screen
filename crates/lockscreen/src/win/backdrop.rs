//! Blurring the desktop behind the overlay. This is the fix for defect D1.
//!
//! What the .NET version does is a no-op: `MainWindow.xaml.cs:191` applies a
//! WPF `BlurEffect` to `OverlayGrid`, whose only content is a uniform
//! `SolidColorBrush`. Blurring a flat fill produces the same flat fill, so the
//! setting has been exposed in SettingsApp for the life of the product and has
//! never once done anything. Blurring what is *behind* a window is a different
//! mechanism entirely, which is why this only gets fixed by someone rewriting
//! the overlay.
//!
//! # The constraint that shapes the overlay
//!
//! A DWM backdrop is drawn by the compositor *behind* the window, and the
//! window is then painted over it. That requires an ordinary composited window.
//! A **layered** window -- `WS_EX_LAYERED` with `SetLayeredWindowAttributes`,
//! which is how the plain translucent overlay gets its alpha -- has no DWM
//! surface to put a backdrop behind, and asking for one there replaces what is
//! behind the window instead of blurring it.
//!
//! So the overlay is built one of two ways, decided by the config before the
//! window exists (see [`super::overlay`]):
//!
//! | `BlurBackground` | Window | Tint |
//! | --- | --- | --- |
//! | `false` | `WS_EX_LAYERED` + `SetLayeredWindowAttributes` | uniform alpha, by the compositor |
//! | `true` | plain composited + `DwmExtendFrameIntoClientArea(-1)` | painted in `WM_PAINT` over the backdrop |
//!
//! This was learned from `flex-taskbar/src/win/backdrop.rs`, which documents
//! the same trap for the same reason.
//!
//! # Where it works
//!
//! `DWMWA_SYSTEMBACKDROP_TYPE` arrived in **Windows 11 22H2**. Earlier systems
//! return an error from `DwmSetWindowAttribute`, which is how [`available`]
//! decides whether to offer it at all.
//!
//! The migration study promised a second path for Windows 10 1809+ using
//! `ACCENT_ENABLE_ACRYLICBLURBEHIND`. **That path is not taken, deliberately.**
//! `SetWindowCompositionAttribute` is undocumented and is not in `windows-rs`,
//! so using it means hand-writing the `ACCENT_POLICY` layout and linking the
//! export by hand -- which is the same hand-transcription risk as defect D3, the
//! 64-bit handle truncation this port exists to remove, in the one process that
//! covers the screen when the machine is unattended. A cosmetic effect is not
//! worth that. On Windows 10, and on Windows 11 before 22H2, the blur setting
//! degrades to plain translucency and [`available`] reports `false` so the UI
//! can say so rather than pretending -- which is the actual sin of D1.

use std::sync::OnceLock;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TABBEDWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::MARGINS;

/// Whether this Windows can draw a backdrop at all.
///
/// Asked once, of a window the caller already has: setting the attribute to
/// "none" changes nothing but reports whether the attribute is known.
pub fn available(probe: HWND) -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        if probe.is_invalid() {
            return false;
        }
        let value = DWMSBT_NONE.0 as u32;
        // SAFETY: `probe` is a live window, and the pointer and size describe a
        // `u32` that outlives the call.
        unsafe {
            DwmSetWindowAttribute(
                probe,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &raw const value as *const _,
                std::mem::size_of::<u32>() as u32,
            )
        }
        .is_ok()
    })
}

/// Put a blurred backdrop behind `hwnd`.
///
/// Returns whether it took. A caller that gets `false` must go on painting its
/// own opaque-enough background, because the window will otherwise be a hole.
///
/// `hwnd` must **not** be `WS_EX_LAYERED` -- see the module documentation.
pub fn apply(hwnd: HWND) -> bool {
    if hwnd.is_invalid() || !available(hwnd) {
        return false;
    }

    // SAFETY: every pointer below addresses a local that outlives its call, and
    // the sizes match the types.
    unsafe {
        // The overlay is always dark, whatever the system theme: it is a dimmed
        // screen, and a light-tinted blur over someone's desktop reads as a
        // rendering fault rather than a lock.
        let dark = windows::core::BOOL::from(true);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &raw const dark as *const _,
            std::mem::size_of::<windows::core::BOOL>() as u32,
        );

        // TABBEDWINDOW is the most heavily blurred of the backdrop types, which
        // is what a lock screen wants -- MAINWINDOW (Mica) is deliberately
        // subtle and barely obscures anything, and obscuring the desktop is the
        // entire point here.
        let kind = DWMSBT_TABBEDWINDOW.0 as u32;
        if DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &raw const kind as *const _,
            std::mem::size_of::<u32>() as u32,
        )
        .is_err()
        {
            return false;
        }

        // Margins of -1 make the whole client area glass, so the backdrop
        // reaches every pixel rather than just a frame.
        let all = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        if DwmExtendFrameIntoClientArea(hwnd, &all).is_err() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_window_is_refused_rather_than_passed_to_dwm() {
        assert!(!apply(HWND::default()));
        assert!(!available(HWND::default()));
    }
}
