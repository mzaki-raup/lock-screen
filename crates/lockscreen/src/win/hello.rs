//! Windows Hello, through the interface an unpackaged Win32 process is meant
//! to use.
//!
//! This is Finding B of the migration study. `PasswordWindow.xaml.cs:102` calls
//! the static `UserConsentVerifier.RequestVerificationAsync`, which is
//! specified for apps **with package identity**. LockScreenApp has none, and
//! for unpackaged processes Windows wants the COM interop interface instead --
//! it takes an `HWND` so the consent dialog can be parented and activated
//! against a real window. Without that, the prompt can appear behind an
//! always-on-top overlay, which is the worst possible place for it.
//!
//! The call is synchronous (`join()` blocks), so it must not run on the UI
//! thread: the consent dialog needs our message loop to keep pumping. It is
//! therefore run on a worker thread, and the answer comes back to the dialog as
//! a posted message. Same shape the sibling's updater uses for WinHTTP.

use std::sync::mpsc;

use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows::core::{HSTRING, factory};
use windows_future::IAsyncOperation;

/// What Hello had to say, flattened to what the dialog actually branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The user proved who they are. Unlock.
    Verified,
    /// They dismissed it, or ran out of attempts. Offer the password, and a
    /// button to try Hello again.
    Declined,
    /// The device is busy -- worth retrying, not worth falling back yet.
    Busy,
    /// Hello cannot be used on this machine or by this user. Go straight to the
    /// password and do not offer a retry.
    Unavailable,
    /// The call itself failed. Treated as `Unavailable`, but carried separately
    /// so it can be logged rather than silently swallowed -- which is what
    /// `catch (Exception ex)` at `PasswordWindow.xaml.cs:126` does today.
    Failed(windows::core::HRESULT),
}

impl Outcome {
    /// Whether offering "Retry Windows Hello" makes sense.
    pub fn is_retryable(self) -> bool {
        matches!(self, Outcome::Declined | Outcome::Busy)
    }

    /// The line to show under the prompt.
    pub fn message(self) -> &'static str {
        match self {
            Outcome::Verified => "Verified.",
            Outcome::Declined => "Windows Hello was cancelled. Try again, or use your password.",
            Outcome::Busy => "The Windows Hello device is busy. Try again.",
            Outcome::Unavailable => "Windows Hello isn't available. Use your password.",
            Outcome::Failed(_) => "Windows Hello could not be used. Use your password.",
        }
    }
}

impl From<UserConsentVerificationResult> for Outcome {
    fn from(result: UserConsentVerificationResult) -> Self {
        match result {
            UserConsentVerificationResult::Verified => Outcome::Verified,
            UserConsentVerificationResult::Canceled | UserConsentVerificationResult::RetriesExhausted => {
                Outcome::Declined
            }
            UserConsentVerificationResult::DeviceBusy => Outcome::Busy,
            // DeviceNotPresent, NotConfiguredForUser, DisabledByPolicy, and
            // anything added later.
            _ => Outcome::Unavailable,
        }
    }
}

/// Whether Hello is set up for this user, before showing a prompt that offers
/// it. Cheap, and blocks only briefly.
pub fn is_available() -> bool {
    UserConsentVerifier::CheckAvailabilityAsync()
        .and_then(|op| op.join())
        .map(|a| a == UserConsentVerifierAvailability::Available)
        .unwrap_or(false)
}

/// Ask Hello to verify the user, parented to `hwnd`.
///
/// Blocks until the user answers, so call it from [`request_on_worker`] rather
/// than from the thread running the message loop.
fn request_blocking(hwnd: HWND, message: &str) -> Outcome {
    let interop = match factory::<UserConsentVerifier, IUserConsentVerifierInterop>() {
        Ok(interop) => interop,
        Err(e) => return Outcome::Failed(e.code()),
    };

    // SAFETY: `hwnd` is a live top-level window owned by the calling process,
    // and the message string outlives the call. The returned async operation is
    // the documented one for this interface's IID.
    let operation: windows::core::Result<IAsyncOperation<UserConsentVerificationResult>> =
        unsafe { interop.RequestVerificationForWindowAsync(hwnd, &HSTRING::from(message)) };

    match operation.and_then(|op| op.join()) {
        Ok(result) => result.into(),
        Err(e) => Outcome::Failed(e.code()),
    }
}

