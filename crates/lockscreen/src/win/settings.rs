//! The settings window.
//!
//! # Defect D7
//!
//! `SettingsApp/MainWindow.xaml.cs:88` -- `Save_Click` -- reads
//! `NewPasswordBox.Password`, checks its strength, encrypts it and writes it. It
//! never asks for the current password and never invokes Windows Hello. Anyone
//! who reaches an unattended desktop can run `Settings.exe` and reset the lock
//! password, so the overlay's protection is one double-click from being
//! bypassed. That undercuts the entire product and is independent of language.
//!
//! The fix is a gate in [`crate::mode::settings`]: the unlock ladder has to be
//! satisfied **before this window opens at all**, and again before a credential
//! change is written. Two gates rather than one because a settings window left
//! open on a desk is the case the first gate does not cover.
//!
//! The ladder used is [`super::unlock::Purpose::Authorize`], which is the same
//! code path as unlocking minus voice: speaker-independent recognition is not
//! evidence of who is standing there, and permission to replace a password
//! needs more than a spoken word.

use lockscreen_core::config::Config;
use lockscreen_core::credential::{self, Kind};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush, HBRUSH};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, GetMessageW, IDCANCEL, IDOK, IsDialogMessageW, MSG, PostQuitMessage,
    RegisterClassW, SW_SHOW, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowPos, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::PCWSTR;
use zeroize::Zeroizing;

use lockscreen_core::release;

use super::{ui, update, voice};

const ID_TITLE: u16 = 30;
const ID_TRANSPARENCY: u16 = 31;
const ID_BLUR: u16 = 32;
const ID_VOICE_ENABLED: u16 = 33;
const ID_TEST_MIC: u16 = 34;
const ID_PASSWORD: u16 = 35;
const ID_PASSWORD_AGAIN: u16 = 36;
const ID_MASTER: u16 = 37;
const ID_MASTER_AGAIN: u16 = 38;
const ID_UPDATE: u16 = 39;

/// What the user changed. Password fields left blank mean "leave it alone", so
/// the hashes are `Option`.
#[derive(Debug, Clone, PartialEq)]
pub struct Changes {
    pub title: String,
    pub transparency: f64,
    pub blur_background: bool,
    pub voice_enabled: bool,
    pub password_hash: Option<String>,
    pub master_hash: Option<String>,
}

impl Changes {
    /// Whether anything here replaces a credential, and so needs the second
    /// authorisation.
    pub fn touches_credentials(&self) -> bool {
        self.password_hash.is_some() || self.master_hash.is_some()
    }
}

/// Why the form cannot be saved yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    TitleEmpty,
    TransparencyNotANumber,
    TransparencyOutOfRange,
    PasswordTooShort,
    PasswordsDiffer,
    MasterTooShort,
    MastersDiffer,
    /// Both changed, to the same thing. The master password exists to be a
    /// different secret.
    MasterSameAsPassword,
    /// Only one half of a password pair was filled in.
    PasswordConfirmMissing,
    MasterConfirmMissing,
}

impl Problem {
    pub fn message(self) -> String {
        match self {
            Problem::TitleEmpty => "The lock screen needs a title.".to_owned(),
            Problem::TransparencyNotANumber => "Transparency must be a number, such as 0.5.".to_owned(),
            Problem::TransparencyOutOfRange => "Transparency must be between 0.0 and 1.0.".to_owned(),
            // Separate messages: the master password's floor is higher, so one
            // message quoting a single number would mislead on one of the two
            // fields.
            Problem::PasswordTooShort => {
                format!("A new unlock password needs at least {} characters.", Kind::Password.min_length())
            }
            Problem::MasterTooShort => {
                format!("A new master password needs at least {} characters.", Kind::Master.min_length())
            }
            Problem::PasswordsDiffer => "The new unlock passwords do not match.".to_owned(),
            Problem::MastersDiffer => "The new master passwords do not match.".to_owned(),
            Problem::MasterSameAsPassword => {
                "The master password must be different from the unlock password.".to_owned()
            }
            Problem::PasswordConfirmMissing => {
                "Type the new unlock password twice, or leave both boxes empty.".to_owned()
            }
            Problem::MasterConfirmMissing => {
                "Type the new master password twice, or leave both boxes empty.".to_owned()
            }
        }
    }
}

