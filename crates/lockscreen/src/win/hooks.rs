//! Global input hooks: waking the prompt, and swallowing the keys that would
//! otherwise get past the lock.
//!
//! Replaces `SharpHook` and the 693 KB `uiohook.dll` it ships, with the two
//! Win32 calls that library wraps.
//!
//! # Defect D3
//!
//! `MainWindow.xaml.cs:30-36` declares `SetWindowsHookEx` as returning `int`
//! and `UnhookWindowsHookEx` as taking one. `HHOOK` is a 64-bit pointer on
//! x64, so that works today only because the values happen to fit in 32 bits.
//! A truncated handle means the unhook fails and a **system-wide keyboard hook
//! survives the unlock**. Here `SetWindowsHookExW` is typed
//! `-> Result<HHOOK>`, the signature generated from Windows metadata, and
//! [`Hooks`] unhooks on drop so there is no path that forgets.
//!
//! # Defect D4
//!
//! The same file asks WPF's `Keyboard.Modifiers` for Ctrl/Alt/Shift/Win state
//! from inside a low-level hook. That reflects the WPF input manager for the
//! focused element, not the physical keyboard, so in a process whose overlay
//! never takes focus it is routinely stale -- which is why Alt+Tab suppression
//! has never been reliable. Alt comes from the hook's own
//! [`LLKHF_ALTDOWN`](KBDLLHOOKSTRUCT_FLAGS) flag, and the rest from
//! `GetAsyncKeyState`, which reports the physical key.
//!
//! # What cannot be suppressed
//!
//! `Ctrl+Alt+Del` is a Secure Attention Sequence handled by win32k before any
//! hook runs. The .NET code tries to swallow it at line 154 and that branch has
//! never executed. It is not reimplemented here; [`should_suppress`] documents
//! the gap instead of pretending to close it.
//!
//! # Speed
//!
//! Windows silently removes a low-level hook whose callback overruns
//! `LowLevelHooksTimeout`. Both callbacks here set an atomic and post a
//! message; nothing else happens on the hook thread.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_F4, VK_LWIN, VK_RWIN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
    WM_XBUTTONDOWN,
};

use super::overlay;

/// The overlay to wake, as a raw handle so it can live in a static. Zero when
/// no session is running.
static TARGET: AtomicIsize = AtomicIsize::new(0);

/// Whether the unlock prompt is on screen.
///
/// Decides whether ordinary keystrokes are passed through or swallowed. While
/// the prompt is up they must pass: the user is typing a password into it, and
/// Windows Hello's PIN entry needs them too.
static PROMPT_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Which modifiers are physically held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Modifiers {
    /// Read the real keyboard.
    ///
    /// `alt` is taken from the hook's own flag rather than `GetAsyncKeyState`:
    /// a low-level hook runs *before* the key reaches the input queue, so for
    /// the keystroke being delivered right now the async state can lag, and Alt
    /// is the one modifier that commonly arrives in the same event as the key
    /// it modifies (`WM_SYSKEYDOWN`).
    fn current(alt_from_hook: bool) -> Self {
        // SAFETY: GetAsyncKeyState takes a virtual-key code and touches no
        // memory of ours.
        let down = |vk: u16| unsafe { GetAsyncKeyState(i32::from(vk)) as u16 & 0x8000 != 0 };
        Self {
            ctrl: down(VK_CONTROL.0),
            alt: alt_from_hook || down(windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU.0),
            shift: down(VK_SHIFT.0),
            win: down(VK_LWIN.0) || down(VK_RWIN.0),
        }
    }
}

