//! First-run setup: choosing the unlock password and the master password.
//!
//! Replaces `IniHelper.EnsureConfigExists()`
//! (`LockScreenApp/Helpers/IniHelper.cs:29-57`), which writes
//! `Password=1234` into whatever directory the app was launched from and then
//! shows it in a `MessageBox`. There is no default password here and nothing is
//! ever displayed.
//!
//! Deliberately installs no input hooks and writes no registry policy: an
//! abandoned setup leaves the machine exactly as it found it.

use lockscreen_core::config::Config;
use lockscreen_core::credential::{self, Kind};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush, HBRUSH};
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    IDCANCEL, IDOK, IsDialogMessageW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, SWP_NOZORDER, SetForegroundWindow,
    SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WNDCLASSW,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};
use windows::core::PCWSTR;
use zeroize::Zeroizing;

use super::ui;

const ID_PASSWORD: u16 = 20;
const ID_PASSWORD_AGAIN: u16 = 21;
const ID_MASTER: u16 = 22;
const ID_MASTER_AGAIN: u16 = 23;

/// What setup produced. The passwords are already hashed: plaintext never
/// leaves [`collect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub password_hash: String,
    pub master_hash: String,
}

/// Why a filled-in form is not acceptable yet.
///
/// Ordered as the form is read, so the message always points at the first
/// field that needs attention rather than the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    PasswordTooShort,
    PasswordsDiffer,
    MasterTooShort,
    MastersDiffer,
    /// The master password is the same secret as the unlock password, which
    /// makes it useless: a recovery credential that fails whenever the thing it
    /// is recovering from fails is not a recovery credential.
    MasterSameAsPassword,
}

impl Problem {
    pub fn message(self) -> String {
        match self {
            // Two messages, because the two floors differ: the unlock password
            // is typed many times a day, the master password is the recovery
            // credential. A single message quoting one number would be wrong
            // for the other field.
            Problem::PasswordTooShort => {
                format!("The unlock password needs at least {} characters.", Kind::Password.min_length())
            }
            Problem::MasterTooShort => {
                format!("The master password needs at least {} characters.", Kind::Master.min_length())
            }
            Problem::PasswordsDiffer => "The unlock passwords do not match.".to_owned(),
            Problem::MastersDiffer => "The master passwords do not match.".to_owned(),
            Problem::MasterSameAsPassword => {
                "The master password must be different from the unlock password, or it cannot recover anything."
                    .to_owned()
            }
        }
    }
}

/// Check a filled-in form.
///
/// Pure, so the rules are testable without a window. Takes `&str` rather than
/// the controls because the caller has already pulled the text into zeroizing
/// buffers.
pub fn validate(password: &str, password_again: &str, master: &str, master_again: &str) -> Result<(), Problem> {
    if !credential::is_acceptable(Kind::Password, password) {
        return Err(Problem::PasswordTooShort);
    }
    if password != password_again {
        return Err(Problem::PasswordsDiffer);
    }
    if !credential::is_acceptable(Kind::Master, master) {
        return Err(Problem::MasterTooShort);
    }
    if master != master_again {
        return Err(Problem::MastersDiffer);
    }
    if master == password {
        return Err(Problem::MasterSameAsPassword);
    }
    Ok(())
}

