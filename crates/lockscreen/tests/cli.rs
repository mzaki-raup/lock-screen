//! End-to-end checks against the real executable, as phase 08 of the migration
//! study asks for.
//!
//! Everything else in this workspace tests a function. These start the binary
//! Cargo just built, from a directory of their own, and look at what it does --
//! which is the only way to check the two things that are properties of the
//! *program* rather than of any function in it:
//!
//! - **portability (D6).** The config has to be the one beside the executable,
//!   not the one in the working directory. These tests put a *different*
//!   `config.ini` in the working directory to make sure the right one wins.
//!   `std::env::current_dir()` would pass every unit test and still be wrong.
//! - **the D7 guard.** `--setup` must refuse a copy that already has
//!   credentials. `main.rs` tests that the message says the right thing; this
//!   tests that the program actually reaches it.
//!
//! # Why none of these locks the screen
//!
//! The overlay is what this program is for, and running it from a test suite
//! would cover the developer's screen, install a system-wide keyboard hook and
//! wait for a password. So every invocation here passes a flag that cannot
//! reach it, and [`run`] refuses to spawn anything that does not -- a guard
//! against a future edit adding a case, not against the ones written here.
//!
//! Every run is also given a deadline. All of these are meant to print a line
//! and exit; a build that instead opened a window would otherwise wait for a
//! person who is not there, and a suite that hangs is worse than one that
//! fails. The regression that would do it is real -- if paths resolved from the
//! working directory again, the D6 test below would read an unconfigured config
//! and open the setup window -- so the deadline is what turns that into a
//! failure.
//!
//! Output capture works because the binary prefers an inherited stdout to a
//! console; see `report` in `main.rs`. Before phase 08 it looked for a console
//! only, so a run with no console would have shown a modal dialog and these
//! tests would have waited on it until the deadline.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The flags that are safe to run unattended. Anything else either locks the
/// screen or opens a window and waits for a person.
const SAFE: &[&str] = &["--version", "-V", "--help", "-h", "/?", "--setup"];

/// How long a run that is only meant to print a line gets.
///
/// Generous: these spawn a process and the first one pays for the OS loading
/// the image. It is a guard against waiting forever, not a performance budget.
const DEADLINE: Duration = Duration::from_secs(20);

/// A `config.ini` with both verifiers present, so `needs_setup()` is false.
///
/// The hashes are shaped like Argon2id PHC strings but are of no particular
/// password: nothing here authenticates, the copy only has to look configured.
const CONFIGURED: &str = "\
[LockScreen]
Title = Enter Password to Unlock
Transparency = 0.500
BlurBackground = false
PasswordHash = $argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$Zm9yZ2V0dGFibGVoYXNodmFsdWU
MasterHash = $argon2id$v=19$m=19456,t=2,p=1$YW5vdGhlcnNhbHR2YWw$YW5vdGhlcmZvcmdldHRhYmxlaA

[VoiceRecognition]
Enabled = false
UnlockCommand = unlock
ConfidenceThreshold = 1.00
ListeningTimeout = 10
";

/// A directory that cleans itself up, holding a copy of the program.
///
/// A copy rather than the built binary itself, because the whole point is to
/// control what sits *beside* the executable, and `target/debug` may already
/// have a `config.ini` from a previous run.
struct Portable {
    dir: PathBuf,
}

impl Portable {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lockscreen-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        std::fs::copy(env!("CARGO_BIN_EXE_LockScreen"), dir.join("LockScreen.exe")).expect("copy the program");
        Self { dir }
    }

    fn exe(&self) -> PathBuf {
        self.dir.join("LockScreen.exe")
    }

    fn write_config(&self, text: &str) {
        std::fs::write(self.dir.join("config.ini"), text).expect("write a config");
    }
}

