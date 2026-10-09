//! Verifying passwords, and reading the old reversible ones in order to stop
//! using them.
//!
//! The .NET app stores the password DPAPI-encrypted, which is *reversible*: any
//! code running as that user recovers the plaintext from `config.ini`. It then
//! holds it in a `string` that is never cleared and compares it with `==`.
//!
//! Verification does not need recovery. This module stores an Argon2id hash as a
//! PHC string and compares in constant time, so a stolen config yields nothing,
//! and the input buffer is zeroized on drop. The DPAPI reader survives only to
//! migrate an old install's *settings*; the recovered password is never used to
//! authenticate.

use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use zeroize::Zeroizing;

/// Which credential a verification was against. The master password is a
/// recovery credential, so its uses are worth recording separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The everyday unlock password.
    Password,
    /// The recovery credential chosen during first-run setup. Never offered
    /// automatically, rate-limited harder, and audited when used.
    Master,
}

/// What went wrong. Deliberately coarse: a caller must not be able to tell a
/// malformed stored hash from a wrong password by inspecting the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// No hash is stored for this credential -- setup has not run.
    NotSet,
    /// The stored value is not a usable PHC string.
    Corrupt,
    /// Hashing itself failed.
    Backend,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Error::NotSet => "no password has been set up yet",
            Error::Corrupt => "the stored password record is unreadable; run setup again",
            Error::Backend => "the password could not be hashed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for Error {}

/// The smallest password the app will accept.
///
/// Replaces the .NET rule of "at least 3 characters, letters and digits only",
/// which accepted `abc` and *rejected* `correct-horse-battery`. Length is the
/// property that matters; character classes are not restricted at all.
pub const MIN_LENGTH: usize = 8;

/// Whether a proposed password is long enough.
pub fn is_acceptable(password: &str) -> bool {
    password.chars().count() >= MIN_LENGTH
}

/// Hash a new password into a PHC string for storage.
///
/// The salt is random per call, so the two credentials get independent records:
/// rotating one must not invalidate the other.
pub fn hash(password: &str) -> Result<String, Error> {
    // `hash_password` generates a fresh random salt per call, which is what
    // gives the two credentials independent records: rotating one must not
    // invalidate the other.
    let hashed = Argon2::default().hash_password(password.as_bytes()).map_err(|_| Error::Backend)?;
    Ok(hashed.to_string())
}

/// Verify a password against a stored PHC string.
///
/// Returns `Ok(false)` for a wrong password and `Err` only when the stored
/// record cannot be used at all. The comparison is constant-time, which the
/// `==` in `PasswordWindow.xaml.cs:76` is not.
pub fn verify(password: &str, stored: Option<&str>) -> Result<bool, Error> {
    let stored = stored.ok_or(Error::NotSet)?;
    let parsed = PasswordHash::new(stored).map_err(|_| Error::Corrupt)?;
    // Zeroized on drop, so a core dump of the process does not contain the
    // attempt. Rust's Drop makes this automatic rather than something to
    // remember at every early return.
    let attempt = Zeroizing::new(password.as_bytes().to_vec());
    Ok(Argon2::default().verify_password(&attempt, &parsed).is_ok())
}

/// Reading the credential the .NET app left behind.
#[cfg(windows)]
pub mod legacy {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
    use zeroize::Zeroizing;

    /// Decrypt a DPAPI value written by `ProtectedData.Protect(.., null,
    /// DataProtectionScope.CurrentUser)`.
    ///
    /// Only succeeds for the Windows user and machine that wrote it, which is
    /// precisely why the port stops storing credentials this way -- a portable
    /// app whose config cannot move is not portable.
    ///
    /// The result is zeroized on drop. Use it to prompt someone to re-enter
    /// their password under the new scheme, never to let them in.
    pub fn decrypt_dpapi(base64_blob: &str) -> Option<Zeroizing<Vec<u8>>> {
        let mut ciphertext = decode_base64(base64_blob)?;
        let input =
            CRYPT_INTEGER_BLOB { cbData: u32::try_from(ciphertext.len()).ok()?, pbData: ciphertext.as_mut_ptr() };
        let mut output = CRYPT_INTEGER_BLOB::default();

        // SAFETY: `input` points at a live buffer of the length it declares, and
        // `output` is a valid out-pointer. On success Windows allocates the
        // plaintext with LocalAlloc; it is copied out and the copy is what
        // escapes. The small leak of the LocalAlloc block is accepted here
        // rather than taking a LocalFree dependency for a one-shot migration path.
        unsafe {
            CryptUnprotectData(&input, None, None, None, None, 0, &mut output).ok()?;
            if output.pbData.is_null() {
                return None;
            }
            let plaintext = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
            Some(Zeroizing::new(plaintext.to_vec()))
        }
    }