struct State {
    password: HWND,
    password_again: HWND,
    master: HWND,
    master_again: HWND,
    status: HWND,
    done: Option<Option<Chosen>>,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// Show the setup window. `None` if the user cancelled.
///
/// `migrating` adds a line explaining that the appearance settings came from a
/// config the .NET version wrote, so somebody who expected their old password
/// to still work is told why it does not.
pub fn collect(migrating: bool, config_path: &std::path::Path) -> Option<Chosen> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None;
    }

    // No owner window: setup runs before any overlay exists.
    let dpi = 96_u32.max(ui::dpi_of(HWND::default()));
    let s = |v: i32| ui::scale(v, dpi);
    let font = ui::message_font(dpi);
    let class = ui::wide("LockScreen.Setup");
    let title = ui::wide("Set up LockScreen");
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;

    let hwnd = unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: super::instance(),
            lpszClassName: PCWSTR(class.as_ptr()),
            // Without this the class has no cursor, and Windows shows the
            // "app starting" hourglass over the window. See `ui::arrow_cursor`.
            hCursor: ui::arrow_cursor(),
            hbrBackground: HBRUSH(GetSysColorBrush(COLOR_WINDOW).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
        match CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            0,
            0,
            100,
            100,
            None,
            None,
            Some(super::instance()),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => {
                ui::delete_font(font);
                return None;
            }
        }
    };

    // ---- layout ----
    let pad = s(18);
    let gap = s(6);
    let field_h = s(26);
    let label_h = s(18);
    let button_w = s(120);
    let button_h = s(28);
    let content_w = s(392);
    let mut y = pad;

    let add = |id: u16, class: &str, text: &str, st: WINDOW_STYLE, r: (i32, i32, i32, i32)| {
        let c = ui::child(hwnd, class, text, WS_CHILD | WS_VISIBLE | st, WINDOW_EX_STYLE(0), id);
        unsafe {
            let _ = SetWindowPos(c, None, r.0, r.1, r.2, r.3, SWP_NOZORDER);
        }
        ui::set_font(c, font);
        c
    };

    let intro = if migrating {
        "Your settings have been carried over from the previous version, but its \
         password could not be: it was stored in a form that only works on one \
         machine. Choose two passwords to finish setting up."
    } else {
        "Choose two passwords. The unlock password is the one you will type \
         every day. The master password is a way back in if you forget it: keep \
         it somewhere safe and do not use it routinely."
    };
    let intro_h = s(64);
    // SS_LABEL truncates with an ellipsis; the intro needs to wrap instead.
    add(0, "STATIC", intro, WINDOW_STYLE(0x0080), (pad, y, content_w, intro_h));
    y += intro_h + gap * 2;

    let field = |label: &str, id: u16, y: &mut i32| -> HWND {
        add(0, "STATIC", label, ui::SS_LABEL, (pad, *y, content_w, label_h));
        *y += label_h;
        let control = add(
            id,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ui::ES_PASSWORD | ui::ES_AUTOHSCROLL,
            (pad, *y, content_w, field_h),
        );
        *y += field_h + gap;
        control
    };

    let password = field("Unlock password", ID_PASSWORD, &mut y);
    let password_again = field("Unlock password again", ID_PASSWORD_AGAIN, &mut y);
    y += gap;
    let master = field("Master password", ID_MASTER, &mut y);
    let master_again = field("Master password again", ID_MASTER_AGAIN, &mut y);

    y += gap;
    let status = add(0, "STATIC", "", ui::SS_LABEL, (pad, y, content_w, s(32)));
    y += s(32) + gap;

    // Where it will be written. A portable app should say so rather than make
    // somebody hunt for it.
    let where_to = format!("Saved in {}", config_path.display());
    add(0, "STATIC", &where_to, ui::SS_LABEL, (pad, y, content_w, label_h));
    y += label_h + gap * 2;

    add(
        IDOK.0 as u16,
        "BUTTON",
        "Save",
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
        *st.borrow_mut() = Some(State { password, password_again, master, master_again, status, done: None })
    });

    unsafe {
        let mut outer = RECT { left: 0, top: 0, right: content_w + 2 * pad, bottom: y };
        let _ = AdjustWindowRectExForDpi(&mut outer, style, false, WS_EX_DLGMODALFRAME, dpi);
        let (w, h) = (ui::rect_w(&outer), ui::rect_h(&outer));
        let work = ui::work_area_at_cursor();
        let (left, top) =
            ui::keep_on_screen(work.left + (ui::rect_w(&work) - w) / 2, work.top + (ui::rect_h(&work) - h) / 2, w, h);
        let _ = SetWindowPos(hwnd, None, left, top, w, h, SWP_NOZORDER);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(password));
    }

    let mut msg = MSG::default();
    loop {
        if STATE.with(|st| st.borrow().as_ref().is_none_or(|s| s.done.is_some())) {
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
        let _ = DestroyWindow(hwnd);
    }
    ui::delete_font(font);
    STATE.with(|st| st.borrow_mut().take()).and_then(|s| s.done).flatten()
}

fn set_status(text: &str) {
    STATE.with(|st| {
        if let Some(state) = st.borrow().as_ref() {
            ui::set_text(state.status, text);
        }
    });
}

