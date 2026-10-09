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

        println!("Lock Screen {}", lockscreen_core::VERSION);

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
