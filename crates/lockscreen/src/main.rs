//! `LockScreen.exe` — one executable, three modes.
//!
//! ```text
//! LockScreen.exe              the overlay      (default, no arguments)
//! LockScreen.exe --settings   the configurator (requires authentication)
//! LockScreen.exe --setup      first-run setup  (only with no verifier stored)
//! ```
//!
//! One binary rather than two because each Rust binary links its own copy of
//! what it uses -- a GUI crate paid for twice -- and, more importantly, because
//! two executables can be updated independently and then disagree about the
//! config format. One binary cannot disagree with itself.
//!
//! **Mode dispatch is not a security boundary.** `--settings` is a flag anyone
//! can pass. The gate is authentication, which is why settings mode
//! authenticates before it opens a window rather than relying on how it was
//! started. Each mode also runs in its own process, so the overlay's hooks are
//! never loaded alongside the settings UI.

// A GUI subsystem binary has no console attached, so printing goes nowhere
// unless something calls AttachConsole first. See `report` below.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod mode;
#[cfg(windows)]
mod win;

use std::process::ExitCode;

use lockscreen_core::{config, paths};

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Overlay,
    Settings,
    Setup,
    Version,
    Help,
}

fn main() -> ExitCode {
    let mode = match parse_args(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(unknown) => {
            report(&format!("LockScreen: unrecognised option `{unknown}`\n\n{USAGE}"));
            return ExitCode::FAILURE;
        }
    };

    match run(mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            report(&message);
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
Usage: LockScreen.exe [OPTION]

    (no option)     lock the screen
    --settings      open settings (asks you to authenticate first)
    --setup         choose the master and unlock passwords
    --version       print the version
    --help          print this message

Settings are kept in config.ini beside this executable.";

/// Shown when `--setup` is asked of a copy that already has credentials.
///
/// Setup must never rewrite an existing credential: changing a password without
/// asking for the current one is defect D7, and a mode flag is not an
/// authentication check -- anyone can pass one.
const ALREADY_SET_UP: &str = "\
This copy is already set up.

To change the passwords, run LockScreen.exe --settings, which asks you to \
authenticate first.";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Mode, String> {
    let mut mode = Mode::Overlay;
    for arg in args {
        mode = match arg.as_str() {
            "--settings" | "-s" => Mode::Settings,
            "--setup" => Mode::Setup,
            "--version" | "-V" => Mode::Version,
            "--help" | "-h" | "/?" => Mode::Help,
            other => return Err(other.to_owned()),
        };
    }
    Ok(mode)
}

fn run(mode: Mode) -> Result<(), String> {
    if let Mode::Version = mode {
        report(&version_line());
        return Ok(());
    }
    if let Mode::Help = mode {
        report(USAGE);
        return Ok(());
    }

    let dir = paths::exe_dir().map_err(|e| format!("Can't work out where this program lives: {e}"))?;
    let config_path = dir.join(paths::CONFIG_FILE);

    // Portable, so the app's own folder has to be writable. Check it up front:
    // discovering it at save time, after someone has typed a new password, is
    // the one outcome worth engineering against.
    if !paths::is_dir_writable(&dir) {
        return Err(format!(
            "Settings can't be saved: {} is read-only.\n\n\
             Move the app somewhere you can write, such as a folder in your user profile.",
            dir.display()
        ));
    }

    let config = config::load(&config_path).map_err(|e| format!("{} could not be read: {e}", config_path.display()))?;

    match mode {
        // `--setup` must never rewrite a credential that already exists.
        // Changing a password without asking for the current one is defect D7,
        // and a mode flag is not an authentication check -- anyone can pass it.
        // A configured copy is sent to settings mode, which authenticates.
        Mode::Setup if !config.needs_setup() => Err(ALREADY_SET_UP.to_owned()),

        // First run, or a config carried over from the .NET app, which has no
        // Argon2 record in it. The overlay must not start in this state: there
        // is nothing to unlock with, and inventing `1234` and showing it in a
        // dialog is exactly what this port removes.
        Mode::Setup => mode::setup::run(&config_path, config),
        _ if config.needs_setup() => mode::setup::run(&config_path, config),

        Mode::Overlay => mode::overlay::run(&config_path, config),
        Mode::Settings => mode::settings::run(&config_path, config),
        Mode::Version | Mode::Help => unreachable!("handled above"),
    }
}

fn version_line() -> String {
    format!(
        "Lock Screen {} ({}){}",
        lockscreen_core::VERSION,
        env!("LOCKSCREEN_COMMIT"),
        match env!("LOCKSCREEN_BUILD_DATE") {
            "unknown" => String::new(),
            date => format!(" built {date}"),
        }
    )
}

/// Say something to the user.
///
/// A GUI-subsystem binary has no stdout to write to, so this attaches to the
/// parent console when there is one -- someone running `--version` from a
/// terminal -- and otherwise falls back to a message box. Phase 06 replaces the
/// message box with the real UI; until then it is better than silence.
fn report(message: &str) {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // SAFETY: no arguments to get wrong; fails harmlessly when there is no
        // parent console, which is the case when launched from Explorer.
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok();
        if attached {
            println!("{message}");
            return;
        }

        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
        use windows::core::HSTRING;
        let text = HSTRING::from(message);
        let caption = HSTRING::from("Lock Screen");
        // SAFETY: both strings outlive the call.
        unsafe {
            MessageBoxW(None, &text, &caption, MB_OK | MB_ICONINFORMATION);
        }
    }
    #[cfg(not(windows))]
    println!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Mode, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn no_arguments_means_lock_the_screen() {
        assert_eq!(parse(&[]), Ok(Mode::Overlay));
    }

    #[test]
    fn each_mode_has_a_flag() {
        assert_eq!(parse(&["--settings"]), Ok(Mode::Settings));
        assert_eq!(parse(&["-s"]), Ok(Mode::Settings));
        assert_eq!(parse(&["--setup"]), Ok(Mode::Setup));
        assert_eq!(parse(&["--version"]), Ok(Mode::Version));
        assert_eq!(parse(&["--help"]), Ok(Mode::Help));
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_silently_locking_the_screen() {
        assert_eq!(parse(&["--sttings"]), Err("--sttings".to_owned()));
        assert_eq!(parse(&["/elevate"]), Err("/elevate".to_owned()));
    }

    #[test]
    fn the_already_set_up_message_points_at_the_authenticated_path() {
        // The user-facing half of the D7 guard: refusing is only useful if it
        // says where to go instead.
        assert!(ALREADY_SET_UP.contains("--settings"));
        assert!(ALREADY_SET_UP.contains("authenticate"));
        // And it must not be mangled by a stray line continuation.
        for line in ALREADY_SET_UP.lines() {
            assert_eq!(line, line.trim_start(), "stray indentation in a user-facing message: {line:?}");
        }
    }

    #[test]
    fn no_user_facing_message_carries_stray_indentation() {
        for line in USAGE.lines().filter(|l| !l.starts_with("    ")) {
            assert_eq!(line, line.trim_start(), "{line:?}");
        }
    }

    #[test]
    fn the_version_line_names_the_package_version() {
        let line = version_line();
        assert!(line.starts_with("Lock Screen "), "{line}");
        assert!(line.contains(lockscreen_core::VERSION), "{line}");
    }
}