/// Whether this keystroke is one that would get past the lock, and so must be
/// swallowed whatever else is happening.
///
/// Pure, so the list is testable without installing a hook. Everything here is
/// a way out of the lock; ordinary typing is decided separately by
/// [`PROMPT_VISIBLE`].
pub fn should_suppress(vk: u16, mods: Modifiers) -> bool {
    // The Windows key on its own opens Start, and with almost any letter opens
    // something else. Swallowed unconditionally rather than per-combination.
    if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
        return true;
    }
    // Ctrl+Esc is the Windows key by another name, and the .NET list misses it
    // -- so a machine "locked" by the current build is one keystroke from the
    // Start menu.
    if mods.ctrl && !mods.alt && vk == VK_ESCAPE.0 {
        return true;
    }
    // Task Manager. The registry policy makes it refuse to open anyway, but
    // that policy is removable by the same user it restricts, so both.
    if mods.ctrl && mods.shift && vk == VK_ESCAPE.0 {
        return true;
    }
    if mods.alt {
        return match vk {
            // Switch away from the lock.
            v if v == VK_TAB.0 => true,
            // Close the lock.
            v if v == VK_F4.0 => true,
            // Alt+Esc cycles windows. Also missing from the .NET list.
            v if v == VK_ESCAPE.0 => true,
            _ => false,
        };
    }
    // Deliberately absent: Ctrl+Alt+Del. The Secure Attention Sequence is
    // handled by win32k before any hook runs, so no return value here can stop
    // it. Sign out, Switch user and the real Windows lock stay reachable, and
    // the study says so rather than implying otherwise.
    false
}

/// Both hooks, unhooked on drop.
///
/// Must be created on a thread with a message loop -- low-level hooks are
/// called on the installing thread, and Windows needs to be able to dispatch to
/// it. The overlay's thread qualifies.
pub struct Hooks {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl Hooks {
    /// Install both hooks, waking `overlay` when input arrives.
    ///
    /// # Errors
    ///
    /// Fails if Windows refuses the hook, which in practice means another
    /// process holds too many or the desktop is locked down. The caller should
    /// still lock the screen: an overlay with no key suppression is weaker, but
    /// an overlay that refuses to appear protects nothing at all.
    pub fn install(overlay: HWND) -> windows::core::Result<Self> {
        TARGET.store(overlay.0 as isize, Ordering::Relaxed);
        // SAFETY: both callbacks have the required signature and live for the
        // program's lifetime; a null module handle is correct for a hook
        // procedure in this process with a thread id of 0 (all threads).
        let (keyboard, mouse) = unsafe {
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0)?;
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
                Ok(mouse) => (keyboard, mouse),
                Err(e) => {
                    // Do not leak the one that succeeded.
                    let _ = UnhookWindowsHookEx(keyboard);
                    return Err(e);
                }
            }
        };
        Ok(Self { keyboard, mouse })
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: both handles came from SetWindowsHookExW and are unhooked
        // exactly once, here. Typed as HHOOK end to end, so there is no 32-bit
        // truncation to lose them to -- defect D3.
        unsafe {
            let _ = UnhookWindowsHookEx(self.keyboard);
            let _ = UnhookWindowsHookEx(self.mouse);
        }
        TARGET.store(0, Ordering::Relaxed);
    }
}

/// Marks the prompt as on screen for as long as it lives.
///
/// An RAII guard rather than a pair of calls because a stuck `true` means the
/// lock stops swallowing keys -- a silent downgrade of the thing the hooks
/// exist to do. `Drop` runs on early returns and unwinds alike.
pub struct PromptVisible;

impl PromptVisible {
    pub fn new() -> Self {
        PROMPT_VISIBLE.store(true, Ordering::Relaxed);
        Self
    }
}

impl Drop for PromptVisible {
    fn drop(&mut self) {
        PROMPT_VISIBLE.store(false, Ordering::Relaxed);
    }
}

