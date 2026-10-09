//! Setup mode: choose the two passwords and write them down as verifiers.
//!
//! Thin on purpose; the window lives in [`crate::win::setup`].
//!
//! **This mode never changes an existing credential.** Overwriting one without
//! asking for the current password is exactly defect D7, which is what
//! `SettingsApp.Save_Click` does today -- so a copy that is already set up is
//! turned away here and sent to settings mode, which authenticates first. The
//! guard is in `main.rs`, where the decision of which mode to run is made; this
//! function asserts it rather than trusting it.

use std::path::Path;

use lockscreen_core::config::{self, Config};

use crate::win::setup;

pub fn run(config_path: &Path, config: Config) -> Result<(), String> {
    debug_assert!(config.needs_setup(), "setup must not be reachable for a configured copy; see D7");

    // A config written by the .NET app has settings worth keeping and a
    // password that cannot be: DPAPI binds it to one user on one machine, so it
    // would not survive the folder being copied anywhere. Saying so beats
    // letting someone wonder why their old password stopped working.
    let migrating = config.legacy_password.is_some();

    let Some(chosen) = setup::collect(migrating, config_path) else {
        return Err("Setup was cancelled, so nothing has been saved and the screen will not lock.".to_owned());
    };

    let config = setup::apply(config, chosen);
    config::save(config_path, &config).map_err(|e| format!("{} could not be written: {e}", config_path.display()))?;

    Err(format!(
        "Setup is complete. Both passwords are stored as Argon2id hashes in\n{}\n\n\
         Run LockScreen.exe again to lock the screen.",
        config_path.display()
    ))
}