/// Read the four fields, validate, hash, and finish.
///
/// Every buffer holding a typed password is `Zeroizing`, so none of them
/// outlive this function even on an early return.
fn attempt_save() {
    let (p1, p2, m1, m2, focus_first) = STATE.with(|st| {
        let state = st.borrow();
        let state = state.as_ref().expect("state is present while the window is up");
        (
            Zeroizing::new(ui::get_text(state.password)),
            Zeroizing::new(ui::get_text(state.password_again)),
            Zeroizing::new(ui::get_text(state.master)),
            Zeroizing::new(ui::get_text(state.master_again)),
            state.password,
        )
    });

    if let Err(problem) = validate(&p1, &p2, &m1, &m2) {
        set_status(&problem.message());
        // Put the caret where the problem is.
        let target = STATE.with(|st| {
            st.borrow().as_ref().map(|s| match problem {
                Problem::PasswordTooShort | Problem::MasterSameAsPassword => s.password,
                Problem::PasswordsDiffer => s.password_again,
                Problem::MasterTooShort => s.master,
                Problem::MastersDiffer => s.master_again,
            })
        });
        unsafe {
            let _ = SetFocus(Some(target.unwrap_or(focus_first)));
        }
        return;
    }

    let (Ok(password_hash), Ok(master_hash)) = (credential::hash(&p1), credential::hash(&m1)) else {
        set_status("The passwords could not be hashed. This is a bug; nothing has been saved.");
        return;
    };

    STATE.with(|st| {
        if let Some(state) = st.borrow_mut().as_mut() {
            state.done = Some(Some(Chosen { password_hash, master_hash }));
        }
    });
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match ui::loword(wparam.0) as i32 {
                x if x == IDOK.0 => attempt_save(),
                x if x == IDCANCEL.0 => STATE.with(|st| {
                    if let Some(state) = st.borrow_mut().as_mut() {
                        state.done = Some(None);
                    }
                }),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            STATE.with(|st| {
                if let Some(state) = st.borrow_mut().as_mut() {
                    state.done = Some(None);
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Apply what setup chose to a config, dropping anything reversible.
pub fn apply(mut config: Config, chosen: Chosen) -> Config {
    config.password_hash = Some(chosen.password_hash);
    config.master_hash = Some(chosen.master_hash);
    // The .NET DPAPI value is not carried forward. `config::save` would drop it
    // anyway; clearing it here means the in-memory config matches what lands on
    // disk, so nothing downstream can accidentally rely on it.
    config.legacy_password = None;
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "a-long-enough-password";
    const GOOD_MASTER: &str = "a-different-master-key";

    #[test]
    fn a_well_filled_form_is_accepted() {
        assert_eq!(validate(GOOD, GOOD, GOOD_MASTER, GOOD_MASTER), Ok(()));
    }

    #[test]
    fn each_password_must_clear_its_own_floor() {
        // The floors differ, and that asymmetry is the policy: four characters
        // for the password typed at the prompt, eight for the recovery
        // credential standing behind it.
        assert_eq!(
            validate("1234", "1234", GOOD_MASTER, GOOD_MASTER),
            Ok(()),
            "four is allowed for the unlock password"
        );
        assert_eq!(validate("abc", "abc", GOOD_MASTER, GOOD_MASTER), Err(Problem::PasswordTooShort), "three is not");

        // The same four characters are refused for the master password.
        assert_eq!(validate(GOOD, GOOD, "1234", "1234"), Err(Problem::MasterTooShort));
        assert_eq!(validate(GOOD, GOOD, "shortish", "shortish"), Ok(()), "eight clears the master's floor");
        assert_eq!(validate(GOOD, GOOD, "short", "short"), Err(Problem::MasterTooShort), "seven or fewer does not");
    }

    #[test]
    fn both_passwords_must_be_typed_twice_the_same_way() {
        assert_eq!(validate(GOOD, "something else entirely", GOOD_MASTER, GOOD_MASTER), Err(Problem::PasswordsDiffer));
        assert_eq!(validate(GOOD, GOOD, GOOD_MASTER, "another thing again"), Err(Problem::MastersDiffer));
    }

    #[test]
    fn the_master_password_cannot_be_the_unlock_password() {
        // A recovery credential that is the same secret as the thing it
        // recovers is not a recovery credential.
        assert_eq!(validate(GOOD, GOOD, GOOD, GOOD), Err(Problem::MasterSameAsPassword));
    }

    #[test]
    fn passphrases_with_symbols_are_accepted() {
        // The .NET policy required letters and digits only, so it rejected
        // every passphrase anyone would actually want.
        let phrase = "correct horse battery staple!";
        let master = "a different #phrase entirely";
        assert_eq!(validate(phrase, phrase, master, master), Ok(()));
    }

    #[test]
    fn the_first_problem_in_reading_order_is_the_one_reported() {
        // Everything wrong at once: the message should name the unlock password
        // rather than whichever check happens to run last.
        assert_eq!(validate("x", "y", "z", "w"), Err(Problem::PasswordTooShort));
    }

    #[test]
    fn every_problem_has_a_message_that_says_what_to_do() {
        for problem in [
            Problem::PasswordTooShort,
            Problem::PasswordsDiffer,
            Problem::MasterTooShort,
            Problem::MastersDiffer,
            Problem::MasterSameAsPassword,
        ] {
            let message = problem.message();
            assert!(message.ends_with('.'), "{problem:?}: {message}");
            assert!(message.len() > 20, "{problem:?}: too terse to act on");
            // A run of spaces mid-sentence means a line continuation was
            // flattened on the way into the source. One shipped in 0.2.3, so
            // every message that reaches a user is asserted against it.
            assert!(!message.contains("  "), "{problem:?} has a run of spaces: {message:?}");
            assert_eq!(message.lines().count(), 1, "{problem:?} spans lines: {message:?}");
        }
    }

    #[test]
    fn applying_setup_stores_both_verifiers_and_drops_the_legacy_one() {
        let config = Config { legacy_password: Some("AQAAANCM-pretend".into()), ..Config::default() };
        assert!(config.needs_setup());

        let chosen = Chosen {
            password_hash: credential::hash(GOOD).expect("hashed"),
            master_hash: credential::hash(GOOD_MASTER).expect("hashed"),
        };
        let config = apply(config, chosen);

        assert!(!config.needs_setup(), "the overlay may start now");
        assert!(config.legacy_password.is_none(), "nothing reversible survives setup");
        assert!(credential::verify(GOOD, config.password_hash.as_deref()).expect("verified"));
        assert!(credential::verify(GOOD_MASTER, config.master_hash.as_deref()).expect("verified"));
        // And the two are not interchangeable.
        assert!(!credential::verify(GOOD, config.master_hash.as_deref()).expect("verified"));
        assert!(!credential::verify(GOOD_MASTER, config.password_hash.as_deref()).expect("verified"));
    }
}
