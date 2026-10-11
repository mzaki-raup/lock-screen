# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this repository is

A Windows lock-screen overlay in Rust, **ported from .NET 8 / WPF**. The port is
finished and merged; the C# is gone from the tree.

- **`crates/`** — the whole program. `lockscreen` is the binary, Win32 and all;
  `lockscreen-core` is the part that needs no window, and therefore the part
  that can be tested without one.
- **The .NET original** is at the **`dotnet-final`** tag and on the
  `backup/dotnet-wpf` branch. Comments throughout the code cite it by path and
  line (`SettingsApp/MainWindow.xaml.cs:88`, and so on) — those paths resolve
  against that tag, not against this tree. To read or build it:

  ```bash
  git worktree add ../lock-screen-dotnet dotnet-final
  ```
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

To compare behaviour against the .NET original, build it from the worktree
above — not from this tree, which no longer contains it:

```bash
dotnet build ../lock-screen-dotnet/LockScreenApp/LockScreenApp.csproj
```

There is no `global.json` and no `rust-toolchain.toml`, deliberately — the house
convention across sibling projects is current stable. The `.sln` went for the
same reason, and `.gitignore` is a Rust one now rather than the Visual Studio
template it started as: two entries in it are load-bearing, `*.ini` and the
negation that lets the test fixture through.

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
- **A window class with a null `hCursor` shows the "app starting" hourglass**,
  not the arrow. Windows does not fall back to the arrow; it leaves the cursor
  as it was, and over a window whose class never set one you get the busy
  pointer. All four classes use `ui::arrow_cursor`, and a test reads the four
  source files to check they still do.
- **Low-level hooks must be fast.** Windows removes a hook whose callback
  overruns `LowLevelHooksTimeout`. Set a flag and return.
- **A backslash does not survive being written through a shell heredoc into a
  Python string literal.** It collapses before Python sees it, and Python then
  treats the bare backslash as its *own* line continuation -- so a Rust string
  continuation written that way arrives as one long line with the indentation
  baked in as literal spaces. That shipped in 0.2.3 and reached a user as
  `is required.                  Try again`. The same edit then broke `'
'`
  into a real newline and failed to compile.

  Write Rust fragments with `cat > file.frag <<'EOF'`, which preserves the text
  exactly, and insert the file's contents -- or use the Write tool. Never put a
  backslash in a Python literal destined for a source file; `chr(10)` and
  `.lines().count()` get the job done without one.

  The backstop is a test: every user-facing message in `voice`, `hello`, `setup`
  and `settings` asserts it contains no double space and spans no lines. Keep
  new messages in those lists -- the bug shipped because `Outcome::Unsure` was
  missing from one.

## Releases

Tag `vX.Y.Z` and push the tag **on its own**, after the branch has landed —
pushing both refs in one `git push` has left the forge without its own row for
the tag, which fails the publish step after the build has already succeeded.
The tag must match the version in the workspace `Cargo.toml`, and `Cargo.lock`
has to move with it in the same commit, because every step runs `--locked`.

Compiled artifacts in CI live in `C:/ci-cache/lock-screen/target`, outside the
workspace, because the first step of every job wipes the workspace. Both
workflows share that directory on purpose: a release pushes a branch and then a
tag, so `build.yml` and `release.yml` run the same four gates on the same
commit, and the second one then recompiles nothing. Measured locally, the four
gates are 85 seconds cold and 20 warm. Do not move the target directory back
inside the workspace without reading that sentence twice.

Releases are **unsigned**, deliberately, as of 9 October 2026. The Sign step
works and is tested, but a publicly trusted certificate cannot live in a CI
secret — code-signing keys must be held in hardware — so signing needs a
key-holding service. See `PARITY.md` §4. Do not add a self-signed certificate
to make the step pass: the verification refuses it on purpose, because a
signature that does not chain to a trusted root looks like assurance and is not.

## Two forges

Since 11 October 2026 the repository lives in two places that hold **one
history**: the Forgejo at `git.zacx.net/owner/lock-screen`, where development
happens, and a public mirror on GitHub under a different account. There is no
sync tool and nothing to transform: the same commits are pushed to both, and
each forge builds and publishes its own release from them. The two binaries
differ in exactly one thing, the releases URL baked in at build time.

- **`LOCKSCREEN_RELEASES_API`** is read by `option_env!` in `win/update.rs`.
  Unset, which is every local build, it is the Forgejo. Each release workflow
  sets it from its own `github.server_url` and `github.repository`, so a copy
  updates from the forge it was downloaded from. `build.rs` declares the
  variable with `rerun-if-env-changed` so the shared CI target directory
  cannot hand one forge's build the other's object file. Do not make it a
  config setting: whoever can edit `config.ini` could then point the updater
  anywhere.
- **The Forgejo owns the version.** Neither workflow bumps a version or makes a
  tag; both refuse a tag that disagrees with `Cargo.toml`. A release is still
  one tag, made here, pushed to both. If GitHub ever bumps versions on its own
  the two streams diverge within a week.
- **Workflows live in two directories.** `.forgejo/workflows/` runs on the
  Forgejo and uses no published actions (forgejo#2809). `.github/workflows/`
  runs on GitHub. Forgejo reads `.github/workflows/` as well, so every job in
  there carries `if: github.server_url == 'https://github.com'` — without it
  the Forgejo tries the GitHub job, fails on its first `uses:`, and every push
  there goes red. Keep that guard on any job you add.
- **One `origin`, two push URLs.** `git push origin main` lands on both. A push
  is rejected, not forced, when a forge has a commit this clone does not, which
  is what makes both sides safely writable. **Never add a Forgejo push mirror**:
  it pushes with force and would silently discard anything committed on GitHub.
  The reverse direction is `scripts/sync-from-github.ps1`, fast-forward only.

## Credentials

Passwords are **Argon2id verifiers**, never recoverable. The .NET version stores
a DPAPI blob — reversible, bound to one user on one machine, and compared with
`==` — which is defect D5 and the reason portability works now.

A DPAPI reader survives in `credential::legacy` **only** to migrate an old
install's settings. It must never authenticate anyone.

Unlock ladder: Windows Hello (biometric, then PIN) → app password → voice
(off by default) → master password, which is never offered automatically.

**The two length floors differ on purpose**: 4 characters for the unlock
password, 8 for the master. Both live in `credential::Kind::min_length`, and
every caller names the kind, so the master cannot be checked against the shorter
floor by accident. Do not "simplify" them back into one number -- the master
password is the recovery credential and must not be the weaker of the two. No
UI string hard-codes either value; they are all formatted from `Kind`.

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
