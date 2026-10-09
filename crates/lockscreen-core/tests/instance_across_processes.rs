//! The instance guard, across real processes.
//!
//! `instance.rs` has a unit test, but it can only prove the guard works
//! *within* one process: it claims the name twice and then drops the first
//! claim. Dropping stands in for the process ending. The two properties that
//! actually matter in the field are not reachable that way:
//!
//! 1. a **second process** cannot claim the name while a first one holds it;
//! 2. the name becomes claimable again when the holder is **killed** -- not
//!    closed, not dropped, killed, with no chance to run any cleanup. The
//!    module comment in `instance.rs` rests the whole design on this ("the
//!    kernel releases it when the process ends however it ends, including a
//!    kill"), and until phase 08 nothing tested it. A lock screen that refused
//!    to start because a previous one crashed would be a bad failure: the
//!    screen would be left unlocked by the very mechanism meant to lock it.
//!
//! The second process is this test binary, re-run with `--ignored` to reach
//! [`hold_the_lock_until_killed`]. Re-running the test binary rather than
//! adding a helper `[[bin]]` keeps the whole thing in one file and means no
//! extra target ships in the crate.
//!
//! Only one test claims the name, deliberately. Two would race -- `cargo test`
//! runs the tests in a binary in parallel, and the name is session-wide. That
//! mistake was made once already in `instance.rs` and produced a test that
//! passed or failed by scheduling.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lockscreen_core::instance::{Instance, Taken};

/// Set on the child, and carries where it should write its readiness marker.
const HOLDER_MARKER: &str = "LOCKSCREEN_TEST_HOLDER_MARKER";

/// How long to wait for the child to come up, and how long it holds the name.
///
/// The hold is bounded so that a parent which dies mid-test cannot leave a
/// process sitting on the name and break every later run.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const HOLD_FOR: Duration = Duration::from_secs(30);

#[test]
fn a_second_process_is_turned_away_and_killing_the_first_frees_the_name() {
    let marker = std::env::temp_dir().join(format!("lockscreen-holder-{}.marker", std::process::id()));
    let _ = std::fs::remove_file(&marker);

    let mut holder = Command::new(std::env::current_exe().expect("the test binary's own path"))
        .args(["--exact", "hold_the_lock_until_killed", "--ignored", "--nocapture"])
        .env(HOLDER_MARKER, &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("a second copy of this test binary should start");

    // Guarantees the child is reaped whatever this test does from here,
    // including panicking on an assertion. Without it a failure would leave a
    // process holding the name for `HOLD_FOR`, and every later run would fail
    // for a reason that had nothing to do with the code.
    struct Reap<'a>(&'a mut std::process::Child);
    impl Drop for Reap<'_> {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    {
        let reap = Reap(&mut holder);

        let ready = Instant::now();
        while !marker.exists() {
            if let Some(status) = reap.0.try_wait().expect("try_wait") {
                panic!("the holder exited before claiming the name: {status:?}");
            }
            assert!(ready.elapsed() < READY_TIMEOUT, "the holder never claimed the name within {READY_TIMEOUT:?}");
            std::thread::sleep(Duration::from_millis(25));
        }

        // Property 1: a second lock screen must not start while one is up. This
        // process is the second one, and the first is a different process --
        // which is what the unit test cannot arrange.
        assert_eq!(
            Instance::claim().err(),
            Some(Taken::AlreadyRunning),
            "another process holds the name, so this claim must be refused"
        );

        // Property 2: kill it. No unwinding, no destructors, no `CloseHandle` --
        // the kernel is the only thing that can release the name now.
        reap.0.kill().expect("kill the holder");
        reap.0.wait().expect("reap the holder");
    }

    // The handle is released as the process is torn down, which is not
    // instantaneous, so this is a short retry rather than a single attempt. If
    // it were a single attempt it would pass or fail by timing -- the same
    // mistake the unit test's history records.
    let freed = Instant::now();
    let reclaimed = loop {
        match Instance::claim() {
            Ok(instance) => break Ok(instance),
            Err(e) if freed.elapsed() >= Duration::from_secs(5) => break Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    assert!(reclaimed.is_ok(), "the name should be claimable after its holder was killed: {:?}", reclaimed.err());

    let _ = std::fs::remove_file(&marker);
}

/// The child process: claim the name, say so, and wait to be killed.
///
/// Ignored, so it never runs as part of the suite. It also does nothing unless
/// the parent set [`HOLDER_MARKER`], so running `cargo test -- --ignored` by
/// hand does not block for half a minute.
#[test]
#[ignore = "the child half of the cross-process test; run only by its parent"]
fn hold_the_lock_until_killed() {
    let Ok(marker) = std::env::var(HOLDER_MARKER) else {
        eprintln!("not started by the cross-process test; doing nothing");
        return;
    };

    let _instance = Instance::claim().expect("the child should be able to claim the name");

    // Written only *after* the claim succeeded, so the parent cannot race it.
    std::fs::write(PathBuf::from(marker), b"held").expect("write the readiness marker");

    // Bounded: the parent kills this long before the sleep is over, and the
    // bound is only here so a parent that died cannot leave the name held.
    std::thread::sleep(HOLD_FOR);
}
