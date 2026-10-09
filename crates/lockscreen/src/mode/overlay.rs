//! Overlay mode: lock the screen and hold it until the user authenticates.
//!
//! Thin on purpose. Everything that touches windows lives in
//! [`crate::win::overlay`]; this decides what an outcome means.

use std::path::Path;

use lockscreen_core::config::Config;

use crate::win::{overlay, unlock};

pub fn run(_config_path: &Path, config: Config) -> Result<(), String> {
    match overlay::Session::run(&config)? {
        overlay::Outcome::Unlocked(unlock::Via::Master) => {
            // A recovery credential was used. Phase 06 replaces this with real
            // structured logging; until then the fact is at least recorded
            // rather than lost.
            audit("master password used to unlock");
            Ok(())
        }
        overlay::Outcome::Unlocked(_) => Ok(()),
        // GetMessageW returned WM_QUIT, or every monitor went away. Neither is
        // an unlock, so say so rather than exiting 0 and looking like one.
        overlay::Outcome::Interrupted => Err("The lock screen was interrupted before it was unlocked.".to_owned()),
    }
}

/// Append one line to `LockScreenData/lockscreen.log` beside the executable.
///
/// Best-effort: failing to write a log line must never stop someone unlocking
/// their own machine.
fn audit(line: &str) {
    use std::io::Write;

    let Ok(dir) = lockscreen_core::paths::data_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("lockscreen.log")) {
        let _ = writeln!(file, "{line}");
    }
}
