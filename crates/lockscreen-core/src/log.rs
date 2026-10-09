//! One place that writes to `LockScreenData/lockscreen.log`.
//!
//! Replaces `catch { }`. The .NET version swallows failures in seven places --
//! `DisableSystemKeys`, `EnableSystemKeys`, both `EncryptionHelper` paths, all
//! three `VoiceConfigHelper` methods -- so a lock screen that silently stopped
//! suppressing keys, or silently fell back to treating a config value as a
//! plaintext password, looks exactly like one that worked.
//!
//! Deliberately minimal. No log crate, no levels beyond what is used, no
//! rotation beyond a size cap: a lock screen that fails to log must still lock,
//! so every function here is best-effort and returns nothing.
//!
//! # What must never be written here
//!
//! Passwords, hashes, or anything recovered from the legacy DPAPI value. The
//! log records *that* a credential was used or changed, never what it was. A
//! log beside a portable executable travels with the folder.

use std::fmt::Write as _;
use std::io::Write as _;

use crate::paths;

/// Above this, the log is truncated back to empty rather than grown forever.
///
/// A ring buffer would be better and is not worth it: this file exists to
/// answer "what happened just now", and a portable app should not quietly
/// accumulate megabytes in the folder somebody dropped it into.
const MAX_BYTES: u64 = 256 * 1024;

/// How much the entry matters. Only three, because only three are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Something happened that is worth being able to reconstruct later.
    Info,
    /// Something failed, but the lock carried on. The .NET `catch { }` cases.
    Warn,
    /// Security-relevant: a credential was used or changed.
    Audit,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "info ",
            Level::Warn => "warn ",
            Level::Audit => "audit",
        }
    }
}

/// Append one line. Best-effort: a failure here is never worth reporting,
/// because the only place to report it would be this file.
pub fn write(level: Level, message: &str) {
    let Ok(dir) = paths::data_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("lockscreen.log");

    // Truncate rather than grow without bound. Checked before the append so a
    // single run cannot be the one that overshoots.
    if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::write(&path, b"");
    }

    let mut line = String::with_capacity(message.len() + 32);
    let _ = write!(line, "{} {} {message}", stamp(), level.tag());

    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

pub fn info(message: &str) {
    write(Level::Info, message);
}

pub fn warn(message: &str) {
    write(Level::Warn, message);
}

/// A credential was used or changed. The one category worth keeping.
pub fn audit(message: &str) {
    write(Level::Audit, message);
}

/// Local time, as `YYYY-MM-DD HH:MM:SS`.
///
/// Hand-rolled from `GetLocalTime` rather than taking a date-time crate for one
/// format string. Local rather than UTC on purpose: somebody reading this is
/// matching it against when they remember being at the machine.
#[cfg(windows)]
fn stamp() -> String {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::SystemInformation::GetLocalTime;

    // SAFETY: GetLocalTime fills a structure it is given by value-return and
    // touches nothing else.
    let t: SYSTEMTIME = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
fn stamp() -> String {
    "--------- --:--:--".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timestamp_has_the_shape_a_reader_expects() {
        let s = stamp();
        assert_eq!(s.len(), 19, "{s}");
        assert_eq!(s.chars().filter(|c| *c == '-').count(), 2, "{s}");
        assert_eq!(s.chars().filter(|c| *c == ':').count(), 2, "{s}");
    }

    #[cfg(windows)]
    #[test]
    fn the_timestamp_is_a_plausible_date() {
        let s = stamp();
        let year: i32 = s[..4].parse().expect("a year");
        assert!(year >= 2026, "the clock says {year}; GetLocalTime was misread");
        let month: u32 = s[5..7].parse().expect("a month");
        assert!((1..=12).contains(&month), "{s}");
        let day: u32 = s[8..10].parse().expect("a day");
        assert!((1..=31).contains(&day), "{s}");
    }

    #[test]
    fn every_level_has_a_fixed_width_tag_so_lines_align() {
        let widths: Vec<usize> = [Level::Info, Level::Warn, Level::Audit].iter().map(|l| l.tag().len()).collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "tags must be the same width: {widths:?}");
    }

    #[test]
    fn writing_is_best_effort_and_never_panics() {
        // Whatever the state of the folder beside the test binary, none of
        // these may unwind: a lock screen that cannot log must still lock.
        info("test: an informational line");
        warn("test: a warning line");
        audit("test: an audit line");
    }
}