/// Whether an input event should bring the prompt up.
///
/// One rule for both hooks, because they had the event list twice and only one
/// of them had the prompt check -- which is to say neither did, since the
/// keyboard hook called `wake()` before looking.
///
/// **Nothing wakes a prompt that is already up.** Those events are the password
/// being typed into it, the PIN going into Hello, or the Escape that dismisses
/// it. Counted as wakes they left the flag set, so `pump` reopened the prompt
/// the instant it closed: cancelling appeared to do nothing at all. The
/// keystroke that cancelled it had already asked for a new one. Fixed in 0.2.6.
///
/// Press events only. A key-up would otherwise reopen the prompt as the
/// cancelling keystroke was released, and a mouse-move would reopen it by
/// moving the mouse.
fn wakes_the_prompt(message: u32, prompt_visible: bool) -> bool {
    if prompt_visible {
        return false;
    }
    matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN)
}

/// Wake the overlay, if a session is running.
fn wake() {
    let target = TARGET.load(Ordering::Relaxed);
    if target != 0 {
        overlay::request_wake(HWND(target as *mut _));
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Negative codes must be passed straight through, untouched.
    if code != HC_ACTION as i32 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let message = wparam.0 as u32;
    if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
        // SAFETY: for HC_ACTION on WH_KEYBOARD_LL, lparam points at a
        // KBDLLHOOKSTRUCT owned by the OS for the duration of this call.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let vk = event.vkCode as u16;
        let mods = Modifiers::current(event.flags.contains(LLKHF_ALTDOWN));

        if should_suppress(vk, mods) {
            // Swallowed: do not chain, or the next hook in the line gets it.
            return LRESULT(1);
        }

        if wakes_the_prompt(message, PROMPT_VISIBLE.load(Ordering::Relaxed)) {
            wake();
        }

        // Ordinary typing. While the prompt is up it has the focus and needs
        // the keystroke -- including Hello's PIN entry. While it is down, the
        // overlay is WS_EX_NOACTIVATE and holds no focus, so anything passed
        // through would land in whatever window is behind the lock. Swallow it.
        if !PROMPT_VISIBLE.load(Ordering::Relaxed) {
            return LRESULT(1);
        }
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let message = wparam.0 as u32;
    if wakes_the_prompt(message, PROMPT_VISIBLE.load(Ordering::Relaxed)) {
        wake();
    }

    // Never suppressed. The prompt needs clicks, and so does Windows Hello's
    // consent dialog, which is a system window this process does not own --
    // swallowing mouse input globally would make the biometric prompt
    // unclickable and could strand someone at their own lock screen. The
    // overlay covers every monitor, so there is nothing underneath to click
    // anyway; a click that lands on another topmost window is the one case
    // this hook catches that the overlay's own messages do not.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers { ctrl: false, alt: false, shift: false, win: false };
    const ALT: Modifiers = Modifiers { ctrl: false, alt: true, shift: false, win: false };
    const CTRL: Modifiers = Modifiers { ctrl: true, alt: false, shift: false, win: false };
    const CTRL_SHIFT: Modifiers = Modifiers { ctrl: true, alt: false, shift: true, win: false };

    /// `A`, as an ordinary key that must always get through to a focused prompt.
    const VK_A: u16 = 0x41;

    #[test]
    fn the_windows_key_is_swallowed_whatever_is_held() {
        for mods in [NONE, ALT, CTRL, CTRL_SHIFT] {
            assert!(should_suppress(VK_LWIN.0, mods), "left win with {mods:?}");
            assert!(should_suppress(VK_RWIN.0, mods), "right win with {mods:?}");
        }
    }

    #[test]
    fn the_combinations_that_escape_the_lock_are_swallowed() {
        assert!(should_suppress(VK_TAB.0, ALT), "Alt+Tab switches away");
        assert!(should_suppress(VK_F4.0, ALT), "Alt+F4 closes the lock");
        assert!(should_suppress(VK_ESCAPE.0, ALT), "Alt+Esc cycles windows");
        assert!(should_suppress(VK_ESCAPE.0, CTRL_SHIFT), "Ctrl+Shift+Esc is Task Manager");
        assert!(should_suppress(VK_ESCAPE.0, CTRL), "Ctrl+Esc is the Start menu");
    }

    #[test]
    fn the_two_gaps_in_the_dotnet_list_are_closed() {
        // The .NET hook blocks the Windows key but not Ctrl+Esc, which opens
        // the same Start menu, nor Alt+Esc. Both are one keystroke past a lock
        // that build considers secure.
        assert!(should_suppress(VK_ESCAPE.0, CTRL));
        assert!(should_suppress(VK_ESCAPE.0, ALT));
    }

    #[test]
    fn ordinary_typing_is_never_suppressed_by_the_combination_list() {
        // Whether it reaches anything is PROMPT_VISIBLE's decision, not this
        // function's. Getting that wrong would stop someone typing a password.
        assert!(!should_suppress(VK_A, NONE));
        assert!(!should_suppress(VK_A, CTRL), "Ctrl+A selects all in the password box");
        assert!(!should_suppress(VK_A, CTRL_SHIFT));
        assert!(!should_suppress(VK_TAB.0, NONE), "Tab moves between the prompt's controls");
        assert!(!should_suppress(VK_ESCAPE.0, NONE), "Escape cancels the prompt");
    }

    #[test]
    fn tab_and_escape_alone_stay_usable_because_the_prompt_needs_them() {
        // Suppressing bare Tab would break keyboard navigation of the prompt,
        // and bare Escape would make Cancel unreachable without a mouse.
        assert!(!should_suppress(VK_TAB.0, NONE));
        assert!(!should_suppress(VK_ESCAPE.0, NONE));
        // But with Alt held, both are escape routes.
        assert!(should_suppress(VK_TAB.0, ALT));
        assert!(should_suppress(VK_ESCAPE.0, ALT));
    }

    #[test]
    fn ctrl_alt_delete_is_not_claimed_to_be_suppressed() {
        // 0x2E is VK_DELETE. The .NET code has a branch for this at line 154
        // which has never run: the SAS never reaches a hook. Asserting the
        // honest answer keeps anyone from "fixing" it back.
        let ctrl_alt = Modifiers { ctrl: true, alt: true, shift: false, win: false };
        assert!(!should_suppress(0x2E, ctrl_alt), "no hook can swallow the SAS; do not pretend");
    }

    #[test]
    fn nothing_wakes_a_prompt_that_is_already_up() {
        // The defect this rule exists for: every keystroke typed into the
        // prompt, and the Escape that dismissed it, counted as a request for a
        // new prompt. Cancelling therefore appeared to do nothing -- the
        // prompt came straight back.
        for message in [WM_KEYDOWN, WM_SYSKEYDOWN, WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_MBUTTONDOWN, WM_XBUTTONDOWN] {
            assert!(wakes_the_prompt(message, false), "{message:#x} should wake a bare overlay");
            assert!(!wakes_the_prompt(message, true), "{message:#x} must not wake a prompt that is up");
        }
    }

    #[test]
    fn only_press_events_wake_the_prompt() {
        use windows::Win32::UI::WindowsAndMessaging::{WM_KEYUP, WM_MOUSEMOVE, WM_SYSKEYUP};
        // A key-up would reopen the prompt as the cancelling keystroke was
        // released, which is the same bug one event later. A mouse-move would
        // reopen it by moving the mouse.
        for message in [WM_KEYUP, WM_SYSKEYUP, WM_MOUSEMOVE] {
            assert!(!wakes_the_prompt(message, false), "{message:#x} is not a press");
            assert!(!wakes_the_prompt(message, true), "{message:#x} is not a press");
        }
    }

    #[test]
    fn the_prompt_visible_guard_clears_itself() {
        PROMPT_VISIBLE.store(false, Ordering::Relaxed);
        {
            let _guard = PromptVisible::new();
            assert!(PROMPT_VISIBLE.load(Ordering::Relaxed), "set while the prompt is up");
        }
        assert!(!PROMPT_VISIBLE.load(Ordering::Relaxed), "and cleared when it goes, on every path");
    }

    #[test]
    fn waking_with_no_session_running_is_harmless() {
        TARGET.store(0, Ordering::Relaxed);
        wake();
    }
}
