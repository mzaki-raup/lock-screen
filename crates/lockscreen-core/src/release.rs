//! Which published release, if any, is newer than the copy that is running.
//!
//! Pure, so all of it is testable off Windows: no network, no filesystem, no
//! windows. [`crate::update`]'s caller does the fetching, hashing and
//! installing. Adapted from `flex-taskbar/src/release.rs`, which reads the same
//! forge.
//!
//! # What the release workflow publishes
//!
//! ```text
//! LockScreen.exe     the program
//! SHA256SUMS.txt     checked before anything is replaced
//! ```
//!
//! A release without checksums is **never installed automatically** -- see
//! [`Update::installable`]. The workflow publishes both or neither.

use serde::Deserialize;

/// The program as published on a release, and the file of checksums beside it.
pub const EXE_ASSET: &str = "LockScreen.exe";
pub const SUMS_ASSET: &str = "SHA256SUMS.txt";

/// A release version, as a tag writes it (`v2.0.1`, or `2.0.1`).
///
/// Ordered, so the newest release is simply the largest. Three numbers and
/// nothing else: a tag carrying `-rc1` or `+build` is **not** a release this
/// understands, and is passed over rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let mut parts = s.split('.');
        let mut next = || parts.next().filter(|p| !p.is_empty())?.parse::<u32>().ok();
        let (major, minor, patch) = (next()?, next()?, next()?);
        parts.next().is_none().then_some(Version { major, minor, patch })
    }

    /// The version this copy was built as, from `Cargo.toml`.
    pub fn running() -> Version {
        Version::parse(crate::VERSION).unwrap_or_default()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// How far the search for a newer release has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    /// Nothing has been looked for in this run.
    #[default]
    Unknown,
    Checking,
    /// Checked, and this copy is the newest there is.
    UpToDate,
    Available(Version),
    /// The last check could not reach the forge.
    Failed,
}

impl State {
    /// What to show beside the version. Empty before anything has been looked
    /// for: an unasked-for "unknown" tells nobody anything.
    pub fn note(self) -> String {
        match self {
            State::Unknown => String::new(),
            State::Checking => "checking\u{2026}".to_owned(),
            State::UpToDate => "up to date".to_owned(),
            State::Available(v) => format!("{v} is available"),
            State::Failed => "couldn't check for updates".to_owned(),
        }
    }

    /// What the button beside it offers to do.
    pub fn action(self) -> &'static str {
        match self {
            State::Checking => "Checking\u{2026}",
            State::Available(_) => "Install update",
            _ => "Check for updates",
        }
    }

    /// Whether pressing it installs rather than looks.
    pub fn offers_update(self) -> bool {
        matches!(self, State::Available(_))
    }

    /// Whether the button does anything. Only while a check is already running
    /// does it not.
    pub fn enabled(self) -> bool {
        self != State::Checking
    }
}

/// The line settings mode shows: the running version, and how the search went.
pub fn version_line(running: Version, state: State) -> String {
    let note = state.note();
    if note.is_empty() { format!("LockScreen {running}") } else { format!("LockScreen {running} \u{2014} {note}") }
}

/// A file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A release worth offering: newer than what is running, with the program on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: Version,
    pub tag: String,
    /// The release's page, for whoever would rather read the notes first.
    pub page: String,
    pub notes: String,
    pub exe: Asset,
    /// The checksums beside it.
    pub sums: Option<Asset>,
}

impl Update {
    /// Whether this release may be installed by the app itself.
    ///
    /// Without checksums, no: the only check available would be "the forge said
    /// so", and a truncated download would be written over the running
    /// program. Such a release is offered as a link to its page instead.
    pub fn installable(&self) -> bool {
        self.sums.is_some()
    }
}

/// What the forge's releases API answers with.
///
/// Unknown fields are ignored, so this reads both Forgejo/Gitea and GitHub.
#[derive(Deserialize)]
struct ApiRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