/// Run a verification on a worker thread and get the answer back on a channel.
///
/// The caller keeps pumping messages while this is in flight -- the consent
/// dialog is a window too, and starving the message loop would hang it.
pub fn request_on_worker(hwnd: HWND, message: &str) -> mpsc::Receiver<Outcome> {
    let (tx, rx) = mpsc::channel();
    let message = message.to_owned();
    // HWND is a kernel handle, not a pointer into our address space, so moving
    // the raw value to another thread is sound. Carried as an isize because
    // HWND itself is not Send.
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        let outcome = request_blocking(HWND(raw as *mut _), &message);
        // A send failure just means the dialog closed first; nothing to do.
        let _ = tx.send(outcome);
        // Then wake the loop, or the answer sits in the channel until some
        // unrelated message happens to arrive. See `win::wake_message_loop`:
        // this is the fix for the "Hello verified, screen stayed locked for a
        // few seconds" defect.
        super::wake_message_loop(raw);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outcome_has_a_message_fit_to_show_at_a_locked_screen() {
        for outcome in [
            Outcome::Verified,
            Outcome::Declined,
            Outcome::Busy,
            Outcome::Unavailable,
            Outcome::Failed(windows::core::HRESULT(-1)),
        ] {
            let message = outcome.message();
            assert!(message.ends_with('.'), "{outcome:?}: {message}");
            // Same guard as the other windows: a flattened line continuation
            // reached a user once, in the voice module.
            assert!(!message.contains("  "), "{outcome:?} has a run of spaces: {message:?}");
            assert_eq!(message.lines().count(), 1, "{outcome:?} spans lines: {message:?}");
        }
        // The one case that must not suggest retrying, since retrying cannot
        // help, is also the one that must not mention an HRESULT at somebody
        // standing at a locked machine.
        assert!(!Outcome::Failed(windows::core::HRESULT(-1)).message().contains("0x"));
    }

    #[test]
    fn every_verification_result_maps_to_an_outcome() {
        assert_eq!(Outcome::from(UserConsentVerificationResult::Verified), Outcome::Verified);
        assert_eq!(Outcome::from(UserConsentVerificationResult::Canceled), Outcome::Declined);
        assert_eq!(Outcome::from(UserConsentVerificationResult::RetriesExhausted), Outcome::Declined);
        assert_eq!(Outcome::from(UserConsentVerificationResult::DeviceBusy), Outcome::Busy);
        assert_eq!(Outcome::from(UserConsentVerificationResult::DeviceNotPresent), Outcome::Unavailable);
        assert_eq!(Outcome::from(UserConsentVerificationResult::NotConfiguredForUser), Outcome::Unavailable);
        assert_eq!(Outcome::from(UserConsentVerificationResult::DisabledByPolicy), Outcome::Unavailable);
        // An unknown future value must land on Unavailable, not Verified.
        assert_eq!(Outcome::from(UserConsentVerificationResult(99)), Outcome::Unavailable);
    }

    #[test]
    fn only_a_dismissal_or_a_busy_device_is_worth_retrying() {
        assert!(Outcome::Declined.is_retryable());
        assert!(Outcome::Busy.is_retryable());
        assert!(!Outcome::Unavailable.is_retryable());
        assert!(!Outcome::Verified.is_retryable());
        assert!(!Outcome::Failed(windows::core::HRESULT(-1)).is_retryable());
    }

    #[test]
    fn availability_can_be_queried_without_showing_a_prompt() {
        // Whatever this machine answers, the call must not panic or block
        // indefinitely. The migration study's probe recorded `Available` here.
        let _ = is_available();
    }
}