impl Drop for Portable {
    fn drop(&mut self) {
        // Best effort: a failed test is more interesting than a tidy temp
        // folder, and on Windows a just-exited process can hold its image for a
        // moment after it goes.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a finished run said.
struct Ran {
    ok: bool,
    out: String,
}

/// Run the copy in `app`, with `cwd` as the working directory.
///
/// # Panics
///
/// If `args` contains nothing from [`SAFE`] -- see the module comment -- or if
/// the run outlives [`DEADLINE`], which means it opened a window instead of
/// printing.
fn run(app: &Portable, cwd: &Path, args: &[&str]) -> Ran {
    assert!(
        args.iter().any(|a| SAFE.contains(a)),
        "refusing to run the program with {args:?}: no safe flag, so this could lock the screen"
    );

    let mut child = Command::new(app.exe())
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program should start");

    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None if started.elapsed() > DEADLINE => break None,
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };

    if status.is_none() {
        // Killed rather than left running: it is holding a window open on
        // somebody's desktop.
        let _ = child.kill();
        let _ = child.wait();
        panic!("`LockScreen.exe {}` did not exit within {DEADLINE:?}; it opened a window", args.join(" "));
    }

    let out = child.wait_with_output().expect("wait");
    Ran { ok: status.expect("checked above").success(), out: String::from_utf8_lossy(&out.stdout).into_owned() }
}

#[test]
fn version_prints_the_version_and_succeeds() {
    let app = Portable::new("version");
    let ran = run(&app, &app.dir, &["--version"]);

    assert!(ran.ok, "--version should succeed: {:?}", ran.out);
    assert!(ran.out.starts_with("LockScreen "), "{:?}", ran.out);
    assert!(ran.out.contains(env!("CARGO_PKG_VERSION")), "{:?}", ran.out);
}

#[test]
fn help_prints_every_mode_and_succeeds() {
    let app = Portable::new("help");
    let ran = run(&app, &app.dir, &["--help"]);

    assert!(ran.ok, "--help should succeed: {:?}", ran.out);
    for flag in ["--settings", "--setup", "--version"] {
        assert!(ran.out.contains(flag), "{flag} should be documented in --help: {:?}", ran.out);
    }
    assert!(ran.out.contains("config.ini"), "--help should say where settings live: {:?}", ran.out);
}

#[test]
fn an_unknown_option_fails_and_says_which_one() {
    let app = Portable::new("unknown");
    // `--setup` is there so the guard in `run` passes. The unknown option is
    // what decides the outcome, which is the point: an unrecognised flag must
    // not fall through to locking the screen.
    let ran = run(&app, &app.dir, &["--elevate", "--setup"]);

    assert!(!ran.ok, "an unknown option must not succeed: {:?}", ran.out);
    assert!(ran.out.contains("--elevate"), "the message should name the option: {:?}", ran.out);
    assert!(ran.out.contains("Usage"), "and show how to use the program: {:?}", ran.out);
}

#[test]
fn setup_refuses_a_copy_that_already_has_credentials() {
    // The D7 guard, end to end. Changing a password has to go through settings
    // mode, which authenticates first; `--setup` is a flag anyone can pass.
    let app = Portable::new("d7");
    app.write_config(CONFIGURED);

    let ran = run(&app, &app.dir, &["--setup"]);

    assert!(!ran.ok, "--setup on a configured copy must fail: {:?}", ran.out);
    assert!(ran.out.contains("already set up"), "{:?}", ran.out);
    assert!(ran.out.contains("--settings"), "it has to say where to go instead: {:?}", ran.out);
}

#[test]
fn the_config_beside_the_program_wins_over_the_one_in_the_working_directory() {
    // Defect D6, and the reason portability works at all. The program is run
    // from a directory that has its own, deliberately *unconfigured*,
    // `config.ini`. If paths ever resolve from the working directory again the
    // program will read that one, conclude setup is outstanding, and open the
    // setup window -- which the deadline in `run` turns into a failure naming
    // exactly that.
    let app = Portable::new("d6");
    app.write_config(CONFIGURED);

    let elsewhere = std::env::temp_dir().join(format!("lockscreen-cli-d6-cwd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&elsewhere);
    std::fs::create_dir_all(&elsewhere).expect("a temp dir");
    std::fs::write(elsewhere.join("config.ini"), "[LockScreen]\nTitle = not this one\n").expect("write");

    let ran = run(&app, &elsewhere, &["--setup"]);

    assert!(!ran.ok, "{:?}", ran.out);
    assert!(ran.out.contains("already set up"), "the config beside the program should have been read: {:?}", ran.out);
    assert!(!ran.out.contains("not this one"), "{:?}", ran.out);

    let _ = std::fs::remove_dir_all(&elsewhere);
}

#[test]
fn the_program_runs_from_a_path_with_spaces_and_unicode_in_it() {
    // Portable means "wherever somebody put it", and that includes
    // `C:\Users\...\OneDrive\Dokumen\Lock Screen\`. The paths all come from
    // `GetModuleFileNameW`, which is wide and handles this; the test is here
    // because a regression to a narrow or shell-quoted path would not show up
    // anywhere else.
    let app = Portable::new("a folder with spaces and ünïcode");
    app.write_config(CONFIGURED);

    let ran = run(&app, &app.dir, &["--setup"]);
    assert!(ran.out.contains("already set up"), "{:?} (in {})", ran.out, app.dir.display());
}
