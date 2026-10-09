//! The Task Manager policy, held for exactly as long as the overlay is up.
//!
//! The .NET app sets `DisableTaskMgr` in `DisableSystemKeys()` and clears it in
//! `EnableSystemKeys()`, which is called from three different places. Any exit
//! that misses all three -- an unhandled exception, a kill -- leaves Task
//! Manager disabled on the user's machine, with nothing in the UI to say why.
//!
//! Here the value is owned by a guard. Restoring it is `Drop`, so it happens on
//! every path out of the scope, including an unwind.
//!
//! Worth being clear about what this is: `HKCU` is writable by the very user it
//! restricts, so it is a speed bump, not a security control. It is not portable
//! either -- it touches the registry of whatever machine the app runs on,
//! whatever folder the app lives in.
//!
//! Uses `Win32_System_Registry` from the `windows` crate rather than the
//! `windows-registry` crate, matching `flex-taskbar`. That is not only
//! convention: `windows-registry 0.100` depends on `windows-result 0.100` while
//! `windows 0.62` carries `windows-result 0.62`, so its `Error` is a different
//! type and will not cross a `?`.

use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE_PTR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{Result, w};

const POLICY_PATH: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Policies\System");
const VALUE: windows::core::PCWSTR = w!("DisableTaskMgr");

/// Holds `DisableTaskMgr` while it lives and clears it on drop.
///
/// Create one when the overlay appears; let it fall out of scope on unlock.
pub struct TaskManagerPolicy {
    key: HKEY,
    /// Whether the value was already present when we arrived. If some other
    /// policy set it before us, leave it alone -- restoring means "put it back
    /// as it was", not "delete it".
    was_already_set: bool,
}

impl TaskManagerPolicy {
    /// Disable Task Manager, returning the guard that will re-enable it.
    ///
    /// # Errors
    ///
    /// Fails if the policy key cannot be created or written, which on a managed
    /// machine may mean group policy owns it. The caller should carry on and
    /// lock the screen anyway: a lock screen that refuses to appear because of
    /// a registry value is worse than one with Task Manager reachable.
    pub fn disable() -> Result<Self> {
        let mut key = HKEY::default();
        // SAFETY: the path is a static wide literal; `key` is a valid
        // out-pointer. The returned key is closed in `Drop`.
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                POLICY_PATH,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                None,
                &mut key,
                None,
            )
            .ok()?;
        }

        let guard = Self { key, was_already_set: value_exists(key) };

        let one = 1u32.to_ne_bytes();
        // SAFETY: `key` is open, the name is a static wide literal, and the
        // slice is a live 4-byte DWORD.
        let set = unsafe { RegSetValueExW(key, VALUE, None, REG_DWORD, Some(&one)) };
        if set != ERROR_SUCCESS {
            // `guard` drops here and closes the key, so the handle does not leak
            // on this path.
            return Err(set.to_hresult().into());
        }

        Ok(guard)
    }
}

/// Whether the value is present already, so that `Drop` knows not to remove
/// something it did not create.
fn value_exists(key: HKEY) -> bool {
    // SAFETY: `key` is open and every optional out-parameter is omitted, which
    // queries for existence only.
    unsafe { RegQueryValueExW(key, VALUE, None, None, None, None) == ERROR_SUCCESS }
}

impl Drop for TaskManagerPolicy {
    fn drop(&mut self) {
        if !self.was_already_set {
            // SAFETY: `self.key` is open until the `RegCloseKey` below.
            // Nothing useful to do with a failure, and panicking in `Drop`
            // during an unwind would abort the process.
            unsafe {
                let _ = RegDeleteValueW(self.key, VALUE);
            }
        }
        // SAFETY: closed exactly once, here, for a key opened in `disable`.
        unsafe {
            let _ = RegCloseKey(self.key);
        }
    }
}

// The handle is only ever used from the thread that owns the guard, but HKEY is
// a plain handle and is safe to move between threads.
unsafe impl Send for TaskManagerPolicy {}

/// Only here to keep the `HANDLE_PTR` import honest about why `HKEY` is `Send`:
/// it is a kernel handle, not a pointer into this process.
const _: () = {
    const fn assert_handle_sized() -> usize {
        core::mem::size_of::<HANDLE_PTR>()
    }
    let _ = assert_handle_sized();
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Takes and releases the real policy value, so it asserts the thing that
    /// actually matters: that nothing is left behind. Ignored by default
    /// because it writes to the running user's registry -- run it deliberately
    /// with `cargo test -- --ignored`.
    #[test]
    #[ignore = "writes to HKCU policy; run deliberately"]
    fn the_value_is_gone_once_the_guard_drops() {
        {
            let guard = TaskManagerPolicy::disable().expect("policy set");
            assert!(!guard.was_already_set, "a policy value is already set; not clobbering it");
            assert!(value_exists(guard.key), "the value should be set while the guard lives");
        }

        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                POLICY_PATH,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ,
                None,
                &mut key,
                None,
            )
            .ok()
            .expect("the key exists");
        }
        assert!(!value_exists(key), "the guard must leave no trace");
        unsafe {
            let _ = RegCloseKey(key);
        }
    }
}
