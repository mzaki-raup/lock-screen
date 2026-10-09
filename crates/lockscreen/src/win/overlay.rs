//! The lock overlay: one window per monitor, for as long as the machine is
//! locked.
//!
//! # Shape
//!
//! A [`Session`] owns every overlay window and the Task Manager policy guard.
//! Dropping it takes the whole lock down, in order, on every path out of
//! [`Session::run`] including a panic. The .NET version instead calls
//! `EnableSystemKeys()` from three separate places, so an exit that misses all
//! three leaves Task Manager disabled on the user's machine with nothing in the
//! UI to explain why.
//!
//! # Two kinds of window
//!
//! Decided by `BlurBackground` before the window exists, because the two are
//! not interchangeable -- see [`super::backdrop`]. A layered window cannot
//! carry a DWM backdrop, so blur-on builds a plain composited window and paints
//! its own tint, and blur-off builds a layered window and lets the compositor
//! do the alpha.
//!
//! # Monitors coming and going
//!
//! `WM_DISPLAYCHANGE` is handled by re-enumerating and rebuilding, not just by
//! repositioning: a monitor plugged in while the machine is locked would
//! otherwise sit uncovered, which is a hole in the lock that the .NET version
//! has as well.
//!
//! # Input
//!
//! A `WS_EX_NOACTIVATE` window never holds the keyboard focus, so key events
//! never arrive here. [`super::hooks`] installs the global hooks that wake the
//! prompt on any key and swallow the combinations that would otherwise get past
//! the lock; the window procedure below still handles clicks landing on the
//! overlay itself, which is the cheaper path for the common case.

use std::cell::Cell;

use lockscreen_core::config::Config;
use lockscreen_core::policy::TaskManagerPolicy;
use lockscreen_core::sound::{self, Sound};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, HGDIOBJ, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, HWND_TOPMOST, LWA_ALPHA, MSG, PostMessageW, RegisterClassW, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOOWNERZORDER, SWP_NOSIZE, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_APP, WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_PAINT,
    WM_RBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::PCWSTR;

use super::{backdrop, hooks, monitors, ui, unlock};

/// Posted to an overlay window to bring the prompt up. Phase 02's hooks use
/// this; the window procedure sets the same flag directly for mouse clicks.
const WM_APP_WAKE: u32 = WM_APP + 10;

/// A cancel returning faster than this did not come from a person.
const INSTANT_CANCEL: std::time::Duration = std::time::Duration::from_millis(50);
/// How many of those in a row before giving up rather than spinning.
const MAX_INSTANT_CANCELS: u32 = 3;

/// How the lock ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Unlocked(unlock::Via),
    /// The loop ended without an unlock -- a `WM_QUIT`, which in practice means
    /// something asked the process to exit.
    Interrupted,
    /// The prompt could not be opened, repeatedly. Reported separately from
    /// [`Outcome::Interrupted`] because it is a bug to chase rather than
    /// something the user did, and because unwinding releases the input hooks.
    PromptFailed,
}

thread_local! {
    /// Something wants the prompt shown.
    static WAKE: Cell<bool> = const { Cell::new(false) };
    /// The monitor layout changed and the overlays need rebuilding.
    static DISPLAY_CHANGED: Cell<bool> = const { Cell::new(false) };
    /// The tint the blurred variant paints, as a `COLORREF` the window
    /// procedure can read without reaching the [`Session`].
    static TINT: Cell<u32> = const { Cell::new(0x0010_0C08) };
}

/// Every overlay window, plus the policy held while they are up.
pub struct Session {
    windows: Vec<HWND>,
    /// Restores `DisableTaskMgr` on drop. `None` if the policy could not be
    /// written, which must not stop the screen from locking.
    _policy: Option<TaskManagerPolicy>,
    /// Unhooks the keyboard and mouse on drop. `None` if Windows refused:
    /// an overlay without key suppression is weaker, but one that refuses to
    /// appear protects nothing at all.
    _hooks: Option<hooks::Hooks>,
    blurred: bool,
    alpha: u8,
}

