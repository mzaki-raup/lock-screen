//! The unlock prompt: a modal window over the overlay.
//!
//! Follows the sibling's `prompt.rs` pattern -- register a class, create a
//! popup owned by the caller's window, lay children out at the owner's DPI,
//! disable the owner, and run a local message loop until an answer arrives.
//! `IsDialogMessageW` is what gives Tab, Enter and Escape their usual meaning
//! without hand-written key handling.
//!
//! Three things here are specific to a lock screen rather than an ordinary
//! dialog:
//!
//! 1. **It cannot be dismissed into nothing.** The overlay is still up behind
//!    it, so closing without authenticating returns [`Answer::Cancelled`] and
//!    the machine stays locked. `WM_CLOSE` is answered rather than vetoed: the
//!    lock is enforced by the overlay outliving the dialog, not by trapping the
//!    user inside a window. The .NET version cancels the close and
//!    re-activates, which is how a dialog ends up feeling unclosable.
//! 2. **Windows Hello runs first, off-thread.** The consent dialog needs our
//!    message loop to keep pumping, so the blocking call lives on a worker and
//!    the answer arrives over a channel the loop polls.
//! 3. **The password never outlives the comparison.** It is read into a
//!    `Zeroizing<String>`, verified, and dropped; the control is cleared after
//!    every failed attempt.

use std::sync::mpsc::Receiver;

use lockscreen_core::config::Config;
use lockscreen_core::credential;
use lockscreen_core::sound::{self, Sound};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush, HBRUSH};
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowRect, IDCANCEL, IDOK, IsDialogMessageW, IsWindowVisible, MSG, PostQuitMessage, RegisterClassW, SW_HIDE,
    SW_SHOW, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_DLGMODALFRAME, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::PCWSTR;
use zeroize::Zeroizing;

use super::{hello, hooks, ui, voice};

const ID_PASSWORD: u16 = 10;
const ID_RETRY_HELLO: u16 = 11;
const ID_USE_MASTER: u16 = 12;
const ID_VOICE: u16 = 13;

/// Posted to the dialog when the Hello worker has an answer.
const WM_APP_HELLO: u32 = WM_APP + 1;
/// Posted when the voice worker has an answer.
const WM_APP_VOICE: u32 = WM_APP + 2;

/// How the prompt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Authenticated. Which credential is carried so a master-password unlock
    /// can be audited.
    Unlocked(Via),
    /// Dismissed without authenticating. The overlay stays up.
    Cancelled,
}

/// Which rung of the ladder let the user in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Hello,
    Password,
    /// Spoken command. Recorded separately from a password because it is not
    /// equivalent to one: recognition is speaker-independent, so this says a
    /// phrase was heard, not who said it.
    Voice,
    Master,
}

/// After this many wrong passwords the prompt slows down. A lock screen should
/// not be a fast oracle. The master password, being a recovery credential,
/// slows from the first mistake.
const SLOW_AFTER: u32 = 3;
const SLOW_BY: std::time::Duration = std::time::Duration::from_millis(750);

