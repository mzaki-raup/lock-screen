//! The system sounds the UI plays.
//!
//! `System.Media.SystemSounds` is a thin wrapper over `MessageBeep`, so these
//! are the same sounds the .NET app produced, reached by the same call. Every
//! one is best-effort: a machine with no audio device should still unlock.

/// Which sound to play. Named for the moment rather than the Win32 constant, so
/// call sites read as intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// The lock screen has appeared.
    Started,
    /// Unlocked successfully.
    Unlocked,
    /// Wrong password.
    Rejected,
    /// Listening for a voice command.
    Listening,
}

#[cfg(windows)]
pub fn play(sound: Sound) {
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONHAND, MB_OK, MESSAGEBOX_STYLE,
    };

    let style: MESSAGEBOX_STYLE = match sound {
        Sound::Started => MB_ICONASTERISK,
        Sound::Unlocked => MB_ICONEXCLAMATION,
        Sound::Rejected => MB_ICONHAND,
        Sound::Listening => MB_OK,
    };

    // SAFETY: MessageBeep takes a value, touches no memory of ours, and is safe
    // to call from any thread. Its result is ignored on purpose -- a missing
    // audio device must not stop someone unlocking their screen.
    unsafe {
        let _ = MessageBeep(style);
    }
}

#[cfg(not(windows))]
pub fn play(_sound: Sound) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_can_be_played_without_panicking() {
        for sound in [Sound::Started, Sound::Unlocked, Sound::Rejected, Sound::Listening] {
            play(sound);
        }
    }
}