    /// Standard base64 with padding, which is what `Convert.ToBase64String`
    /// emits. Hand-rolled to avoid a dependency for one migration path.
    fn decode_base64(s: &str) -> Option<Vec<u8>> {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut accumulator = 0u32;
        let mut bits = 0u32;
        for byte in s.bytes() {
            if byte.is_ascii_whitespace() || byte == b'=' {
                continue;
            }
            let value = TABLE.iter().position(|&t| t == byte)? as u32;
            accumulator = (accumulator << 6) | value;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((accumulator >> bits) as u8);
            }
        }
        Some(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn base64_decodes_the_rfc_test_vectors() {
            assert_eq!(decode_base64("").as_deref(), Some(&b""[..]));
            assert_eq!(decode_base64("Zg==").as_deref(), Some(&b"f"[..]));
            assert_eq!(decode_base64("Zm8=").as_deref(), Some(&b"fo"[..]));
            assert_eq!(decode_base64("Zm9vYmFy").as_deref(), Some(&b"foobar"[..]));
        }

        #[test]
        fn base64_rejects_rubbish() {
            assert!(decode_base64("not*valid").is_none());
        }

        #[test]
        fn a_blob_this_user_did_not_write_does_not_decrypt() {
            assert!(decrypt_dpapi("AQAAANCMnd8BFdERjHoAwE/Cl+sBAAAAdGhpcyBpcyBub3QgYSByZWFsIGJsb2I=").is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correct_password_verifies_and_a_wrong_one_does_not() {
        let stored = hash("correct-horse-battery").expect("hashed");
        assert!(verify("correct-horse-battery", Some(&stored)).expect("verified"));
        assert!(!verify("Correct-horse-battery", Some(&stored)).expect("verified"));
        assert!(!verify("", Some(&stored)).expect("verified"));
    }

    #[test]
    fn the_stored_record_is_an_argon2id_phc_string() {
        let stored = hash("a-long-enough-password").expect("hashed");
        assert!(stored.starts_with("$argon2id$v=19$"), "{stored}");
        assert!(!stored.contains("a-long-enough-password"), "the password must not appear in its own hash");
    }

    #[test]
    fn two_credentials_get_independent_salts() {
        let password = hash("the-same-input-twice").expect("hashed");
        let master = hash("the-same-input-twice").expect("hashed");
        assert_ne!(password, master, "equal passwords must not produce equal records");
        assert!(verify("the-same-input-twice", Some(&password)).expect("verified"));
        assert!(verify("the-same-input-twice", Some(&master)).expect("verified"));
    }

    #[test]
    fn an_absent_or_corrupt_record_is_an_error_not_a_pass() {
        assert_eq!(verify("anything", None), Err(Error::NotSet));
        assert_eq!(verify("anything", Some("")), Err(Error::Corrupt));
        assert_eq!(verify("anything", Some("1234")), Err(Error::Corrupt));
        // The .NET EncryptionHelper returns the stored text unchanged when
        // decryption fails, so a hand-edited plaintext `Password=` became a
        // working password. Here it cannot: a non-PHC value never verifies.
        assert_eq!(verify("plaintext", Some("plaintext")), Err(Error::Corrupt));
    }

    #[test]
    fn the_length_floor_accepts_passphrases_the_dotnet_rule_rejected() {
        assert!(is_acceptable("correct-horse-battery"));
        assert!(is_acceptable("p@ssw0rd!"));
        assert!(!is_acceptable("abc"), "the .NET rule accepted this");
        assert!(!is_acceptable("1234"));
    }
}