impl Session {
    /// Cover every monitor and hold the lock.
    ///
    /// Returns once the user authenticates. Cancelling the prompt leaves the
    /// overlay up and waits for the next wake, which is what makes this a lock
    /// rather than a dialog.
    pub fn run(config: &Config) -> Result<Outcome, String> {
        let mut session = Self::create(config)?;
        sound::play(Sound::Started);
        let outcome = session.pump(config);
        // `session` drops here: windows destroyed, then the policy restored.
        Ok(outcome)
    }

    fn create(config: &Config) -> Result<Self, String> {
        let blurred = config.blur_background;
        let alpha = (config.transparency.clamp(0.0, 1.0) * 255.0).round() as u8;

        // The tint the blurred variant paints over the backdrop: darker the
        // more opaque the configured transparency asks for.
        let level = u32::from(255u8.saturating_sub(alpha)) / 5;
        TINT.set(level | (level << 8) | (level << 16));

        register_class();

        let mut session = Self { windows: Vec::new(), _policy: None, _hooks: None, blurred, alpha };
        session.build_windows()?;

        // Best-effort. A lock screen that refuses to appear because a registry
        // value would not write is worse than one with Task Manager reachable,
        // and on a managed machine group policy may own this key.
        session._policy = TaskManagerPolicy::disable().ok();

        // Installed on this thread, which owns the message loop: a low-level
        // hook is called on the installing thread and Windows must be able to
        // dispatch to it. Attached to the first overlay, which is the window
        // the wake message is posted to.
        if let Some(&first) = session.windows.first() {
            session._hooks = hooks::Hooks::install(first).ok();
        }
        Ok(session)
    }

