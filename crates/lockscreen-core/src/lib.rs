//! Everything both modes of `LockScreen.exe` need, linked once.
//!
//! The app is portable: one executable, and a `config.ini` beside it. Nothing
//! is written to `%APPDATA%`, `%LOCALAPPDATA%` or `%ProgramData%`, and there is
//! no installer. See `RUST-MIGRATION.md` §8.
//!
//! The modules split along the lines the port needs to keep straight:
//!
//! - [`paths`] resolves everything from the *executable's* directory, which is
//!   not the working directory. Confusing the two is the bug the .NET version
//!   has (defect D6), so it lives in one place and nowhere else.
//! - [`config`] reads and writes that file, including ones written by the .NET
//!   app -- spaces around `=`, a UTF-8 BOM and all.
//! - [`credential`] verifies passwords against Argon2id hashes, and can still
//!   read the old DPAPI-encrypted value in order to migrate away from it.
//! - [`policy`] holds the Task Manager policy key, restoring it on drop.
//! - [`instance`] makes sure only one lock screen runs per session.
//! - [`log`] is the one place anything is written to disk for diagnostics,
//!   replacing the seven `catch { }` blocks the .NET version swallows failures
//!   in.
//! - [`release`] decides which published release, if any, is worth offering.
//! - [`sound`] is the handful of system sounds the UI plays.

pub mod config;
pub mod credential;
pub mod log;
pub mod paths;
pub mod release;
pub mod sound;

#[cfg(windows)]
pub mod instance;
#[cfg(windows)]
pub mod policy;

/// The version this copy was built as, from `Cargo.toml`.
///
/// Shown in settings mode only -- never on the lock overlay, where it would
/// tell whoever is standing at the machine which build to look up.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
