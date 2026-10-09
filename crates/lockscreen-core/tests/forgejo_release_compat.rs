//! The updater, against a real answer from the forge it reads.
//!
//! `tests/fixtures/forgejo-releases.json` was not written by hand. It is the
//! verbatim response from
//! `https://git.zacx.net/api/v1/repos/owner/lock-screen/releases?limit=10`
//! after the first release was published by `.github/workflows/release.yml` --
//! the same URL, with the same query, that `win::update` fetches at runtime.
//!
//! It exists because the unit tests in `release.rs` feed the parser JSON the
//! parser's author wrote. That proves the logic and proves nothing about the
//! forge: a field named `download_url` instead of `browser_download_url` would
//! pass every one of them and then find no asset on a real release, reporting
//! "no update" for ever while an update sat there. The fields this depends on
//! are Forgejo's, not ours, and this is the only test that reads them as they
//! actually arrive.
//!
//! Kept verbatim, including the fields the parser ignores. Trimming it to what
//! is used today would throw away the thing being tested -- that a response
//! carrying more than we ask for still reads correctly.

use std::path::PathBuf;

use lockscreen_core::release::{self, EXE_ASSET, SUMS_ASSET, Version};

fn fixture() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forgejo-releases.json");
    std::fs::read_to_string(path).expect("the committed forge response should be readable")
}

fn v(major: u32, minor: u32, patch: u32) -> Version {
    Version { major, minor, patch }
}

#[test]
fn the_fixture_still_carries_the_field_names_that_make_it_worth_having() {
    let json = fixture();
    // If somebody regenerates this from a different forge, or hand-edits it,
    // these are the names the parser depends on. Named here so the reason the
    // file exists is not lost.
    for field in ["tag_name", "browser_download_url", "prerelease", "draft", "html_url", "assets"] {
        assert!(json.contains(field), "the fixture must keep Forgejo's `{field}` field");
    }
    assert!(json.contains(EXE_ASSET), "the release must carry {EXE_ASSET}");
    assert!(json.contains(SUMS_ASSET), "and {SUMS_ASSET}, or it would not be installable");
}

#[test]
fn an_older_copy_is_offered_the_published_release() {
    // The case that matters: a real release, read by the real parser.
    let update = release::newer_than(&fixture(), v(0, 0, 9)).expect("0.0.9 should be offered the published release");

    assert_eq!(update.version, v(0, 1, 0));
    assert_eq!(update.tag, "v0.1.0");
    assert!(update.page.starts_with("https://"), "{}", update.page);

    assert_eq!(update.exe.name, EXE_ASSET);
    assert!(
        update.exe.url.starts_with("https://"),
        "an update must not be fetched over plain HTTP: {}",
        update.exe.url
    );
    assert!(update.exe.size > 100_000, "the program is not that small: {}", update.exe.size);

    let sums = update.sums.as_ref().expect("the release publishes checksums");
    assert_eq!(sums.name, SUMS_ASSET);
    assert!(sums.url.starts_with("https://"), "{}", sums.url);

    // Both halves present, so the app may install it rather than only pointing
    // at the page.
    assert!(update.installable(), "a release with checksums is installable");
}

#[test]
fn the_published_version_itself_is_not_offered_an_update() {
    // "Up to date" and "the check failed" are different answers, and this is
    // the first: a well-formed response with nothing newer in it.
    assert_eq!(
        release::newer_than(&fixture(), v(0, 1, 0)),
        None,
        "a copy on the newest release has nothing to install"
    );
}

#[test]
fn a_newer_copy_is_never_walked_backwards() {
    // The downgrade protection, against a real response. Somebody running a
    // build newer than anything published -- a developer, or someone who was
    // given a fix early -- must not be offered the older release.
    assert_eq!(release::newer_than(&fixture(), v(0, 1, 1)), None, "0.1.1 must not be offered 0.1.0");
    assert_eq!(release::newer_than(&fixture(), v(1, 0, 0)), None);
    assert_eq!(release::newer_than(&fixture(), v(99, 0, 0)), None);
}

#[test]
fn the_notes_survive_the_round_trip() {
    // Shown in the settings window before anybody installs anything, so an
    // empty string here would be a blank panel.
    let update = release::newer_than(&fixture(), v(0, 0, 1)).expect("offered");
    assert!(!update.notes.trim().is_empty(), "the release notes should not come through empty");
}
