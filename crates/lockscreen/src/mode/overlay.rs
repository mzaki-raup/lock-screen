//! Overlay mode: lock the screen and hold it until the user authenticates.
//!
//! Thin on purpose. Everything that touches windows lives in
//! [`crate::win::overlay`]; this decides what an outcome means.

use std::path::Path;

use lockscreen_core::config::Config;
use lockscreen_core::{instance, log};

use crate::win::{overlay, unlock};

pub fn run(_config_path: &Path, config: Config) -> Result<(), String> {
    // Claimed before anything is drawn or hooked. Two overlays would mean two
    // sets of global hooks and two processes each believing it owns the Task
    // Manager policy, so unlocking one would clear the policy while the other
    // was still up -- which is what the .NET version does, having no guard.
    let _instance = match instance::Instance::claim() {
        Ok(instance) => instance,
        // Already locked, which is what whoever pressed the hotkey twice
        // wanted. Exit quietly: telling them so would be noise.
        Err(instance::Taken::AlreadyRunning) => return Ok(()),
        // Something is wrong with the session rather than with us. Lock anyway;
        // an unguarded lock screen beats no lock screen.
        Err(instance::Taken::Unavailable) => {
            log::warn("the single-instance mutex could not be created; locking without a guard");
            return lock(&config);
        }
    };
    lock(&config)
}

/// The lock itself, once the instance is claimed.
fn lock(config: &Config) -> Result<(), String> {
    for screen in crate::win::monitors::all() {
        log::info(&format!(
            "covering {} {}x{} at ({},{}) dpi {}",
            screen.device,
            screen.width(),
            screen.height(),
            screen.bounds.left,
            screen.bounds.top,
            screen.dpi()
        ));
    }

    match overlay::Session::run(config)? {
        overlay::Outcome::Unlocked(unlock::Via::Master) => {
            log::audit("master password used to unlock");
            Ok(())
        }
        overlay::Outcome::Unlocked(_) => Ok(()),
        // GetMessageW returned WM_QUIT, or every monitor went away. Neither is
        // an unlock, so say so rather than exiting 0 and looking like one.
        overlay::Outcome::Interrupted => Err("The lock screen was interrupted before it was unlocked.".to_owned()),
        overlay::Outcome::PromptFailed => {
            log::warn("the unlock prompt could not be opened; the lock was released");
            Err("The unlock prompt could not be opened, so the lock has been released rather than \
                 leaving the keyboard captured.\n\n\
                 This is a bug. There is a note of it in LockScreenData/lockscreen.log."
                .to_owned())
        }
    }
}