struct State {
    password_box: HWND,
    status: HWND,
    retry_hello: HWND,
    use_master: HWND,
    /// Stored verifiers. Hashes, not secrets.
    password_hash: Option<String>,
    master_hash: Option<String>,
    /// Whether the master-password rung is being entered.
    master_mode: bool,
    attempts: u32,
    answer: Option<Answer>,
    /// In flight, if any. Held here rather than in `ask`'s locals so that a
    /// retry started from the window procedure is visible to the loop.
    hello_rx: Option<Receiver<hello::Outcome>>,
    /// The voice button, when voice unlock is switched on. `None` otherwise,
    /// and then no row was laid out for it at all.
    voice_button: Option<HWND>,
    voice_rx: Option<Receiver<voice::Outcome>>,
    /// What the recogniser listens for, and how sure it has to be. Copied from
    /// the config so the worker does not borrow it.
    voice_command: String,
    voice_required: voice::Confidence,
    voice_patience: std::time::Duration,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// Show the prompt over `owner` and block until the user answers.
///
/// `owner` is the overlay window: disabled while the prompt is up, re-enabled
/// afterwards. The prompt is `WS_EX_TOPMOST` so it sits above an overlay that
/// is itself topmost.
pub fn ask(owner: HWND, config: &Config) -> Answer {
    if STATE.with(|s| s.borrow().is_some()) {
        return Answer::Cancelled;
    }

    // Tells the keyboard hook to pass ordinary keystrokes through: this window
    // has the focus and the user is about to type a password into it, and
    // Windows Hello's PIN entry needs them too. Dropped on every exit path, so
    // the lock cannot be left in a state where it swallows nothing.
    let _typing = hooks::PromptVisible::new();

    let dpi = ui::dpi_of(owner);
    let s = |v: i32| ui::scale(v, dpi);
    let font = ui::message_font(dpi);
    let class = ui::wide("LockScreen.Unlock");
    let title = ui::wide(&config.title);
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;

    let hwnd = unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: super::instance(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hbrBackground: HBRUSH(GetSysColorBrush(COLOR_WINDOW).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
        match CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            0,
            0,
            100,
            100,
            Some(owner),
            None,
            Some(super::instance()),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => {
                ui::delete_font(font);
                return Answer::Cancelled;
            }
        }
    };

    // ---- layout, all at the owner's DPI ----
    let pad = s(16);
    let gap = s(8);
    let field_h = s(26);
    let button_h = s(28);
    let wide_button = s(180);
    let button_w = s(140);
    let content_w = s(368);
    let mut y = pad;

    let add = |id: u16, class: &str, text: &str, st: WINDOW_STYLE, r: (i32, i32, i32, i32)| {
        let c = ui::child(hwnd, class, text, WS_CHILD | WS_VISIBLE | st, WINDOW_EX_STYLE(0), id);
        unsafe {
            let _ = SetWindowPos(c, None, r.0, r.1, r.2, r.3, SWP_NOZORDER);
        }
        ui::set_font(c, font);
        c
    };

    let status = add(0, "STATIC", "", ui::SS_LABEL, (pad, y, content_w, s(36)));
    y += s(36) + gap;

    let password_box = add(
        ID_PASSWORD,
        "EDIT",
        "",
        WS_TABSTOP | WS_BORDER | ui::ES_PASSWORD | ui::ES_AUTOHSCROLL,
        (pad, y, content_w, field_h),
    );
    y += field_h + gap + s(4);

    let retry_hello = add(ID_RETRY_HELLO, "BUTTON", "Retry Windows Hello", WS_TABSTOP, (pad, y, wide_button, button_h));
    let use_master = add(
        ID_USE_MASTER,
        "BUTTON",
        "Use master password",
        WS_TABSTOP,
        (pad + wide_button + gap, y, wide_button, button_h),
    );
    y += button_h + gap;

    // Only laid out when voice unlock is switched on, so a dialog that cannot
    // use it is not left with a gap where the button would have been.
    let voice_button = if config.voice.enabled {
        let b = add(ID_VOICE, "BUTTON", "Unlock by voice", WS_TABSTOP, (pad, y, wide_button, button_h));
        y += button_h + gap;
        Some(b)
    } else {
        None
    };
    y += s(6);

    add(
        IDOK.0 as u16,
        "BUTTON",
        "Unlock",
        WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
        (content_w + pad - button_w * 2 - gap, y, button_w, button_h),
    );
    add(
        IDCANCEL.0 as u16,
        "BUTTON",
        "Cancel",
        WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        (content_w + pad - button_w, y, button_w, button_h),
    );
    y += button_h + pad;

    STATE.with(|st| {
        *st.borrow_mut() = Some(State {
            password_box,
            status,
            retry_hello,
            use_master,
            password_hash: config.password_hash.clone(),
            master_hash: config.master_hash.clone(),
            master_mode: false,
            attempts: 0,
            answer: None,
            hello_rx: None,
            voice_button,
            voice_rx: None,
            voice_command: config.voice.unlock_command.clone(),
            voice_required: voice::required_level(config.voice.confidence_threshold),
            voice_patience: voice::patience(config.voice.listening_timeout),
        })
    });

    // The two optional buttons stay hidden until something makes them relevant.
    unsafe {
        let _ = ShowWindow(retry_hello, SW_HIDE);
        let _ = ShowWindow(use_master, SW_HIDE);
    }

    // ---- size and place ----
    unsafe {
        let mut outer = RECT { left: 0, top: 0, right: content_w + 2 * pad, bottom: y };
        let _ = AdjustWindowRectExForDpi(&mut outer, style, false, WS_EX_DLGMODALFRAME, dpi);
        let (w, h) = (ui::rect_w(&outer), ui::rect_h(&outer));

        // Centred on the owner when it is visible -- for the overlay, that is
        // the monitor it covers -- and otherwise on the monitor with the
        // pointer, which is what makes the prototype usable without an overlay.
        let mut anchor = RECT::default();
        if IsWindowVisible(owner).as_bool() {
            let _ = GetWindowRect(owner, &mut anchor);
        } else {
            anchor = ui::work_area_at_cursor();
        }
        let (left, top) = ui::keep_on_screen(
            anchor.left + (ui::rect_w(&anchor) - w) / 2,
            anchor.top + (ui::rect_h(&anchor) - h) / 2,
            w,
            h,
        );
        let _ = SetWindowPos(hwnd, None, left, top, w, h, SWP_NOZORDER);

        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(password_box));
    }

