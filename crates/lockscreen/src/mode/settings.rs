//! Settings mode, and the gates that make it safe to have.
//!
//! This is the fix for defect D7. `SettingsApp.Save_Click` changes the lock
//! password without asking for the current one and without invoking Windows
//! Hello, so anyone who reaches an unattended desktop can reset it -- the
//! overlay's protection is one double-click from being bypassed.
//!
//! Two gates, not one:
//!
//! 1. **Before the window opens.** Nothing is shown, and no setting can be read
//!    or written, until the unlock ladder is satisfied.
//! 2. **Before a credential change is written.** A settings window left open on
//!    a desk is precisely what the first gate does not cover, so replacing a
//!    password asks again.
//!
//! Appearance changes do not ask twice. The second gate is about credentials;
//! making someone re-authenticate to adjust a transparency slider would teach
//! them to click through the prompt without reading it, which is worse than not
//! having it.

use std::path::Path;

use lockscreen_core::config::{self, Config};
use windows::Win32::Foundation::HWND;

use crate::win::{settings, unlock};

pub fn run(config_path: &Path, config: Config) -> Result<(), String> {
    // Gate 1. No owner window: settings mode runs with no overlay, and the
    // prompt centres itself on the monitor holding the pointer.
    //
    // The ladder is the same one that unlocks the screen, minus voice -- see
    // `unlock::Purpose::Authorize`. One implementation, so the two cannot drift
    // into disagreeing about who is allowed in.
    match unlock::ask(HWND::default(), &config, unlock::Purpose::Authorize) {
        unlock::Answer::Unlocked(via) => {
            if via == unlock::Via::Master {
                audit("master password used to open settings");
            }
        }
        unlock::Answer::Cancelled => {
            return Err("Settings were not opened: nothing was changed.".to_owned());
        }
    }

    let Some(changes) = settings::edit(&config) else {
        return Err("Settings were closed without saving.".to_owned());
    };

    // Gate 2. Only for a credential change, and only after the form has already
    // been validated, so nobody is asked to authenticate for a save that was
    // going to be rejected anyway.
    if changes.touches_credentials() {
        match unlock::ask(HWND::default(), &config, unlock::Purpose::Authorize) {
            unlock::Answer::Unlocked(via) => {
                if via == unlock::Via::Master {
                    audit("master password used to authorise a credential change");
                }
            }
            unlock::Answer::Cancelled => {
                return Err("The new password was not saved: the change was not authorised.\n\n\
                            Any other settings you changed were not saved either."
                    .to_owned());
            }
        }
    }

    let touched_credentials = changes.touches_credentials();
    let updated = settings::apply(config, changes);
    config::save(config_path, &updated).map_err(|e| format!("{} could not be written: {e}", config_path.display()))?;

    if touched_credentials {
        audit("a credential was changed from settings");
    }

    Err(format!("Settings saved to\n{}", config_path.display()))
}

/// Append one line to `LockScreenData/lockscreen.log` beside the executable.
///
/// Best-effort, and deliberately sparse: who authorised what, never what they
/// typed. Phase 06 replaces this with real structured logging.
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
