//! Compatibility with configs written by the .NET app.
//!
//! `tests/fixtures/dotnet-config.ini` was not hand-written. It was produced by
//! running the real .NET code path -- `INIFileParserDotNetCore` 2.5.2 writing
//! the same keys in the same order as `IniHelper.EnsureConfigExists()`, with a
//! `Password` value from `ProtectedData.Protect(.., null,
//! DataProtectionScope.CurrentUser)`, exactly as `EncryptionHelper` produces.
//!
//! Generating it rather than writing it caught two things an imitation would
//! have missed, and both would have broken silently:
//!
//! 1. the .NET writer emits `key = value`, **with spaces** around the `=`;
//! 2. the file starts with a **UTF-8 BOM**.
//!
//! Either one, unhandled, means every setting reads back as its default while
//! the app looks like it is working. That is why this test exists before the
//! code it guards.
//!
//! One caveat the test enforces rather than hides: the `Password` value is a
//! DPAPI blob bound to the user and machine that generated it, so it cannot be
//! decrypted here or in CI. The round-trip is tested with a freshly made blob
//! instead; the committed one is only ever checked for *shape*.

use std::path::PathBuf;

use lockscreen_core::config;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dotnet-config.ini")
}

fn fixture_text() -> String {
    std::fs::read_to_string(fixture_path()).expect("the committed .NET fixture should be readable")
}

#[test]
fn the_fixture_still_has_the_two_traits_that_make_it_worth_having() {
    let bytes = std::fs::read(fixture_path()).expect("fixture bytes");

    assert_eq!(&bytes[..3], b"\xef\xbb\xbf", "the fixture must keep its UTF-8 BOM; that is half the point of it");

    let text = String::from_utf8(bytes).expect("utf-8");
    assert!(text.contains("Title = Enter Password to Unlock"), "the fixture must keep the spaces around `=`");
    assert!(!text.contains("Title=Enter"), "if this fires, someone normalised the fixture and weakened the test");
}

#[test]
fn every_setting_is_read_back_from_a_real_dotnet_file() {
    let config = config::parse(&fixture_text()).expect("a .NET config should parse");

    // The values `IniHelper.EnsureConfigExists()` writes on first run. If BOM or
    // spacing handling regresses, these come back as defaults instead -- and
    // note that two of the expected values *are* the defaults, which is why the
    // title and timeout below are the load-bearing assertions.
    assert_eq!(config.title, "Enter Password to Unlock");
    assert_eq!(config.transparency, 0.5);
    assert!(!config.blur_background);

    assert!(!config.voice.enabled);
    assert_eq!(config.voice.unlock_command, "unlock");
    assert_eq!(config.voice.listening_timeout, 10);
}

#[test]
fn a_title_that_is_not_the_default_proves_the_value_was_actually_parsed() {
    // Guards against the failure mode where everything silently falls back to
    // defaults and the test above passes for the wrong reason.
    let text = fixture_text().replace("Title = Enter Password to Unlock", "Title = Zaki's machine = locked");
    let config = config::parse(&text).expect("parsed");
    assert_eq!(
        config.title, "Zaki's machine = locked",
        "a value containing `=` must survive splitting on the first one"
    );
}

#[test]
fn the_legacy_password_is_carried_as_opaque_text_and_never_as_a_credential() {
    let config = config::parse(&fixture_text()).expect("parsed");

    let blob = config.legacy_password.as_deref().expect("the .NET fixture has a Password value");
    assert!(blob.starts_with("AQAAANCMnd8BFdER"), "a DPAPI blob's header is stable; got {blob:.24}");
    assert_eq!(blob.len(), 308, "the fixture's blob length should not drift silently");

    // The important part: a .NET config alone never satisfies the app. There is
    // no Argon2 record in it, so setup still has to run and the overlay must
    // refuse to start.
    assert!(config.password_hash.is_none());
    assert!(config.master_hash.is_none());
    assert!(config.needs_setup(), "a .NET config must not be treated as set up");
}

#[test]
fn reading_a_dotnet_config_and_saving_it_drops_the_reversible_credential() {
    let dir = std::env::temp_dir().join(format!("lockscreen-migrate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let path = dir.join("config.ini");

    let mut config = config::parse(&fixture_text()).expect("parsed");
    let legacy = config.legacy_password.clone().expect("a legacy value");

    // What migration looks like: keep the appearance settings, set real
    // verifiers, let the DPAPI value fall away.
    config.password_hash = Some(lockscreen_core::credential::hash("a-real-password-now").expect("hashed"));
    config.master_hash = Some(lockscreen_core::credential::hash("a-master-password").expect("hashed"));
    config::save(&path, &config).expect("saved");

    let text = std::fs::read_to_string(&path).expect("read back");
    assert!(!text.contains(&legacy), "the DPAPI blob must not survive a save");

    let reloaded = config::load(&path).expect("reloaded");
    assert_eq!(reloaded.title, "Enter Password to Unlock", "settings migrate forward");
    assert_eq!(reloaded.transparency, 0.5);
    assert!(reloaded.legacy_password.is_none(), "nothing reversible should remain");
    assert!(!reloaded.needs_setup(), "after setup the overlay may start");

    let _ = std::fs::remove_dir_all(&dir);
}

/// DPAPI itself, round-tripped with a blob made here rather than the committed
/// one -- which is bound to the machine that generated it and would fail
/// anywhere else, including CI.
#[cfg(windows)]
#[test]
fn dpapi_round_trips_on_the_machine_that_made_the_blob() {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};

    let mut plaintext = b"1234".to_vec();
    let input = CRYPT_INTEGER_BLOB { cbData: plaintext.len() as u32, pbData: plaintext.as_mut_ptr() };
    let mut output = CRYPT_INTEGER_BLOB::default();

    let base64 = unsafe {
        CryptProtectData(&input, None, None, None, None, 0, &mut output).expect("DPAPI should encrypt");
        let blob = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        encode_base64(blob)
    };

    let recovered = lockscreen_core::credential::legacy::decrypt_dpapi(&base64).expect("our own blob should decrypt");
    assert_eq!(&recovered[..], b"1234");
}

/// The committed fixture's blob is expected *not* to decrypt anywhere but the
/// machine that wrote it. Asserting the shape of that failure keeps the
/// migration path honest about its one real limitation.
#[cfg(windows)]
#[test]
fn the_committed_blob_is_not_portable_and_fails_cleanly() {
    let config = config::parse(&fixture_text()).expect("parsed");
    let blob = config.legacy_password.as_deref().expect("a legacy value");

    // On the authoring machine this decrypts; anywhere else it returns None.
    // Either way it must not panic, and must never return something wrong.
    match lockscreen_core::credential::legacy::decrypt_dpapi(blob) {
        Some(plaintext) => assert_eq!(&plaintext[..], b"1234", "this machine wrote the fixture"),
        None => { /* the expected result on any other machine or user */ }
    }
}

#[cfg(windows)]
fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}