    // ---- Windows Hello first, if it is set up for this user ----
    if hello::is_available() {
        start_hello(hwnd);
    } else {
        set_status("Windows Hello isn't available. Enter your password to unlock.");
        show_master_button();
    }

    // ---- message loop ----
    let mut msg = MSG::default();
    loop {
        // Poll the worker. A channel rather than a cross-thread PostMessage so
        // the thread boundary stays one-directional: the worker never touches
        // our windows.
        if let Some(outcome) = STATE.with(|st| {
            let mut state = st.borrow_mut();
            let state = state.as_mut()?;
            let outcome = state.hello_rx.as_ref()?.try_recv().ok();
            if outcome.is_some() {
                state.hello_rx = None;
            }
            outcome
        }) {
            on_hello(hwnd, outcome);
        }

        if let Some(outcome) = STATE.with(|st| {
            let mut state = st.borrow_mut();
            let state = state.as_mut()?;
            let outcome = state.voice_rx.as_ref()?.try_recv().ok();
            if outcome.is_some() {
                state.voice_rx = None;
            }
            outcome
        }) {
            on_voice(hwnd, outcome);
        }

        if STATE.with(|st| st.borrow().as_ref().is_none_or(|s| s.answer.is_some())) {
            break;
        }

        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            unsafe { PostQuitMessage(msg.wParam.0 as i32) };
            break;
        }
        unsafe {
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe {
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(hwnd);
        let _ = SetForegroundWindow(owner);
    }
    ui::delete_font(font);
    STATE.with(|st| st.borrow_mut().take()).and_then(|s| s.answer).unwrap_or(Answer::Cancelled)
}

/// Kick off a Hello request, unless one is already in flight.
fn start_hello(hwnd: HWND) {
    let already = STATE.with(|st| st.borrow().as_ref().is_some_and(|s| s.hello_rx.is_some()));
    if already {
        return;
    }
    set_status("Waiting for Windows Hello\u{2026}");
    STATE.with(|st| {
        if let Some(state) = st.borrow().as_ref() {
            unsafe {
                let _ = ShowWindow(state.retry_hello, SW_HIDE);
            }
        }
    });
    let rx = hello::request_on_worker(hwnd, "Authenticate to unlock");
    STATE.with(|st| {
        if let Some(state) = st.borrow_mut().as_mut() {
            state.hello_rx = Some(rx);
        }
    });
}

/// Apply a Hello answer. Routed through the window procedure so that the
/// decision lives in one place whether it arrived from the first attempt or a
/// retry.
fn on_hello(hwnd: HWND, outcome: hello::Outcome) {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            hwnd,
            WM_APP_HELLO,
            Some(WPARAM(outcome_code(outcome))),
            Some(LPARAM(0)),
        );
    }
}