/// What the form holds, before validation.
pub struct Typed<'a> {
    pub title: &'a str,
    pub transparency: &'a str,
    pub blur_background: bool,
    pub voice_enabled: bool,
    pub password: &'a str,
    pub password_again: &'a str,
    pub master: &'a str,
    pub master_again: &'a str,
}

/// Validate the form and hash anything that changed.
///
/// Pure apart from the hashing, so the rules are testable without a window.
/// Blank password boxes mean "unchanged", which is the behaviour the .NET
/// settings app had and the one people expect -- the difference is that getting
/// here now requires authenticating.
pub fn validate(typed: &Typed<'_>) -> Result<Changes, Problem> {
    if typed.title.trim().is_empty() {
        return Err(Problem::TitleEmpty);
    }
    let transparency: f64 = typed.transparency.trim().parse().map_err(|_| Problem::TransparencyNotANumber)?;
    if !(0.0..=1.0).contains(&transparency) {
        return Err(Problem::TransparencyOutOfRange);
    }

    // Half-filled pairs are a mistake, not an instruction. Saying so beats
    // silently ignoring a password somebody thinks they just set.
    let changing_password = !typed.password.is_empty() || !typed.password_again.is_empty();
    let changing_master = !typed.master.is_empty() || !typed.master_again.is_empty();

    if changing_password && (typed.password.is_empty() || typed.password_again.is_empty()) {
        return Err(Problem::PasswordConfirmMissing);
    }
    if changing_master && (typed.master.is_empty() || typed.master_again.is_empty()) {
        return Err(Problem::MasterConfirmMissing);
    }

    if changing_password {
        if !credential::is_acceptable(Kind::Password, typed.password) {
            return Err(Problem::PasswordTooShort);
        }
        if typed.password != typed.password_again {
            return Err(Problem::PasswordsDiffer);
        }
    }
    if changing_master {
        if !credential::is_acceptable(Kind::Master, typed.master) {
            return Err(Problem::MasterTooShort);
        }
        if typed.master != typed.master_again {
            return Err(Problem::MastersDiffer);
        }
    }
    // Only checkable when both are being set in the same save. Setting one to
    // match the other across two separate saves is not detectable without
    // storing something reversible, which is exactly what the port removed --
    // an acknowledged gap, not an oversight.
    if changing_password && changing_master && typed.password == typed.master {
        return Err(Problem::MasterSameAsPassword);
    }

    let hash = |changing: bool, value: &str| -> Result<Option<String>, Problem> {
        if changing { credential::hash(value).map(Some).map_err(|_| Problem::PasswordTooShort) } else { Ok(None) }
    };

    Ok(Changes {
        title: typed.title.trim().to_owned(),
        transparency,
        blur_background: typed.blur_background,
        voice_enabled: typed.voice_enabled,
        password_hash: hash(changing_password, typed.password)?,
        master_hash: hash(changing_master, typed.master)?,
    })
}

/// Fold accepted changes into a config, leaving untouched credentials alone.
pub fn apply(mut config: Config, changes: Changes) -> Config {
    config.title = changes.title;
    config.transparency = changes.transparency;
    config.blur_background = changes.blur_background;
    config.voice.enabled = changes.voice_enabled;
    if let Some(hash) = changes.password_hash {
        config.password_hash = Some(hash);
    }
    if let Some(hash) = changes.master_hash {
        config.master_hash = Some(hash);
    }
    config
}

