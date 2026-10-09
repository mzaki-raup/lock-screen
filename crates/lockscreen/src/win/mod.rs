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
pub mod monitors;
pub mod overlay;
pub mod ui;
pub mod unlock;

/// This module's `HINSTANCE`, for window class registration.
pub fn instance() -> HINSTANCE {
    // SAFETY: a null module name asks for the running executable, which cannot
    // fail for a loaded process.
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}