/// Begin listening, unless an attempt is already running.
fn start_voice() {
    let Some((command, required, patience, button)) = STATE.with(|st| {
        let mut state = st.borrow_mut();
        let state = state.as_mut()?;
        if state.voice_rx.is_some() {
            return None;
        }
        Some((state.voice_command.clone(), state.voice_required, state.voice_patience, state.voice_button))
    }) else {
        return;
    };

    set_status("Listening\u{2026}");
    sound::play(Sound::Listening);
    if let Some(button) = button {
        // SAFETY: a live child window.
        unsafe {
            let _ = EnableWindow(button, false);
        }
    }

    let rx = voice::listen_on_worker(&command, required, patience);
    STATE.with(|st| {
        if let Some(state) = st.borrow_mut().as_mut() {
            state.voice_rx = Some(rx);
        }
    });
}

/// Route a voice answer through the window procedure, so the decision lives in
/// one place.
fn on_voice(hwnd: HWND, outcome: voice::Outcome) {
    let accepted = outcome == voice::Outcome::Accepted;
    // Only the verdict crosses the WPARAM; the message is set here, because
    // what was heard is a String and has no business in a window message.
    set_status(&outcome.message());
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            hwnd,
            WM_APP_VOICE,
            Some(WPARAM(usize::from(accepted))),
            Some(LPARAM(0)),
        );
    }
}

/// Hello outcomes cross a `WPARAM`, so they travel as a small code.
fn outcome_code(outcome: hello::Outcome) -> usize {
    match outcome {
        hello::Outcome::Verified => 0,
        hello::Outcome::Declined => 1,
        hello::Outcome::Busy => 2,
        hello::Outcome::Unavailable => 3,
        hello::Outcome::Failed(_) => 4,
    }
}

fn outcome_from_code(code: usize) -> hello::Outcome {
    match code {
        0 => hello::Outcome::Verified,
        1 => hello::Outcome::Declined,
        2 => hello::Outcome::Busy,
        3 => hello::Outcome::Unavailable,
        _ => hello::Outcome::Failed(windows::core::HRESULT(-1)),
    }
}

fn set_status(text: &str) {
    STATE.with(|st| {
        if let Some(state) = st.borrow().as_ref() {
            ui::set_text(state.status, text);
        }
    });
}

fn show_master_button() {
    STATE.with(|st| {
        if let Some(state) = st.borrow().as_ref()
            && state.master_hash.is_some()
        {
            unsafe {
                let _ = ShowWindow(state.use_master, SW_SHOW);
            }
        }
    });
}

fn finish(answer: Answer) {
    STATE.with(|st| {
        if let Some(state) = st.borrow_mut().as_mut() {
            state.answer = Some(answer);
        }
    });
}

