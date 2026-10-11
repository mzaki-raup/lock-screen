//! Checking for, downloading and installing a newer release.
//!
//! Adapted from `flex-taskbar/src/win/update.rs`, which reads the same forge.
//! The decision logic lives in [`lockscreen_core::release`] so it can be tested
//! without a network; this module is the I/O.
//!
//! Installing is deliberately dull:
//!
//! 1. the program is downloaded into `LockScreenData/update/`;
//! 2. its SHA-256 must match the `SHA256SUMS.txt` published beside it -- a
//!    release without one is never installed, only opened in a browser;
//! 3. the running `LockScreen.exe` is **renamed** aside (Windows will not let a
//!    running image be overwritten, but it will let it be renamed) and the new
//!    one takes its place; anything that fails puts the old name back;
//! 4. the leftover is deleted on the next start.
//!
//! # Two deviations from the sibling, both because this is a lock screen
//!
//! - **Settings mode only.** Checking and installing are unreachable from the
//!   overlay. Otherwise someone at a locked desktop would have a path that
//!   writes a new executable, and there is no reason to hand them one.
//! - **No restart.** `flex-taskbar` exits with a restart code and lets a
//!   supervisor relaunch it. Doing that here could relaunch the *overlay*, and
//!   restarting a lock screen mid-session is a bypass rather than a
//!   convenience. The install finishes and the user starts it again themselves.
//!
//! There is also no background check. The sibling looks once a day; this does
//! not, because the only mode that may check is one you have already
//! authenticated into, and a timer that fires while the screen is locked would
//! be exactly the path the first deviation closes.

use std::path::{Path, PathBuf};

use lockscreen_core::release::{self, State, Update, Version};
use lockscreen_core::{log, paths};

/// Where this copy looks for its own releases.
///
/// The program is published from two forges -- the self-hosted Forgejo and a
/// GitHub mirror of the same history -- and a copy must update from the one it
/// came from. So the URL is fixed **when the program is built**, by whichever
/// release workflow is building it, through `LOCKSCREEN_RELEASES_API`. A build
/// with nothing set, which is every local build, reads the Forgejo.
///
/// Fixed at build time and not read from `config.ini`, deliberately: a setting
/// could be pointed anywhere by anyone who can edit the file, and the whole
/// updater would then install whatever that place served. The build is the
/// only input the updater trusts about *where* to look. See `build.rs` for the
/// rebuild guard, which matters on a runner whose target directory is shared
/// between builds for different forges.
///
/// Both forges answer this in the same shape -- Gitea copied GitHub's releases
/// API and Forgejo kept it -- so nothing else here cares which one it is. The
/// query differs only in name: `limit` for Forgejo, `per_page` for GitHub.
pub const RELEASES_API: &str = match option_env!("LOCKSCREEN_RELEASES_API") {
    Some(url) => url,
    None => DEFAULT_RELEASES_API,
};

/// The Forgejo, which is where development happens and where a build with no
/// `LOCKSCREEN_RELEASES_API` set looks.
const DEFAULT_RELEASES_API: &str = "https://git.zacx.net/api/v1/repos/owner/lock-screen/releases?limit=10";

/// Nothing the API answers with should be anywhere near this large.
const MAX_JSON: usize = 1 << 20;
/// Nor should the program be: it is well under a megabyte.
const MAX_EXE: usize = 32 << 20;

/// The name the running program is renamed to while it is replaced.
const OLD_EXE: &str = "LockScreen.old.exe";

/// The caps are invariants, not behaviour, so they are checked when the program
/// is built rather than when its tests are run.
///
/// The binary is around 580 KB today: the cap wants headroom for it to grow
/// without being so large that a hostile answer could exhaust memory before the
/// limit bites.
const _: () = {
    assert!(MAX_EXE > 4 << 20, "too tight to survive the binary growing");
    assert!(MAX_EXE <= 64 << 20, "too loose to be a meaningful cap");
    assert!(MAX_JSON < MAX_EXE, "an API answer should never be allowed to be larger than the program");
};

/// Ask the forge what it has. Blocking; call it off the UI thread.
pub fn check() -> State {
    // Logged, because which forge a copy reads is decided by who built it and
    // is otherwise invisible: a copy from GitHub that quietly read the Forgejo
    // would look fine in every other respect.
    log::info(&format!("checking for updates at {RELEASES_API}"));
    match super::http::get(RELEASES_API, MAX_JSON) {
        Ok(body) => match String::from_utf8(body) {
            Ok(json) => match release::newer_than(&json, Version::running()) {
                Some(update) => {
                    log::info(&format!("update available: {}", update.version));
                    State::Available(update.version)
                }
                None => State::UpToDate,
            },
            Err(_) => {
                log::warn("the forge's answer was not text");
                State::Failed
            }
        },
        Err(e) => {
            log::warn(&format!("the update check failed: {e}"));
            State::Failed
        }
    }
}

/// Ask again and return the release itself, so the caller can install it.
///
/// Checked a second time rather than cached from [`check`]: minutes may have
/// passed while somebody read the notes, and the answer is cheap.
pub fn fetch_update() -> Result<Update, String> {
    let body = super::http::get(RELEASES_API, MAX_JSON)?;
    let json = String::from_utf8(body).map_err(|_| "the forge's answer was not text".to_owned())?;
    release::newer_than(&json, Version::running()).ok_or_else(|| "there is no newer release".to_owned())
}

