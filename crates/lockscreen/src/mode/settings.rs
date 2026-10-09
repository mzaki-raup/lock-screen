//! The settings window. Phase 05.
//!
//! Must authenticate *before* it opens, and again before any credential
//! change -- defect D7, where the .NET `Save_Click` rewrites the lock password
//! without asking for the current one or invoking Windows Hello, so the overlay
//! can be reset by anyone who reaches the desktop.
//!
//! Also where the version is shown and where the updater is reachable from
//! (phase 07) -- never from the overlay, which would mean a path that writes a
//! new executable while the machine is locked.

use std::path::Path;

use lockscreen_core::config::Config;

pub fn run(_config_path: &Path, _config: Config) -> Result<(), String> {
    Err(format!(
        "Settings are not implemented yet (phase 05).\n\n\
         Lock Screen {}\n\n\
         When this lands it will ask you to authenticate before opening, which \
         the .NET version never did.",
        lockscreen_core::VERSION
    ))
}