/// Check whatever is in the password box against the active credential.
fn attempt_password() {
    let (control, hash, master_mode, attempts) = STATE.with(|st| {
        let state = st.borrow();
        let state = state.as_ref().expect("state is present while the dialog is up");
        let hash = if state.master_mode { state.master_hash.clone() } else { state.password_hash.clone() };
        (state.password_box, hash, state.master_mode, state.attempts)
    });

    // The plaintext exists only inside this function, and `Zeroizing` clears it
    // on every exit path including an early return.
    let typed = Zeroizing::new(ui::get_text(control));

    if typed.is_empty() {
        set_status("Enter your password.");
        return;
    }

    let slow_after = if master_mode { 1 } else { SLOW_AFTER };
    if attempts >= slow_after {
        std::thread::sleep(SLOW_BY);
    }

    match credential::verify(&typed, hash.as_deref()) {
        Ok(true) => {
            sound::play(Sound::Unlocked);
            finish(Answer::Unlocked(if master_mode { Via::Master } else { Via::Password }));
        }
        Ok(false) => {
            sound::play(Sound::Rejected);
            ui::set_text(control, "");
            STATE.with(|st| {
                if let Some(state) = st.borrow_mut().as_mut() {
                    state.attempts += 1;
                }
            });
            set_status(if master_mode {
                "That master password is not correct."
            } else {
                "That password is not correct."
            });
            show_master_button();
            unsafe {
                let _ = SetFocus(Some(control));
            }
        }
        Err(e) => {
            // No stored verifier, or an unreadable one. Say so rather than
            // looping, and never fall back to accepting the input -- which is
            // what the .NET EncryptionHelper effectively did when decryption
            // failed.
            set_status("This copy isn't set up. Run setup again.");
            ui::error(None, &format!("{e}\n\nRun LockScreen.exe --setup."));
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_HELLO => {
            let outcome = outcome_from_code(wparam.0);
            if outcome == hello::Outcome::Verified {
                sound::play(Sound::Unlocked);
                finish(Answer::Unlocked(Via::Hello));
                return LRESULT(0);
            }
            set_status(outcome.message());
            STATE.with(|st| {
                if let Some(state) = st.borrow().as_ref() {
                    unsafe {
                        let _ = ShowWindow(state.retry_hello, if outcome.is_retryable() { SW_SHOW } else { SW_HIDE });
                        let _ = SetFocus(Some(state.password_box));
                    }
                }
            });
            show_master_button();
            LRESULT(0)
        }
        WM_APP_VOICE => {
            if wparam.0 == 1 {
                sound::play(Sound::Unlocked);
                finish(Answer::Unlocked(Via::Voice));
                return LRESULT(0);
            }
            // Failed attempts count against the same limit as a wrong password,
            // so a spoken guess is not a cheaper one.
            STATE.with(|st| {
                if let Some(state) = st.borrow_mut().as_mut() {
                    state.attempts += 1;
                    if let Some(button) = state.voice_button {
                        unsafe {
                            let _ = EnableWindow(button, true);
                        }
                    }
                    unsafe {
                        let _ = SetFocus(Some(state.password_box));
                    }
                }
            });
            show_master_button();
            LRESULT(0)
        }
        WM_COMMAND => {
            match ui::loword(wparam.0) as i32 {
                x if x == IDOK.0 => attempt_password(),
                x if x == ID_VOICE as i32 => start_voice(),
                x if x == IDCANCEL.0 => finish(Answer::Cancelled),
                x if x == ID_RETRY_HELLO as i32 => start_hello(hwnd),
                x if x == ID_USE_MASTER as i32 => {
                    STATE.with(|st| {
                        if let Some(state) = st.borrow_mut().as_mut() {
                            state.master_mode = true;
                            state.attempts = 0;
                            ui::set_text(state.password_box, "");
                            unsafe {
                                let _ = ShowWindow(state.use_master, SW_HIDE);
                                let _ = SetFocus(Some(state.password_box));
                            }
                        }
                    });
                    set_status("Enter your master password.");
                }
                _ => {}
            }
            LRESULT(0)
        }
        // Answered, not vetoed: the overlay outliving the dialog is what keeps
        // the machine locked.
        WM_CLOSE => {
            finish(Answer::Cancelled);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_outcomes_survive_the_wparam_round_trip() {
        for outcome in
            [hello::Outcome::Verified, hello::Outcome::Declined, hello::Outcome::Busy, hello::Outcome::Unavailable]
        {
            assert_eq!(outcome_from_code(outcome_code(outcome)), outcome);
        }
        assert!(matches!(
            outcome_from_code(outcome_code(hello::Outcome::Failed(windows::core::HRESULT(-2)))),
            hello::Outcome::Failed(_)
        ));
    }

    #[test]
    fn an_unknown_code_is_a_failure_not_a_verification() {
        assert!(matches!(outcome_from_code(999), hello::Outcome::Failed(_)));
        assert_ne!(outcome_from_code(999), hello::Outcome::Verified);
    }
}
