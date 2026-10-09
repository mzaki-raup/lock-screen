//! One lock screen at a time.
//!
//! The .NET version has no guard at all, so launching it twice stacks two
//! overlays, two sets of global hooks, and two processes each believing it owns
//! the Task Manager policy. Unlocking then clears the policy while the other
//! overlay is still up, and the second overlay has to be unlocked separately --
//! with the first one's hooks still swallowing keys underneath it.
//!
//! A named mutex rather than a lock file: the kernel releases it when the
//! process ends however it ends, including a kill, so a crashed lock screen
//! cannot leave a stale claim that stops the next one from starting. A lock
//! file would need its own staleness story, and "the lock screen refuses to
//! start because of a leftover file" is a bad failure for this program.
//!
//! The name is per-session (`Local\`), not global. Two users logged into the
//! same machine via fast user switching each get their own lock screen, which is
//! correct: they are locking different desktops.

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

/// Held for the lifetime of the process that owns the lock.
pub struct Instance {
    handle: HANDLE,
}

/// Why the claim failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Another process in this session already holds it.
    AlreadyRunning,
    /// The mutex could not be created. Reported separately because it means
    /// something is wrong with the session rather than that a lock screen is
    /// up, and the caller may reasonably carry on anyway.
    Unavailable,
}

impl Instance {
    /// Claim the right to be the lock screen in this session.
    ///
    /// # Errors
    ///
    /// [`Taken::AlreadyRunning`] when another instance holds it. The caller
    /// should exit quietly: the screen is already locked, which is what the
    /// user wanted, and saying "already running" to someone who just pressed a
    /// hotkey twice is noise.
    pub fn claim() -> Result<Self, Taken> {
        // `Local\` scopes the name to the logon session. The GUID is there so
        // the name cannot collide with another program's mutex; a readable
        // prefix is kept so it is recognisable in Process Explorer.
        //
        // SAFETY: a static wide name; no security attributes, so the mutex gets
        // the default descriptor for this session.
        let handle = unsafe {
            CreateMutexW(None, false, w!("Local\\LockScreen.Instance.{6f3a1c42-8b7d-4e55-9a10-2c8f0d6b4e91}"))
        };

        match handle {
            Ok(handle) => {
                // CreateMutexW succeeds and returns the *existing* handle when
                // the name is taken, signalling it only through GetLastError.
                // Checking the handle alone would let every instance think it
                // won.
                let already = unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS;
                if already {
                    // SAFETY: a handle we were just given and are done with.
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    Err(Taken::AlreadyRunning)
                } else {
                    Ok(Self { handle })
                }
            }
            Err(_) => Err(Taken::Unavailable),
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Closing the handle releases the kernel's last reference, so the name
        // becomes claimable again. Not `ReleaseMutex`: this process never
        // *waited* on the mutex, it only owns the name, and releasing
        // ownership it never took would fail.
        //
        // SAFETY: created in `claim`, closed exactly once here.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both properties in **one** test, deliberately.
    ///
    /// The mutex is session-wide, so two tests that each claim it cannot run in
    /// parallel -- and `cargo test` runs them in parallel by default. Split in
    /// two, they raced: one held the name while the other's claim came back
    /// `AlreadyRunning`, and it passed or failed depending on scheduling. That
    /// is worse than failing outright, because a suite that is green most of
    /// the time teaches you to re-run it.
    ///
    /// Note this does assume no real lock screen is running while the tests
    /// are, which is the same assumption as the ignored `policy` test.
    #[test]
    fn the_claim_is_exclusive_and_is_released_when_its_holder_drops() {
        let first = Instance::claim().expect("nothing else should hold it during tests");

        // Exclusive: a second lock screen must not be able to start.
        assert_eq!(Instance::claim().err(), Some(Taken::AlreadyRunning), "two lock screens must not coexist");

        // Released: a crashed or killed lock screen must not stop the next one
        // from starting. Dropping stands in for the process ending, which is
        // what the kernel does for us.
        drop(first);
        let again = Instance::claim();
        assert!(again.is_ok(), "the name should be free again: {:?}", again.err());
    }
}
