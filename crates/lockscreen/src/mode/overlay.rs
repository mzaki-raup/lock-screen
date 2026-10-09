//! The lock overlay.
//!
//! **Phase 01 is not finished.** What is here is a deliberately minimal
//! single-monitor overlay, built to host the unlock prompt so that the one
//! thing a prototype can actually settle gets settled: whether a modal dialog
//! reliably appears above, and keeps focus over, a window that is itself
//! `WS_EX_TOPMOST`. Everything else about the overlay is still to do:
//!
//! - one window per monitor from `EnumDisplayMonitors` (this covers one);
//! - `DWMWA_SYSTEMBACKDROP_TYPE` / `ACCENT_ENABLE_ACRYLICBLURBEHIND` for D1;
//! - the `WH_KEYBOARD_LL` / `WH_MOUSE_LL` hooks and key suppression (phase 02);
//! - the Task Manager policy guard.
//!
//! The last two are deliberately absent from the prototype: it installs no
//! hooks and writes nothing to the registry, so a prototype that misbehaves
//! cannot leave the machine in a worse state than it found it. Cancelling the
//! prompt exits rather than re-locking, for the same reason -- a prototype
//! should not be able to trap anyone.

use std::path::Path;

use lockscreen_core::config::Config;
use lockscreen_core::sound::{self, Sound};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HBRUSH};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LWA_ALPHA, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    SetLayeredWindowAttributes, ShowWindow, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::PCWSTR;

use crate::win::{self, ui, unlock};

pub fn run(_config_path: &Path, config: Config) -> Result<(), String> {
    let overlay = create_overlay(&config).ok_or("The overlay window could not be created.")?;

    sound::play(Sound::Started);

    let answer = unlock::ask(overlay, &config);

    // SAFETY: created above and not destroyed before now.
    unsafe {
        let _ = DestroyWindow(overlay);
    }

    match answer {
        unlock::Answer::Unlocked(via) => {
            let how = match via {
                unlock::Via::Hello => "Windows Hello",
                unlock::Via::Password => "the password",
                unlock::Via::Master => "the master password",
            };
            Err(format!(
                "Unlocked with {how}.\n\n\
                 That is as far as the prototype goes. Phase 01 turns this into the real overlay: \
                 one window per monitor, DWM blur, and the keyboard hooks that suppress \
                 Win, Alt+Tab and Ctrl+Shift+Esc."
            ))
        }
        unlock::Answer::Cancelled => Err("Cancelled. The real overlay would stay locked and re-prompt on the next \
                                          key or click; the prototype exits so it cannot trap you."
            .to_owned()),
    }
}

/// A single translucent, click-through-proof, always-on-top window over the
/// primary monitor.
///
/// `WS_EX_NOACTIVATE` matters more than it looks: the overlay must never take
/// the keyboard focus, because the dialog that sits on top of it needs it. That
/// is exactly the interaction this prototype exists to check.
fn create_overlay(config: &Config) -> Option<HWND> {
    let class = ui::wide("LockScreen.Overlay");
    let alpha = (config.transparency.clamp(0.0, 1.0) * 255.0) as u8;

    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_proc),
            hInstance: win::instance(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hbrBackground: HBRUSH(CreateSolidBrush(COLORREF(0)).0),
            ..Default::default()
        };
        RegisterClassW(&wc);

        // Physical pixels throughout. The manifest declares PerMonitorV2, so
        // GetSystemMetrics reports real pixels and there is no
        // device-independent unit anywhere to mix them up with -- which is the
        // whole of defect D2.
        let w = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(SM_CXSCREEN);
        let h = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(SM_CYSCREEN);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            w,
            h,
            None,
            None,
            Some(win::instance()),
            None,
        )
        .ok()?;

        // A uniform alpha over a black brush. Note what this is *not*: a blur.
        // Blurring a flat fill is a no-op, which is defect D1 -- the .NET
        // version applies a BlurEffect to exactly this and the setting has
        // never done anything.
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOW);
        Some(hwnd)
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
