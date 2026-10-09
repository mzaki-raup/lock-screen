//! First-run setup. Phase 03.
//!
//! Collects the master password and the unlock password, each confirmed twice
//! and checked against `credential::is_acceptable`, then writes both Argon2id
//! verifiers. There is no default password and nothing is ever displayed --
//! replacing `IniHelper.EnsureConfigExists()`, which wrote `Password=1234` and
//! then showed it in a dialog.
//!
//! Deliberately installs no keyboard hook and writes no registry policy, so an
//! abandoned setup leaves the system untouched.

use std::path::Path;

use lockscreen_core::config::Config;

pub fn run(config_path: &Path, config: Config) -> Result<(), String> {
    let carried_over = config.legacy_password.is_some();
    Err(format!(
        "First-run setup is not implemented yet (phase 03).\n\n\
         {}\n\n\
         When this lands it will ask for a master password and an unlock \
         password, store Argon2id verifiers for both in\n{}\n\
         and never write a default password.",
        if carried_over {
            "This config was written by the .NET version: its settings will be kept, \
             and its reversible DPAPI password dropped."
        } else {
            "No password is set up yet."
        },
        config_path.display(),
    ))
}