impl ApiRelease {
    fn asset(&self, name: &str) -> Option<Asset> {
        self.assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name) && !a.browser_download_url.is_empty())
            .map(|a| Asset { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size })
    }
}

/// The newest release in the API's answer that is newer than `current`.
///
/// Drafts, pre-releases, tags that are not a plain version, and releases with no
/// program attached are all passed over. **A version equal to or older than
/// `current` is refused**, which is the downgrade protection: an attacker who
/// can serve this JSON must not be able to walk an install back to a known-bad
/// build.
pub fn newer_than(json: &str, current: Version) -> Option<Update> {
    let releases: Vec<ApiRelease> = serde_json::from_str(json).ok()?;
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let version = Version::parse(&r.tag_name)?;
            if version <= current {
                return None;
            }
            Some(Update {
                version,
                tag: r.tag_name.clone(),
                page: r.html_url.clone(),
                notes: r.body.trim().to_owned(),
                exe: r.asset(EXE_ASSET)?,
                sums: r.asset(SUMS_ASSET),
            })
        })
        .max_by_key(|u| u.version)
}

/// The checksum of `name` in a `SHA256SUMS.txt`, lower-cased.
///
/// Lines are `<hex>  <file name>`, as `sha256sum` writes them.
pub fn sha256_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        // `sha256sum` marks a binary-mode file with a `*`.
        let file = file.trim().trim_start_matches('*');
        let hash = hash.trim();
        let ok = file.eq_ignore_ascii_case(name) && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32, patch: u32) -> Version {
        Version { major, minor, patch }
    }

    fn release(tag: &str, with_sums: bool) -> String {
        let sums = if with_sums {
            r#", {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/sums", "size": 120}"#
        } else {
            ""
        };
        format!(
            r#"{{"tag_name": "{tag}", "draft": false, "prerelease": false,
               "html_url": "https://x/{tag}", "body": " notes ",
               "assets": [{{"name": "LockScreen.exe", "browser_download_url": "https://x/exe", "size": 580000}}{sums}]}}"#
        )
    }

    /// GitHub's answer, with the fields it adds that Forgejo does not and the
    /// `per_page` paging that the GitHub build asks for. The parser must read
    /// it unchanged: the GitHub release workflow builds the very same source,
    /// and the only difference between the two copies is the URL baked in.
    ///
    /// Shaped from the documented GitHub REST response rather than captured
    /// from the mirror, which did not exist when this was written. PARITY.md
    /// carries the item to replace it with a verbatim fixture once a release
    /// has been published there, as `forgejo_release_compat.rs` does.
    #[test]
    fn a_github_shaped_answer_reads_the_same_as_a_forgejo_one() {
        let json = r#"[{
            "url": "https://api.github.com/repos/u/lock-screen/releases/1",
            "assets_url": "https://api.github.com/repos/u/lock-screen/releases/1/assets",
            "upload_url": "https://uploads.github.com/repos/u/lock-screen/releases/1/assets{?name,label}",
            "html_url": "https://github.com/u/lock-screen/releases/tag/v9.0.0",
            "id": 1, "node_id": "RE_x", "tag_name": "v9.0.0", "target_commitish": "main",
            "name": "LockScreen 9.0.0", "draft": false, "prerelease": false,
            "created_at": "2026-10-11T00:00:00Z", "published_at": "2026-10-11T00:00:00Z",
            "author": {"login": "u", "id": 1, "type": "User"},
            "assets": [
              {"url": "https://api.github.com/repos/u/lock-screen/releases/assets/1", "id": 1,
               "name": "LockScreen.exe", "label": "", "content_type": "application/octet-stream",
               "state": "uploaded", "size": 580000, "download_count": 0,
               "browser_download_url": "https://github.com/u/lock-screen/releases/download/v9.0.0/LockScreen.exe"},
              {"url": "https://api.github.com/repos/u/lock-screen/releases/assets/2", "id": 2,
               "name": "SHA256SUMS.txt", "label": "", "content_type": "text/plain",
               "state": "uploaded", "size": 200, "download_count": 0,
               "browser_download_url": "https://github.com/u/lock-screen/releases/download/v9.0.0/SHA256SUMS.txt"}
            ],
            "tarball_url": "https://api.github.com/repos/u/lock-screen/tarball/v9.0.0",
            "zipball_url": "https://api.github.com/repos/u/lock-screen/zipball/v9.0.0",
            "body": "- a change"
        }]"#;
        let update = newer_than(json, v(0, 1, 0)).expect("a GitHub release should be offered");
        assert_eq!(update.version, v(9, 0, 0));
        assert_eq!(update.page, "https://github.com/u/lock-screen/releases/tag/v9.0.0");
        assert_eq!(update.exe.url, "https://github.com/u/lock-screen/releases/download/v9.0.0/LockScreen.exe");
        assert!(update.installable(), "both assets are there, so it must be installable");
    }

    #[test]
    fn versions_parse_with_or_without_the_tag_prefix() {
        assert_eq!(Version::parse("v2.0.1"), Some(v(2, 0, 1)));
        assert_eq!(Version::parse("2.0.1"), Some(v(2, 0, 1)));
        assert_eq!(Version::parse("  V0.1.0 "), Some(v(0, 1, 0)));
    }

    #[test]
    fn a_tag_that_is_not_three_plain_numbers_is_refused_rather_than_guessed_at() {
        for tag in ["v2.0", "2.0.1.4", "v2.0.1-rc1", "2.0.1+build", "latest", "", "v", "v2.a.1", "v-1.0.0"] {
            assert_eq!(Version::parse(tag), None, "{tag:?} should not parse");
        }
    }

    #[test]
    fn versions_order_by_each_component_in_turn() {
        assert!(v(2, 0, 0) > v(1, 9, 9));
        assert!(v(0, 2, 0) > v(0, 1, 99));
        assert!(v(0, 1, 2) > v(0, 1, 1));
        // The port's own path: 0.1.0 today, 2.0.0 at the release that replaces
        // the .NET app, which shipped as 1.1.1.
        assert!(v(2, 0, 0) > v(1, 1, 1));
        assert!(v(1, 1, 1) > v(0, 1, 0));
    }

    #[test]
    fn the_newest_newer_release_is_chosen_whatever_order_the_api_returns_them() {
        let json = format!("[{}, {}, {}]", release("v0.2.0", true), release("v0.4.0", true), release("v0.3.0", true));
        let found = newer_than(&json, v(0, 1, 0)).expect("something newer");
        assert_eq!(found.version, v(0, 4, 0));
        assert_eq!(found.notes, "notes", "the body is trimmed");
        assert_eq!(found.exe.name, "LockScreen.exe");
        assert!(found.installable());
    }

    #[test]
    fn a_release_that_is_not_newer_is_refused_which_is_the_downgrade_protection() {
        let json = format!("[{}, {}]", release("v0.1.0", true), release("v0.0.9", true));
        // Equal is not newer, and older is certainly not: an attacker who can
        // serve this JSON must not be able to walk an install backwards.
        assert_eq!(newer_than(&json, v(0, 1, 0)), None);
        assert_eq!(newer_than(&json, v(9, 9, 9)), None);
    }

    #[test]
    fn drafts_and_prereleases_are_passed_over() {
        let draft = release("v9.0.0", true).replace(r#""draft": false"#, r#""draft": true"#);
        let pre = release("v8.0.0", true).replace(r#""prerelease": false"#, r#""prerelease": true"#);
        let json = format!("[{draft}, {pre}, {}]", release("v0.2.0", true));
        assert_eq!(newer_than(&json, v(0, 1, 0)).expect("one").version, v(0, 2, 0));
    }

    #[test]
    fn a_release_with_no_program_attached_is_not_an_update() {
        let json = r#"[{"tag_name": "v9.0.0", "draft": false, "prerelease": false, "assets": []}]"#;
        assert_eq!(newer_than(json, v(0, 1, 0)), None);
    }

    #[test]
    fn a_release_without_checksums_is_found_but_not_installable() {
        let json = format!("[{}]", release("v0.2.0", false));
        let found = newer_than(&json, v(0, 1, 0)).expect("found");
        assert!(!found.installable(), "without checksums the app must not replace itself");
        assert_eq!(found.page, "https://x/v0.2.0", "so the page is offered instead");
    }

    #[test]
    fn rubbish_from_the_forge_is_no_update_rather_than_a_panic() {
        for json in ["", "null", "{}", "[", r#"{"not": "an array"}"#, "[1, 2, 3]"] {
            assert_eq!(newer_than(json, v(0, 1, 0)), None, "{json:?}");
        }
    }

    #[test]
    fn a_repository_with_no_releases_yet_reads_as_up_to_date() {
        // Not a hypothetical: this is what git.zacx.net answers today, verified
        // against the real API. An empty array must mean "nothing newer", not a
        // failure -- a failed check and being up to date are different things
        // to report, and conflating them would say the forge was unreachable
        // every time nothing had been released.
        assert_eq!(newer_than("[]", v(0, 1, 0)), None);
    }

    #[test]
    fn checksums_are_read_from_the_sha256sum_layout() {
        let sums = "\
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  SHA256SUMS.txt
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  LockScreen.exe
";
        assert_eq!(
            sha256_of(sums, "LockScreen.exe").as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(sha256_of(sums, "not-published.exe"), None);
    }

    #[test]
    fn a_binary_mode_marker_and_odd_casing_are_tolerated() {
        let sums = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD *lockscreen.EXE\n";
        assert_eq!(
            sha256_of(sums, "LockScreen.exe").as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            "the hash is lower-cased and the name matched case-insensitively"
        );
    }

    #[test]
    fn a_malformed_checksum_line_is_not_accepted() {
        // Too short, too long, not hex, and no name: each would otherwise be
        // compared against a real digest and could never match -- but failing
        // here says why, instead of reporting a mismatch.
        for sums in [
            "abc  LockScreen.exe\n",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015adXX  LockScreen.exe\n",
            "zzz816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  LockScreen.exe\n",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n",
            "",
        ] {
            assert_eq!(sha256_of(sums, "LockScreen.exe"), None, "{sums:?}");
        }
    }

    #[test]
    fn the_version_line_reads_well_in_every_state() {
        let running = v(0, 1, 0);
        assert_eq!(version_line(running, State::Unknown), "LockScreen 0.1.0");
        assert_eq!(version_line(running, State::UpToDate), "LockScreen 0.1.0 \u{2014} up to date");
        assert_eq!(version_line(running, State::Available(v(0, 2, 0))), "LockScreen 0.1.0 \u{2014} 0.2.0 is available");
    }

    #[test]
    fn a_failed_check_is_never_mistaken_for_being_up_to_date() {
        assert_ne!(State::Failed.note(), State::UpToDate.note());
        assert!(!State::Failed.offers_update());
        assert!(State::Failed.enabled(), "and it can be tried again");
    }

    #[test]
    fn only_an_available_update_offers_to_install() {
        assert!(State::Available(v(1, 0, 0)).offers_update());
        for other in [State::Unknown, State::UpToDate, State::Failed, State::Checking] {
            assert!(!other.offers_update(), "{other:?}");
        }
        assert!(!State::Checking.enabled(), "and a check in flight cannot be started twice");
    }

    #[test]
    fn the_running_version_matches_the_crate_version() {
        assert_eq!(Version::running().to_string(), crate::VERSION, "a release tag is checked against this");
    }
}
