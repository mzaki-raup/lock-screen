# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this repository is

A Windows lock-screen overlay, **being ported from .NET 8 / WPF to Rust**. On
this branch the Rust implementation is the one under development; the C# is kept
for reference while the port finishes.

- **`crates/`** — the Rust port. This is where new work goes.
- **`LockScreenApp/`, `SettingsApp/`** — the original .NET 8 WPF apps, legacy.
  Also preserved on the `backup/dotnet-wpf` branch and the `dotnet-final` tag,
  so they can be deleted from this branch whenever the port no longer needs them
  for comparison.
- **`RUST-MIGRATION.md`** — the feasibility study and the record of every design
  decision taken so far. **Read it before changing architecture**; it carries the
  reasoning, the measurements, and the defect numbering (D1–D7) used throughout
  the code comments.
- **`PARITY.md`** — phase 08. What the suite proves, what was measured on real
  mixed-DPI hardware, and the checklist of parity checks that need a person at
  the keyboard. Add to the checklist rather than quietly deciding something is
  verified.

## Build and test

```bash
cargo build
cargo test
cargo clippy --release --all-targets --locked -- -D warnings
cargo fmt --check
```

All four must pass. They are the same gates the release workflow runs, so a
change that fails `clippy -D warnings` locally will fail the release too.

Three diagnostics are ignored by default, because they assert nothing and their
output depends on the machine. Run them deliberately:

```bash
# what this machine can do: folder, config, monitors, Hello, microphone
cargo test -p lockscreen --bin LockScreen environment -- --ignored --nocapture
# the monitor layout, with per-monitor DPI
cargo test -p lockscreen --bin LockScreen layout -- --ignored --nocapture
# what a DPI-unaware process sees instead -- the measurement behind D2
cargo test -p lockscreen --bin LockScreen dpi_virtualisation -- --ignored --nocapture
```

The legacy .NET apps, if you need to compare behaviour:

```bash
dotnet build LockScreenApp/LockScreenApp.csproj
dotnet build SettingsApp/SettingsApp.csproj
```

There is no `global.json` and no `rust-toolchain.toml`, deliberately — the house
convention across sibling projects is current stable. The `.sln` was removed for
the same reason: it was a Visual Studio artifact with no role in the port.

## The binary

One executable, three modes. Not two executables: a GUI crate would be linked
twice, and two binaries can be updated independently and then disagree about the
config format.

```
LockScreen.exe              lock the screen
LockScreen.exe --settings   configure (authenticates first)
LockScreen.exe --setup      first-run passwords (refuses if already set up)
```

**Mode dispatch is not a security boundary.** Anyone can pass `--settings`. The
gate is authentication, which is why settings mode authenticates before opening
a window rather than trusting how it was started.

## Conventions

Taken from the sibling project `flex-taskbar`, which is the reference for
anything not covered here. Match it rather than inventing.

- **No GUI toolkit.** Raw Win32 through `windows-rs`. `eframe` needs a live GPU
  context and has no software renderer, which for a lock screen is a lockout
  risk; both toolkits also measured ~10 MB against this binary's ~500 KB. See
  §5 of the study.
- **Portable.** Everything lives beside the executable: `config.ini` for
  settings, `LockScreenData/` for logs and update downloads. Nothing in
  `%APPDATA%`. Paths resolve from `GetModuleFileNameW` — **never**
  `std::env::current_dir()`, which is defect D6.
- **Physical pixels throughout.** The manifest declares `PerMonitorV2`, so there
  is no device-independent unit to mix them up with. That mixture is defect D2.
- `windows` crate features are listed explicitly and alphabetically in the
  workspace `Cargo.toml`, each with a comment saying what needs it.
- Windows resources (icon, manifest, `VERSIONINFO`) come from `build.rs` via
  `embed-resource`, generated from `CARGO_PKG_VERSION`. The version lives in one
  place: the workspace `Cargo.toml`.
- `rustfmt.toml`: 120 columns, `use_small_heuristics = "Max"`.
- Supported floor is **Windows 10 build 17763 (1809)**.

## Things that will bite you

Learned the hard way; each cost real time.

- **A layered window cannot carry a DWM backdrop.** `WS_EX_LAYERED` has no DWM
  surface, so asking for a backdrop *replaces* what is behind the window instead
  of blurring it. The overlay is therefore built one of two ways depending on
  the blur setting — see `win/backdrop.rs`.
- **`SetLayeredWindowAttributes` fails, not no-ops, on a non-layered window.**
  Add `WS_EX_LAYERED` with `SetWindowLongPtrW` first, or the overlay is an
  invisible hole over the desktop.
- **XML comments cannot contain `--`.** A dash in `app.manifest` makes it
  malformed, and Windows reports only "side-by-side configuration is incorrect"
  (error 14001) with no hint why.
- **`windows-registry` is on the 0.100 version line** and its `Error` will not
  cross a `?` from `windows` 0.62. Use `Win32_System_Registry` from the
  `windows` crate instead.
- **`SpeechRecognitionConfidence` is numbered `High = 0 … Rejected = 3`.**
  Comparing the raw values inverts the test.
- **`.gitignore` has a blanket `*.ini`**, so a new test fixture needs an
  explicit negation or it is silently left out of the commit.
- **Low-level hooks must be fast.** Windows removes a hook whose callback
  overruns `LowLevelHooksTimeout`. Set a flag and return.

## Credentials

Passwords are **Argon2id verifiers**, never recoverable. The .NET version stores
a DPAPI blob — reversible, bound to one user on one machine, and compared with
`==` — which is defect D5 and the reason portability works now.

A DPAPI reader survives in `credential::legacy` **only** to migrate an old
install's settings. It must never authenticate anyone.

Unlock ladder: Windows Hello (biometric, then PIN) → app password → voice
(off by default) → master password, which is never offered automatically.

Voice recognition is **speaker-independent**: it recognises the word, not the
person. It is a convenience that bypasses the password, not a factor. Keep it
off by default.

## Testing

Pure logic is separated from Win32 so it can be tested without a window —
`hooks::should_suppress`, `setup::validate`, `voice::required_level`,
`monitors::index_at`. Prefer adding to those over testing through a window.

`crates/lockscreen-core/tests/dotnet_config_compat.rs` reads a fixture generated
by the real .NET code path. It is the regression gate on config compatibility
and it caught two traps a hand-written fixture would have missed: the .NET writer
emits `key = value` with spaces, and the file carries a UTF-8 BOM.

Two integration tests drive real processes, and both have a rule attached:

- **`crates/lockscreen/tests/cli.rs`** runs the built executable. It must never
  reach the overlay — that would cover the screen, install a keyboard hook and
  wait for a password on whoever's machine is running the suite. Every
  invocation goes through `run`, which refuses args carrying no safe flag, and
  every run has a deadline so a build that opens a window fails instead of
  hanging. Keep both if you add a case.
- **`crates/lockscreen-core/tests/instance_across_processes.rs`** re-runs itself
  to hold the instance mutex from a second process. Only **one** test there may
  claim the name: the mutex is session-wide and `cargo test` runs a binary's
  tests in parallel, which is how `instance.rs` once ended up with two tests
  that passed by scheduling.