    /// Create one window per attached monitor. Replaces whatever was there.
    fn build_windows(&mut self) -> Result<(), String> {
        self.destroy_windows();

        let screens = monitors::all();
        if screens.is_empty() {
            return Err("Windows reports no attached monitors, so there is nothing to lock.".to_owned());
        }

        let class = ui::wide("LockScreen.Overlay");
        for screen in &screens {
            // Layered only when not blurred. This is the fork
            // [`super::backdrop`] exists to explain: a layered window has no
            // DWM surface for a backdrop to go behind.
            let ex = if self.blurred {
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
            } else {
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
            };

            // SAFETY: the class is registered, and every coordinate is a
            // physical pixel from EnumDisplayMonitors.
            let created = unsafe {
                CreateWindowExW(
                    ex,
                    PCWSTR(class.as_ptr()),
                    PCWSTR::null(),
                    WS_POPUP | WS_VISIBLE,
                    screen.bounds.left,
                    screen.bounds.top,
                    screen.width(),
                    screen.height(),
                    None,
                    None,
                    Some(super::instance()),
                    None,
                )
            };
            let Ok(hwnd) = created else { continue };

            // Ask for the backdrop first: if it is refused this window has to
            // become layered instead, and that has to happen before it is shown
            // or there is a visible flash of a transparent hole.
            let mut layered = !self.blurred;
            if self.blurred && !backdrop::apply(hwnd) {
                // Windows 10, or Windows 11 before 22H2. Fall back to plain
                // translucency rather than leaving a hole over the desktop.
                //
                // The window was created *without* WS_EX_LAYERED, because a
                // layered window cannot carry a backdrop. SetLayeredWindowAttributes
                // does not quietly no-op on such a window -- it fails, and the
                // overlay would stay fully transparent: an invisible hole over
                // the desktop, which is worse than no lock at all. So the style
                // is added first.
                //
                // SAFETY: `hwnd` is live, and GWL_EXSTYLE takes the extended
                // style bits this window was created with.
                unsafe {
                    let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current | WS_EX_LAYERED.0 as isize);
                }
                layered = true;
            }
            if layered {
                // SAFETY: `hwnd` is live and now definitely has WS_EX_LAYERED,
                // either from creation or from the fallback above.
                unsafe {
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), self.alpha, LWA_ALPHA);
                }
            }

            // SAFETY: `hwnd` is live for both calls.
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                // Assert topmost explicitly after showing. Creating several
                // topmost windows in a row does not fix their order relative to
                // one another, and a monitor whose overlay slipped underneath
                // something is an uncovered monitor.
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    screen.bounds.left,
                    screen.bounds.top,
                    screen.width(),
                    screen.height(),
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
            self.windows.push(hwnd);
        }

        if self.windows.is_empty() {
            return Err("No overlay window could be created.".to_owned());
        }
        Ok(())
    }

    /// The message loop. Returns when the user authenticates.
    fn pump(&mut self, config: &Config) -> Outcome {
        let mut msg = MSG::default();
        let mut instant_cancels = 0u32;
        // The first prompt is immediate: the machine has just locked and
        // somebody is standing in front of it.
        WAKE.set(true);

        loop {
            // A monitor was attached, removed, or re-resolutioned. Rebuild
            // before prompting, so the prompt lands on a window that still
            // matches the hardware.
            if DISPLAY_CHANGED.replace(false) && self.build_windows().is_err() {
                // Every monitor went away. Nothing to cover and nowhere to
                // prompt; the lock has effectively ended.
                return Outcome::Interrupted;
            }

            if WAKE.replace(false) {
                let anchor = self.window_under_cursor();
                let started = std::time::Instant::now();
                match unlock::ask(anchor, config) {
                    unlock::Answer::Unlocked(via) => return Outcome::Unlocked(via),
                    unlock::Answer::Cancelled => {
                        // A cancel that comes back faster than any human could
                        // click is not a cancel -- it is the prompt failing to
                        // open. That matters more here than it looks: with the
                        // hooks installed, keystrokes are swallowed whenever no
                        // prompt is visible, so a prompt that cannot open would
                        // spin forever with the keyboard dead system-wide.
                        // Ctrl+Alt+Del would still work, but a lock screen
                        // should not need rescuing.
                        if started.elapsed() < INSTANT_CANCEL {
                            instant_cancels += 1;
                            if instant_cancels >= MAX_INSTANT_CANCELS {
                                return Outcome::PromptFailed;
                            }
                        } else {
                            instant_cancels = 0;
                        }
                        // Still locked. Re-assert and wait: the prompt is gone,
                        // the lock is not.
                        self.reassert();
                    }
                }
            }

            let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if got.0 <= 0 {
                return Outcome::Interrupted;
            }
            // SAFETY: `msg` was filled by GetMessageW.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// The overlay on the monitor the pointer is on, so the prompt appears
    /// where the person is looking. Falls back to the first overlay.
    fn window_under_cursor(&self) -> HWND {
        let mut point = POINT::default();
        // SAFETY: a plain out-parameter.
        unsafe {
            let _ = GetCursorPos(&mut point);
        }
        let screens = monitors::all();
        monitors::index_at(&screens, point.x, point.y)
            .and_then(|i| self.windows.get(i).copied())
            .unwrap_or_else(|| self.windows.first().copied().unwrap_or_default())
    }

    /// Put every overlay back on top and repaint it. Needed after the prompt
    /// closes: another topmost window -- a notification, say -- can otherwise
    /// end up above us.
    fn reassert(&self) {
        for &hwnd in &self.windows {
            // SAFETY: every handle is live until `destroy_windows`.
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOMOVE | SWP_NOSIZE,
                );
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
    }

    fn destroy_windows(&mut self) {
        for hwnd in self.windows.drain(..) {
            // SAFETY: created in `build_windows`, destroyed exactly once here.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.destroy_windows();
        // `_policy` drops after this, restoring DisableTaskMgr.
    }
}

/// Register the overlay window class. Idempotent: a second registration fails
/// harmlessly and the existing class is used.
fn register_class() {
    let class = ui::wide("LockScreen.Overlay");
    // SAFETY: a static wide name, and a procedure with the right signature.
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: super::instance(),
            lpszClassName: PCWSTR(class.as_ptr()),
            // No class background. The layered variant is filled by the
            // compositor and the blurred one paints its own, so letting Windows
            // erase first would flash.
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // A click anywhere on the overlay brings the prompt up. Keyboard waking
        // needs the phase 02 hooks: a NOACTIVATE window never holds the
        // keyboard focus, so key events do not arrive here. Mouse *movement*
        // deliberately does not wake -- a cursor resting on the overlay would
        // re-prompt on every stray pixel.
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_APP_WAKE => {
            WAKE.set(true);
            LRESULT(0)
        }

        // A monitor was added, removed, or changed resolution while locked.
        // The loop rebuilds every overlay rather than repositioning the ones it
        // has, because the count may have changed.
        WM_DISPLAYCHANGE => {
            DISPLAY_CHANGED.set(true);
            LRESULT(0)
        }

        WM_ERASEBKGND => LRESULT(1),

        // Only the blurred variant actually needs to paint; on a layered window
        // this fills with the same tint under a compositor alpha, which is
        // harmless and keeps one code path.
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: BeginPaint and EndPaint are paired, `rcPaint` is the
            // region Windows asked for, and the brush is deleted before return.
            unsafe {
                let hdc = BeginPaint(hwnd, &mut ps);
                if !hdc.is_invalid() {
                    let brush = HBRUSH(CreateSolidBrush(COLORREF(TINT.get())).0);
                    FillRect(hdc, &ps.rcPaint, brush);
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }

        // There is deliberately no WM_CLOSE arm. DefWindowProcW would destroy
        // the window and leave the desktop exposed with the process still
        // running; the overlay ends when the Session drops and no sooner.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Ask a running session to show the prompt.
///
/// Phase 02's keyboard hook calls this: the hook runs on its own callback and
/// must not touch window state directly.
#[allow(dead_code, reason = "used by the phase 02 input hooks")]
pub fn request_wake(overlay: HWND) {
    // SAFETY: a post to a window owned by this process; harmless if it is gone.
    unsafe {
        let _ = PostMessageW(Some(overlay), WM_APP_WAKE, WPARAM(0), LPARAM(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wake_flag_latches_and_clears_exactly_once() {
        WAKE.set(false);
        assert!(!WAKE.replace(false));
        WAKE.set(true);
        assert!(WAKE.replace(false), "a wake is observed");
        assert!(!WAKE.replace(false), "and only once");
    }

    #[test]
    fn the_display_change_flag_behaves_the_same_way() {
        DISPLAY_CHANGED.set(false);
        DISPLAY_CHANGED.set(true);
        assert!(DISPLAY_CHANGED.replace(false));
        assert!(!DISPLAY_CHANGED.replace(false));
    }

    #[test]
    fn the_tint_darkens_as_the_overlay_gets_more_opaque() {
        // alpha 255 (fully opaque) should give the darkest tint, alpha 0 the
        // lightest, with every channel equal so the overlay reads as grey.
        let tint_for = |alpha: u8| {
            let level = u32::from(255u8.saturating_sub(alpha)) / 5;
            (level | (level << 8) | (level << 16), level)
        };
        let (opaque, opaque_level) = tint_for(255);
        let (clear, clear_level) = tint_for(0);
        assert_eq!(opaque_level, 0, "a fully opaque overlay tints to black");
        assert!(clear_level > opaque_level);
        assert_eq!(opaque, 0);
        assert_eq!(clear & 0xFF, (clear >> 8) & 0xFF, "grey: red and green channels match");
        assert_eq!(clear & 0xFF, (clear >> 16) & 0xFF, "grey: red and blue channels match");
    }
}
