//! Win32 windows, written against `windows-rs` directly.
//!
//! No GUI toolkit. That is the house pattern -- `flex-taskbar` builds a bar,
//! flyouts, a search window and several settings windows this way, 16,000
//! lines of it -- and for a lock screen it is also the safer choice: measured
//! against a minimal password dialog, `eframe` compiles `glow`, `wgpu`, `ash`
//! and `naga` and has no software renderer at all, while this binary needs no
//! GPU context to put a password box on screen. A lock screen whose prompt
//! cannot render has locked the user out of their own machine.
//!
//! It is also 356 KB rather than ~10 MB, and the OS edit control brings IME,
//! accessibility and clipboard behaviour with it.

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

pub mod backdrop;
pub mod hello;
pub mod hooks;
pub mod http;
pub mod monitors;
pub mod overlay;
pub mod settings;
pub mod setup;
pub mod ui;
pub mod unlock;
pub mod update;
pub mod voice;

/// This module's `HINSTANCE`, for window class registration.
pub fn instance() -> HINSTANCE {
    // SAFETY: a null module name asks for the running executable, which cannot
    // fail for a loaded process.
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

/// Posted to a prompt's window by a worker thread that has just put an answer
/// on its channel.
///
/// Nothing reads the message. Its only job is to make `GetMessageW` return, so
/// the loop goes round and polls the channel.
pub const WM_APP_WORKER_DONE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 9;

/// Wake a window's message loop from a worker thread.
///
/// This is the fix for a defect that shipped in 0.1.0: Windows Hello would
/// verify, and the prompt would sit there with the password box still showing
/// until something unrelated happened -- a mouse move, a repaint -- because a
/// send on an `mpsc` channel does not wake `GetMessageW`. The worker put the
/// answer in the channel and the UI thread was parked waiting for a *window
/// message*, which is a different thing entirely. The delay looked like
/// slowness in Hello; it was the loop never being told.
///
/// `PostMessageW` is the documented way to do this and is explicitly safe to
/// call from any thread. The worker still touches no window *state*: it posts
/// one message carrying nothing and returns.
///
/// Takes the handle as an `isize` because `HWND` is not `Send`. That is sound:
/// a window handle is a kernel-side identifier, not a pointer into this
/// process's address space, and a handle that has since died makes
/// `PostMessageW` fail rather than do something unpredictable.
pub fn wake_message_loop(raw_hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    // SAFETY: see the note above on the handle and on cross-thread posting. A
    // failure here means the window is gone, which means the prompt closed
    // while the worker was still running -- nothing to do about it and nothing
    // to report.
    unsafe {
        let _ = PostMessageW(Some(HWND(raw_hwnd as *mut _)), WM_APP_WORKER_DONE, WPARAM(0), LPARAM(0));
    }
}

#[cfg(test)]
mod tests {
    /// A support report: everything the overlay's behaviour depends on that is
    /// a property of *this machine* rather than of the code.
    ///
    /// Added in phase 08. The parity checks that remain are the ones only a
    /// person sitting at the keyboard can do -- the overlay covers the screen
    /// and Windows Hello raises a prompt -- and those are worth nothing unless
    /// you know what the machine can actually do first. Running this is the
    /// step before working through the checklist in `PARITY.md`.
    ///
    /// Ignored: it asserts nothing, probing speech takes about a second, and
    /// the answers change with the hardware.
    ///
    /// ```text
    /// cargo test -p lockscreen --bin LockScreen environment -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "diagnostic; prints what this machine can do"]
    fn environment() {
        use lockscreen_core::{config, paths};

        println!("LockScreen {}", lockscreen_core::VERSION);

        match paths::exe_dir() {
            Ok(dir) => {
                println!(
                    "  folder      {} ({})",
                    dir.display(),
                    if paths::is_dir_writable(&dir) { "writable" } else { "READ-ONLY -- settings cannot be saved" }
                );
                let path = dir.join(paths::CONFIG_FILE);
                match config::load(&path) {
                    Ok(config) if config.needs_setup() => println!("  config      {} needs setup", path.display()),
                    Ok(config) => println!(
                        "  config      {} ready, voice {}",
                        path.display(),
                        if config.voice.enabled { "on" } else { "off" }
                    ),
                    Err(e) => println!("  config      {} unreadable: {e}", path.display()),
                }
            }
            Err(e) => println!("  folder      could not be found: {e}"),
        }

        for screen in super::monitors::all() {
            println!(
                "  monitor     {:<14} {}x{} at ({},{}) dpi {} ({}%){}",
                screen.device,
                screen.width(),
                screen.height(),
                screen.bounds.left,
                screen.bounds.top,
                screen.dpi(),
                screen.dpi() * 100 / 96,
                if screen.primary { " primary" } else { "" }
            );
        }

        println!(
            "  hello       {}",
            if super::hello::is_available() {
                "available"
            } else {
                "not available -- the prompt will go straight to the password"
            }
        );
        println!(
            "  voice       {}",
            if super::voice::is_available("unlock") {
                "a recogniser compiled, so the microphone path works"
            } else {
                "unavailable -- no microphone, no speech for this language, or privacy settings say no"
            }
        );
    }
}