/// Download the program and check it against the published checksum.
///
/// Returns where it was written. Nothing is replaced yet.
pub fn download(update: &Update) -> Result<PathBuf, String> {
    // No checksums, no automatic install. The only check available would be
    // "the forge said so", and a truncated download would be written over the
    // running program.
    let sums = update.sums.as_ref().ok_or("the release has no checksums, so it will not be installed")?;

    let sums = super::http::get(&sums.url, MAX_JSON)?;
    let sums = String::from_utf8(sums).map_err(|_| "the checksums file wasn't text".to_owned())?;
    let want = release::sha256_of(&sums, &update.exe.name)
        .ok_or_else(|| format!("{} isn't listed in {}", update.exe.name, release::SUMS_ASSET))?;

    let body = super::http::get(&update.exe.url, MAX_EXE)?;
    let got = super::http::sha256(&body).ok_or("the download could not be hashed")?;
    if got != want {
        log::warn(&format!("a download of {} did not match its published checksum", update.version));
        return Err(format!(
            "the download does not match the checksum published with it, so it was thrown away.\n\n\
             Expected {want}\nGot      {got}"
        ));
    }

    let dir = paths::data_dir().map_err(|e| format!("the data folder could not be found: {e}"))?.join("update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} couldn't be made: {e}", dir.display()))?;
    let file = dir.join(format!("LockScreen-{}.exe", update.version));
    std::fs::write(&file, &body).map_err(|e| format!("{} couldn't be written: {e}", file.display()))?;
    Ok(file)
}

/// Put `new` where the running program is.
///
/// The running image is renamed aside first and put back if anything fails, so
/// a failure leaves a working program rather than none at all.
pub fn install(new: &Path) -> Result<(), String> {
    let dir = paths::exe_dir().map_err(|e| format!("the program's folder could not be found: {e}"))?;
    let exe = dir.join("LockScreen.exe");
    let aside = dir.join(OLD_EXE);

    // A leftover from an earlier update would block the rename.
    let _ = std::fs::remove_file(&aside);
    std::fs::rename(&exe, &aside).map_err(|e| format!("{} couldn't be moved aside: {e}", exe.display()))?;

    match std::fs::copy(new, &exe) {
        Ok(_) => {
            let _ = std::fs::remove_file(new);
            log::audit("the program was replaced by an update");
            Ok(())
        }
        Err(e) => {
            // Put the running program back, or there would be none at all.
            let _ = std::fs::rename(&aside, &exe);
            Err(format!("{} couldn't be written: {e}", exe.display()))
        }
    }
}

/// Delete the renamed-aside copy left by a previous update.
///
/// Called at startup. It cannot be removed during the update itself: it is the
/// image the process is running from.
pub fn clean_up() {
    let Ok(dir) = paths::exe_dir() else { return };
    let aside = dir.join(OLD_EXE);
    if aside.exists() && std::fs::remove_file(&aside).is_ok() {
        log::info("removed the previous version left by an update");
    }
}

/// Open a release's page in the browser.
///
/// Used for a release that cannot be installed automatically, so somebody can
/// still get at it.
pub fn open_page(update: &Update) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let open = super::ui::wide("open");
    let url = super::ui::wide(&update.page);
    // SAFETY: both strings outlive the call; ShellExecuteW's result is a
    // pseudo-handle that needs no closing.
    unsafe {
        ShellExecuteW(None, PCWSTR(open.as_ptr()), PCWSTR(url.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds for the URL whichever forge is building, so a release workflow
    /// that set `LOCKSCREEN_RELEASES_API` to something malformed fails its own
    /// test step rather than shipping a copy that can never update.
    #[test]
    fn the_api_url_is_https_and_names_this_repository() {
        assert!(RELEASES_API.starts_with("https://"), "an update must not be fetched over plain HTTP");
        assert!(RELEASES_API.contains("/repos/"), "{RELEASES_API}");
        assert!(RELEASES_API.contains("/lock-screen/releases"), "{RELEASES_API}");
        // A page size, so a forge with hundreds of releases is not all
        // downloaded to find the newest. Forgejo and GitHub name it differently.
        assert!(RELEASES_API.contains("limit=") || RELEASES_API.contains("per_page="), "{RELEASES_API}");
    }

    /// A local build must read the Forgejo. If this fails, something in the
    /// environment set `LOCKSCREEN_RELEASES_API`, and every binary built from
    /// this machine would update from wherever that points.
    #[test]
    fn a_build_with_nothing_set_reads_the_forgejo() {
        if option_env!("LOCKSCREEN_RELEASES_API").is_none() {
            assert_eq!(RELEASES_API, DEFAULT_RELEASES_API);
            assert!(DEFAULT_RELEASES_API.starts_with("https://git.zacx.net/api/v1/repos/owner/lock-screen/"));
        }
    }

    #[test]
    fn the_aside_name_is_not_the_program_name() {
        // If these were ever equal, the rename would be a no-op and the install
        // would overwrite the running image -- which Windows refuses, leaving
        // no program at all.
        assert_ne!(OLD_EXE, "LockScreen.exe");
        assert!(OLD_EXE.ends_with(".exe"), "so Windows still treats it as a program it can delete");
    }

    #[test]
    fn cleaning_up_with_nothing_to_clean_is_harmless() {
        // Runs at every startup, including the overwhelming majority where no
        // update has ever happened.
        clean_up();
    }
}