struct State {
    title: HWND,
    transparency: HWND,
    blur: HWND,
    voice_enabled: HWND,
    test_mic: HWND,
    password: HWND,
    password_again: HWND,
    master: HWND,
    master_again: HWND,
    status: HWND,
    voice_command: String,
    /// The update button and the line beside it. Only settings mode has these:
    /// the overlay must not offer a path that writes a new executable.
    update_button: HWND,
    version_line: HWND,
    update_state: release::State,
    done: Option<Option<Changes>>,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

fn checked(control: HWND) -> bool {
    // SAFETY: a live child window; BM_GETCHECK takes no pointers.
    unsafe { SendMessageW(control, BM_GETCHECK, None, None).0 as u32 == BST_CHECKED.0 }
}

fn set_checked(control: HWND, value: bool) {
    let state = if value { BST_CHECKED.0 } else { 0 };
    // SAFETY: as above.
    unsafe {
        SendMessageW(control, BM_SETCHECK, Some(WPARAM(state as usize)), None);
    }
}

/// Show the settings window. `None` if cancelled.
pub fn edit(config: &Config) -> Option<Changes> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None;
    }

    let dpi = 96_u32.max(ui::dpi_of(HWND::default()));
    let s = |v: i32| ui::scale(v, dpi);
    let font = ui::message_font(dpi);
    let class = ui::wide("LockScreen.Settings");
    let title_text = ui::wide("Lock Screen settings");
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
            WS_EX_DLGMODALFRAME,
            PCWSTR(class.as_ptr()),
            PCWSTR(title_text.as_ptr()),
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
    let row_h = s(24);
    let button_w = s(120);
    let button_h = s(28);
    let content_w = s(400);
    let mut y = pad;

    let add = |id: u16, class: &str, text: &str, st: WINDOW_STYLE, r: (i32, i32, i32, i32)| {
        let c = ui::child(hwnd, class, text, WS_CHILD | WS_VISIBLE | st, WINDOW_EX_STYLE(0), id);
        unsafe {
            let _ = SetWindowPos(c, None, r.0, r.1, r.2, r.3, SWP_NOZORDER);
        }
        ui::set_font(c, font);
        c
    };

    let text_field = |label: &str, id: u16, value: &str, y: &mut i32| -> HWND {
        add(0, "STATIC", label, ui::SS_LABEL, (pad, *y, content_w, label_h));
        *y += label_h;
        let c = add(id, "EDIT", value, WS_TABSTOP | WS_BORDER | ui::ES_AUTOHSCROLL, (pad, *y, content_w, field_h));
        *y += field_h + gap;
        c
    };

    let title = text_field("Lock screen title", ID_TITLE, &config.title, &mut y);
    let transparency =
        text_field("Transparency (0.0 to 1.0)", ID_TRANSPARENCY, &format!("{:.2}", config.transparency), &mut y);

    let blur = add(
        ID_BLUR,
        "BUTTON",
        "Blur the desktop behind the lock screen",
        WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        (pad, y, content_w, row_h),
    );
    y += row_h + gap;

    let voice_enabled = add(
        ID_VOICE_ENABLED,
        "BUTTON",
        "Allow unlocking by saying the command out loud",
        WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        (pad, y, content_w, row_h),
    );
    y += row_h;

    // Said plainly, next to the switch, rather than buried in a manual nobody
    // reads. This is the one setting that can make the lock weaker.
    add(
        0,
        "STATIC",
        "Anyone who says the command can unlock: Windows recognises the word, not the voice.",
        WINDOW_STYLE(0x0080),
        (pad + s(20), y, content_w - s(20), s(32)),
    );
    y += s(32) + gap;

    let test_mic = add(ID_TEST_MIC, "BUTTON", "Test microphone", WS_TABSTOP, (pad, y, s(150), button_h));
    y += button_h + gap * 2;

    add(
        0,
        "STATIC",
        "Leave the password boxes empty to keep the current ones.",
        ui::SS_LABEL,
        (pad, y, content_w, label_h),
    );
    y += label_h + gap;

    let password_field = |label: &str, id: u16, y: &mut i32| -> HWND {
        add(0, "STATIC", label, ui::SS_LABEL, (pad, *y, content_w, label_h));
        *y += label_h;
        let c = add(
            id,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ui::ES_PASSWORD | ui::ES_AUTOHSCROLL,
            (pad, *y, content_w, field_h),
        );
        *y += field_h + gap;
        c
    };

    let password = password_field("New unlock password", ID_PASSWORD, &mut y);
    let password_again = password_field("New unlock password again", ID_PASSWORD_AGAIN, &mut y);
    let master = password_field("New master password", ID_MASTER, &mut y);
    let master_again = password_field("New master password again", ID_MASTER_AGAIN, &mut y);

    y += gap;
    let status = add(0, "STATIC", "", WINDOW_STYLE(0x0080), (pad, y, content_w, s(34)));
    y += s(34) + gap;

    // Version here and nowhere else. On the lock overlay it would tell whoever
    // is standing at the machine which build to look up.
    let version_line = add(
        0,
        "STATIC",
        &release::version_line(release::Version::running(), release::State::Unknown),
        ui::SS_LABEL,
        (pad, y, content_w - s(150) - gap, label_h + s(6)),
    );
    let update_button = add(
        ID_UPDATE,
        "BUTTON",
        release::State::Unknown.action(),
        WS_TABSTOP,
        (pad + content_w - s(150), y - s(4), s(150), button_h),
    );
    y += button_h + gap;
    add(0, "STATIC", &format!("Build {}", env!("LOCKSCREEN_COMMIT")), ui::SS_LABEL, (pad, y, content_w, label_h));
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

    set_checked(blur, config.blur_background);
    set_checked(voice_enabled, config.voice.enabled);

    STATE.with(|st| {
        *st.borrow_mut() = Some(State {
            title,
            transparency,
            blur,
            voice_enabled,
            test_mic,
            password,
            password_again,
            master,
            master_again,
            status,
            voice_command: config.voice.unlock_command.clone(),
            update_button,
            version_line,
            update_state: release::State::Unknown,
            done: None,
        })
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
        let _ = SetFocus(Some(title));
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

fn attempt_save() {
    let (title, transparency, blur, voice, p1, p2, m1, m2) = STATE.with(|st| {
        let state = st.borrow();
        let state = state.as_ref().expect("state is present while the window is up");
        (
            ui::get_text(state.title),
            ui::get_text(state.transparency),
            checked(state.blur),
            checked(state.voice_enabled),
            Zeroizing::new(ui::get_text(state.password)),
            Zeroizing::new(ui::get_text(state.password_again)),
            Zeroizing::new(ui::get_text(state.master)),
            Zeroizing::new(ui::get_text(state.master_again)),
        )
    });

    let typed = Typed {
        title: &title,
        transparency: &transparency,
        blur_background: blur,
        voice_enabled: voice,
        password: &p1,
        password_again: &p2,
        master: &m1,
        master_again: &m2,
    };

    match validate(&typed) {
        Ok(changes) => STATE.with(|st| {
            if let Some(state) = st.borrow_mut().as_mut() {
                state.done = Some(Some(changes));
            }
        }),
        Err(problem) => {
            set_status(&problem.message());
            let target = STATE.with(|st| {
                st.borrow().as_ref().map(|s| match problem {
                    Problem::TitleEmpty => s.title,
                    Problem::TransparencyNotANumber | Problem::TransparencyOutOfRange => s.transparency,
                    Problem::PasswordTooShort | Problem::MasterSameAsPassword | Problem::PasswordConfirmMissing => {
                        s.password
                    }
                    Problem::PasswordsDiffer => s.password_again,
                    Problem::MasterTooShort | Problem::MasterConfirmMissing => s.master,
                    Problem::MastersDiffer => s.master_again,
                })
            });
            if let Some(target) = target {
                unsafe {
                    let _ = SetFocus(Some(target));
                }
            }
        }
    }
}

/// Reflect the current update state in the line and the button.
fn show_update_state(state: release::State) {
    STATE.with(|st| {
        let mut held = st.borrow_mut();
        let Some(s) = held.as_mut() else { return };
        s.update_state = state;
        ui::set_text(s.version_line, &release::version_line(release::Version::running(), state));
        ui::set_text(s.update_button, state.action());
        // SAFETY: a live child window.
        unsafe {
            let _ = EnableWindow(s.update_button, state.enabled());
        }
    });
}

/// The update button: looks the first time, installs the second.
///
/// Both halves block the UI thread, which is acceptable here and nowhere else:
/// settings is a modal window somebody is sitting in front of, not the prompt
/// standing between them and their desktop. The button is disabled first so the
/// window reads as busy rather than frozen.
fn update_pressed() {
    let state = STATE.with(|st| st.borrow().as_ref().map(|s| s.update_state)).unwrap_or_default();

    if !state.offers_update() {
        show_update_state(release::State::Checking);
        let found = update::check();
        show_update_state(found);
        set_status(match found {
            release::State::UpToDate => "This is the newest release.",
            release::State::Failed => "The forge could not be reached. There is a note in the log.",
            _ => "",
        });
        return;
    }

    // Installing, and only ever because somebody pressed this. No silent
    // auto-update: a compromised release channel should not own every install
    // with nobody in the loop.
    set_status("Downloading...");
    let found = match update::fetch_update() {
        Ok(found) => found,
        Err(e) => {
            set_status(&format!("The update could not be fetched: {e}"));
            return;
        }
    };

    if !found.installable() {
        // Published without checksums, so it is offered as a page rather than
        // installed. The release workflow publishes both or neither, so this
        // means something went wrong at release time.
        set_status("That release has no checksums, so it will not be installed here. Opening its page instead.");
        update::open_page(&found);
        return;
    }

    match update::download(&found).and_then(|file| update::install(&file)) {
        Ok(()) => {
            // Deliberately does not restart. Relaunching could put the overlay
            // back up, and restarting a lock screen mid-session is a bypass
            // rather than a convenience.
            set_status(&format!(
                "Version {} is installed. Close this window, then start Lock Screen again to use it.",
                found.version
            ));
        }
        Err(e) => set_status(&format!("The update was not installed: {e}")),
    }
}

/// Probe the recogniser and report. Takes about a second, which is why the
/// button is disabled while it runs.
fn test_microphone() {
    let (button, command) = STATE.with(|st| {
        let state = st.borrow();
        let state = state.as_ref().expect("state is present");
        (state.test_mic, state.voice_command.clone())
    });

    set_status("Checking the microphone\u{2026}");
    // SAFETY: a live child window.
    unsafe {
        let _ = EnableWindow(button, false);
    }

    let available = voice::is_available(&command);

    set_status(if available {
        "The microphone works and speech recognition is available."
    } else {
        "Speech recognition is not available. Check Settings, Privacy, Microphone, and that speech is \
         installed for your display language."
    });
    // SAFETY: as above.
    unsafe {
        let _ = EnableWindow(button, true);
    }
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
                x if x == ID_TEST_MIC as i32 => test_microphone(),
                x if x == ID_UPDATE as i32 => update_pressed(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn typed<'a>(title: &'a str, transparency: &'a str) -> Typed<'a> {
        Typed {
            title,
            transparency,
            blur_background: false,
            voice_enabled: false,
            password: "",
            password_again: "",
            master: "",
            master_again: "",
        }
    }

    #[test]
    fn appearance_only_changes_leave_both_credentials_alone() {
        let changes = validate(&typed("Locked", "0.75")).expect("valid");
        assert_eq!(changes.title, "Locked");
        assert_eq!(changes.transparency, 0.75);
        assert!(changes.password_hash.is_none());
        assert!(changes.master_hash.is_none());
        assert!(!changes.touches_credentials(), "no second authorisation needed");
    }

    #[test]
    fn the_title_cannot_be_blanked() {
        assert_eq!(validate(&typed("", "0.5")), Err(Problem::TitleEmpty));
        assert_eq!(validate(&typed("   ", "0.5")), Err(Problem::TitleEmpty));
    }

    #[test]
    fn transparency_is_checked_rather_than_silently_clamped() {
        // Loading a hand-edited config clamps, because an unopenable lock
        // screen is worse. Typing it here is a mistake worth reporting.
        assert_eq!(validate(&typed("Locked", "abc")), Err(Problem::TransparencyNotANumber));
        assert_eq!(validate(&typed("Locked", "")), Err(Problem::TransparencyNotANumber));
        assert_eq!(validate(&typed("Locked", "1.5")), Err(Problem::TransparencyOutOfRange));
        assert_eq!(validate(&typed("Locked", "-0.1")), Err(Problem::TransparencyOutOfRange));
        assert!(validate(&typed("Locked", "0")).is_ok());
        assert!(validate(&typed("Locked", "1")).is_ok());
    }

    #[test]
    fn a_half_filled_password_pair_is_reported_not_ignored() {
        // Silently discarding a password somebody believes they just set is the
        // worst available behaviour.
        let mut t = typed("Locked", "0.5");
        t.password = "a-long-enough-password";
        assert_eq!(validate(&t), Err(Problem::PasswordConfirmMissing));

        let mut t = typed("Locked", "0.5");
        t.password_again = "a-long-enough-password";
        assert_eq!(validate(&t), Err(Problem::PasswordConfirmMissing));

        let mut t = typed("Locked", "0.5");
        t.master = "a-long-enough-password";
        assert_eq!(validate(&t), Err(Problem::MasterConfirmMissing));
    }

    #[test]
    fn changing_one_credential_leaves_the_other_untouched() {
        let mut t = typed("Locked", "0.5");
        t.password = "a-brand-new-password";
        t.password_again = "a-brand-new-password";
        let changes = validate(&t).expect("valid");
        assert!(changes.password_hash.is_some());
        assert!(changes.master_hash.is_none(), "the master password was not being changed");
        assert!(changes.touches_credentials(), "and this needs the second authorisation");
    }

    #[test]
    fn both_changed_to_the_same_secret_is_refused() {
        let mut t = typed("Locked", "0.5");
        t.password = "the-same-thing-twice";
        t.password_again = "the-same-thing-twice";
        t.master = "the-same-thing-twice";
        t.master_again = "the-same-thing-twice";
        assert_eq!(validate(&t), Err(Problem::MasterSameAsPassword));
    }

    #[test]
    fn short_and_mismatched_new_passwords_are_refused() {
        let mut t = typed("Locked", "0.5");
        t.password = "abc";
        t.password_again = "abc";
        assert_eq!(validate(&t), Err(Problem::PasswordTooShort));

        let mut t = typed("Locked", "0.5");
        t.password = "a-long-enough-password";
        t.password_again = "a-different-long-one";
        assert_eq!(validate(&t), Err(Problem::PasswordsDiffer));
    }

    #[test]
    fn a_four_character_unlock_password_is_allowed_but_not_a_four_character_master() {
        // Changing only the unlock password, to four characters.
        let mut t = typed("Locked", "0.5");
        t.password = "1234";
        t.password_again = "1234";
        assert!(validate(&t).is_ok(), "four characters is allowed for the unlock password");

        // The same, for the master password, is not.
        let mut t = typed("Locked", "0.5");
        t.master = "1234";
        t.master_again = "1234";
        assert_eq!(validate(&t), Err(Problem::MasterTooShort), "the recovery credential keeps the longer floor");

        let mut t = typed("Locked", "0.5");
        t.master = "shortish";
        t.master_again = "shortish";
        assert!(validate(&t).is_ok(), "eight clears it");
    }

    #[test]
    fn the_two_messages_quote_the_two_different_floors() {
        // One message for both would be wrong for one of the fields. The
        // numbers come from `Kind` rather than being written out here, so this
        // fails if the message and the rule ever drift apart.
        let password = Problem::PasswordTooShort.message();
        let master = Problem::MasterTooShort.message();
        assert!(password.contains(&Kind::Password.min_length().to_string()), "{password}");
        assert!(master.contains(&Kind::Master.min_length().to_string()), "{master}");
        assert_ne!(password, master, "the two floors differ, so the two messages must");
    }

    #[test]
    fn applying_changes_preserves_credentials_that_were_not_set() {
        let original = Config {
            password_hash: Some(credential::hash("original-password").expect("hashed")),
            master_hash: Some(credential::hash("original-master-key").expect("hashed")),
            ..Config::default()
        };

        let changes = validate(&typed("Renamed", "0.25")).expect("valid");
        let updated = apply(original.clone(), changes);

        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.password_hash, original.password_hash, "untouched");
        assert_eq!(updated.master_hash, original.master_hash, "untouched");
        assert!(credential::verify("original-password", updated.password_hash.as_deref()).expect("verified"));
    }

    #[test]
    fn applying_a_new_password_replaces_only_that_one() {
        let original = Config {
            password_hash: Some(credential::hash("original-password").expect("hashed")),
            master_hash: Some(credential::hash("original-master-key").expect("hashed")),
            ..Config::default()
        };

        let mut t = typed("Locked", "0.5");
        t.password = "a-replacement-password";
        t.password_again = "a-replacement-password";
        let updated = apply(original.clone(), validate(&t).expect("valid"));

        assert!(credential::verify("a-replacement-password", updated.password_hash.as_deref()).expect("verified"));
        assert!(!credential::verify("original-password", updated.password_hash.as_deref()).expect("verified"));
        assert_eq!(updated.master_hash, original.master_hash, "the master password still works");
    }

    #[test]
    fn the_voice_switch_survives_a_round_trip() {
        let mut t = typed("Locked", "0.5");
        t.voice_enabled = true;
        let updated = apply(Config::default(), validate(&t).expect("valid"));
        assert!(updated.voice.enabled);
        // And the command and threshold are not disturbed by the switch.
        assert_eq!(updated.voice.confidence_threshold, Config::default().voice.confidence_threshold);
    }

    #[test]
    fn every_problem_has_an_actionable_message() {
        for problem in [
            Problem::TitleEmpty,
            Problem::TransparencyNotANumber,
            Problem::TransparencyOutOfRange,
            Problem::PasswordTooShort,
            Problem::PasswordsDiffer,
            Problem::MasterTooShort,
            Problem::MastersDiffer,
            Problem::MasterSameAsPassword,
            Problem::PasswordConfirmMissing,
            Problem::MasterConfirmMissing,
        ] {
            let message = problem.message();
            assert!(message.ends_with('.'), "{problem:?}: {message}");
            assert_eq!(message, message.trim(), "{problem:?}: stray whitespace");
        }
    }
}
