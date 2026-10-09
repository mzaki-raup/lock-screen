# Lock Screen

A configurable lock-screen overlay for Windows 10 (build 17763 or later) and Windows 11. It covers
every connected monitor, and any key or mouse click brings up an unlock prompt that tries Windows
Hello first — face or fingerprint, falling back to your PIN — then an app password, with an optional
spoken command as a convenience and a master password as a recovery path.

- **One executable**, three modes: the overlay, the settings window, and first-run setup.
- **Portable.** Everything lives beside the exe. Nothing in `%APPDATA%`, no installer, no registry
  keys beyond the Task Manager policy it sets while locked and removes on unlock.
- **Raw Win32**, no GUI toolkit, no .NET runtime to install. The binary is about 654 KB.
- **Fully local.** The only network access is the update check, and only when you ask for it from
  the settings window.

> **Status — please read.** This is a deterrent overlay, **not a security boundary.** It is an
> ordinary user-mode process drawing an always-on-top window, so anyone who can run code as your
> user can terminate it, and `Ctrl+Alt+Del` is a Secure Attention Sequence that no application can
> intercept — Sign out, Switch user and the real Windows lock all remain reachable. Use it to stop
> someone casually poking at an unattended desk, not to stop a determined attacker with an account
> on the machine. If you need an unbypassable lock, the mechanism is a Windows Credential Provider,
> which this is not. See [Limitations](#limitations).

## Unlock methods

| Order | Method | Notes |
| --: | --- | --- |
| 1 | **Windows Hello** — face, fingerprint, or PIN | Attempted automatically when the prompt appears. Windows chooses the modality and handles its own retry limits. Skipped when Hello is not set up, with no wait. |
| 2 | **App password** | Shown when Hello is unavailable or you dismiss it. Minimum 4 characters. Stored as an Argon2id verifier, never recoverable. Attempts are slowed after three wrong ones. |
| 3 | **Voice command** | Off by default. Say your chosen word. A convenience, not a factor — see [Limitations](#limitations). |
| 4 | **Master password** | Never offered automatically. The button appears only once you are at the password box, and only if a master password is set. Minimum 8 characters. Slowed after a single wrong attempt, because a recovery credential should not be a fast oracle. |

## First run

There is no default password. Run the app and it asks you to set two:

```
LockScreen.exe --setup
```

The unlock password must be at least **4** characters; the master password at least **8**. They must
differ from each other. Character classes are not restricted for either — length is the property
that matters, and the rule this replaces accepted `abc` while rejecting `correct-horse-battery`.

The floors differ because the two are used differently. The unlock password is the one typed at the
prompt, often many times a day. The master password is the recovery credential: never offered
automatically, slowed after a *single* wrong attempt rather than three, and the one that must not be
guessable. See [Limitations](#limitations) for what the shorter floor costs.

The master password is **not** your Windows account password, and the app never asks for that. It is
a second credential of your choosing, for the case where you have forgotten the first.

Setup refuses to run on a copy that is already configured. To change a password afterwards, use the
settings window, which asks you to authenticate first.

## Requirements

- Windows 10 build 17763 (1809) or later, 64-bit; Windows 11 supported
- Nothing else to install — the binary is self-contained
- Windows Hello hardware or a configured PIN for method 1; without it the prompt goes straight to
  the password
- A microphone and Windows speech recognition for method 3, if you enable it

## Install

Download the zip from the [releases page](https://git.zacx.net/owner/lock-screen/releases), unpack
it anywhere you can write, and run it. That is the whole install.

```
LockScreen.exe            the program
config.ini                your settings, written on first run
LockScreenData/
  lockscreen.log          what happened, truncated at 256 KB
  update/                 downloads, derived -- safe to delete
```

Paths resolve from the executable's own location, so it does not matter what directory you launch
from or what a shortcut's *Start in* field says. Copy the folder to a USB stick and the settings go
with it, which works because the passwords are hashes rather than values tied to one Windows user on
one machine.

Put it somewhere writable: the app checks up front and refuses to start otherwise, rather than
discovering it after you have typed a new password.

## Usage

```
LockScreen.exe              lock the screen
LockScreen.exe --settings   open settings (asks you to authenticate first)
LockScreen.exe --setup      choose the two passwords (refuses if already set up)
LockScreen.exe --version    print the version and build
LockScreen.exe --help       print the usage
```

Only one overlay can run at a time. Start it twice and the second copy exits quietly — the screen is
already locked, which is what you wanted.

While the overlay is up it suppresses the Windows key, `Alt+Tab`, `Alt+F4`, `Alt+Esc`, `Ctrl+Esc`
and `Ctrl+Shift+Esc`, and sets the per-user policy that disables Task Manager. The policy is
restored on unlock — and if you had set it yourself beforehand, it is left alone.

## Configuration

The settings window is the intended way to edit this; the file is documented because it is plain
text and portable.

```ini
[LockScreen]
Title = Enter Password to Unlock
Transparency = 0.500
BlurBackground = false
PasswordHash = <Argon2id PHC string>
MasterHash = <Argon2id PHC string>

[VoiceRecognition]
Enabled = false
UnlockCommand = unlock
ConfidenceThreshold = 1.00
ListeningTimeout = 10
```

| Key | Meaning |
| --- | --- |
| `Title` | Text shown on the unlock prompt |
| `Transparency` | `0.0` (invisible) to `1.0` (opaque black) |
| `BlurBackground` | Blurs the desktop behind the overlay, on **Windows 11 22H2 and later**. The overlay is built one of two ways depending on this — a layered window has no surface for the compositor to blur — and `Transparency` applies either way: as the window's alpha when blur is off, and as the tint painted over the backdrop when it is on. Where the backdrop API is unavailable it degrades to plain translucency and the setting reports itself as unavailable. |
| `PasswordHash`, `MasterHash` | Argon2id verifiers with independent random salts, so rotating one does not invalidate the other. Not reversible, not comparable by eye, and useless to anyone who copies the file. |
| `UnlockCommand` | The word to listen for. Lower-cased on read. |
| `ConfidenceThreshold` | `0.0`–`1.0`, mapping onto WinRT's Low (0.4) / Medium (0.7) / High (1.0). Defaults to `1.00`. |
| `ListeningTimeout` | Seconds to listen before giving up |

Out-of-range values are clamped rather than rejected, so a hand-edited file still opens the lock
screen.

### Coming from the .NET version

Put your old `config.ini` beside `LockScreen.exe` and run it. The appearance and voice settings
carry over; the password does not, and the app says so. The old one was a DPAPI blob — reversible,
and bound to one Windows user on one machine, which is exactly why the portable layout works now and
did not before. You will be asked to choose both passwords, and the old `Password` key is dropped
rather than written back.

## Updates

Settings → check for updates. Deliberately dull, and deliberately manual:

- **Nothing is checked in the background**, and the update path is unreachable from the overlay.
  Somebody at a locked desktop should not be handed something that writes a new executable.
- **HTTPS only.** A non-HTTPS URL is refused outright, so no redirect in an asset URL can talk it
  into plain HTTP. TLS and certificate validation come from Schannel and the system certificate
  store.
- **The published SHA-256 must match** before anything is replaced. A release without checksums is
  never installed automatically, only opened in your browser.
- **Only a strictly newer version** is offered, so a hostile answer cannot walk an install back to a
  known-bad build.
- The running program is renamed aside and the new one put in its place; **anything that fails puts
  the old name back**, so a failed update leaves a working program rather than none at all.
- It does not restart itself. Close the window and start it again — relaunching could put the
  *overlay* back up, and restarting a lock screen mid-session is a bypass rather than a convenience.

Releases are **not currently Authenticode-signed** (see [Limitations](#limitations)).

## Building

Current stable Rust, on Windows. No toolchain file and no `global.json`, deliberately.

```bash
cargo build --release
```

The four gates, which are the same ones the release workflow runs:

```bash
cargo fmt --check
cargo clippy --release --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

126 tests, none of which need a window — the logic that can be tested without one is kept separate
from the Win32 that cannot. Three machine diagnostics are ignored by default:

```bash
# what this machine can do: folder, config, monitors, Hello, microphone
cargo test -p lockscreen --bin LockScreen environment -- --ignored --nocapture
```

## Limitations

Honest ones. The first two are properties of the approach, not bugs to be fixed.

- **`Ctrl+Alt+Del` is not suppressed and cannot be.** It is a Secure Attention Sequence; no
  low-level keyboard hook sees it. Sign out, Switch user and the real Windows lock stay reachable,
  and so does Task Manager through them.
- **It is a user-mode process.** Anyone who can run code as your user can end it, and the Task
  Manager policy it sets is an `HKCU` value removable by the same user it restricts.
- **A four-character unlock password is weak against an *offline* attack.** At the machine it is
  slow to guess: the stored verifier is Argon2id, and the prompt throttles after three wrong
  attempts. But `config.ini` is portable by design — the point of the format is that the folder can
  be copied — and four characters in a copied config falls quickly to an attacker with the file. The
  master password's eight-character floor is what stands behind it. Use more than four if the folder
  will ever leave your control.
- **Voice recognition is speaker-independent.** It recognises the word, not the person — so with it
  enabled, anyone who can speak can unlock the machine. That is why it is off by default, why the
  confidence floor is raised to High, and why it is described above as a convenience rather than a
  factor. Leave it off unless you have thought about that.
- **Releases are unsigned.** `SHA256SUMS.txt` travels the same channel as the binary, so it protects
  against a network attacker and a truncated download — not against a compromise of the forge
  itself. Only Authenticode covers that, and the certificate is an open question.
- **Windows 10 has not been run on.** The supported floor is 17763 and the code paths are there,
  but no Windows 10 machine has been available to confirm it. Note that Windows 10 gets **no blur**
  either way: the only mechanism for it there is the undocumented
  `SetWindowCompositionAttribute`, which is not in `windows-rs` and would mean hand-transcribing a
  struct layout in the one process that covers the screen when the machine is unattended. That is
  the same class of risk as the handle truncation this port removed, and a cosmetic effect does not
  justify it. Blur degrades to plain translucency there.
- **The parity checklist is not finished.** [`PARITY.md`](PARITY.md) §3 lists what still needs a
  person at the keyboard; the overlay covers the screen and Hello cannot be answered by software, so
  no test can do it.

## Repository layout

```
crates/
  lockscreen/           the binary -- Win32, one executable, three modes
    src/mode/           overlay, settings, setup: what each mode does
    src/win/            windows, hooks, monitors, Hello, voice, updates
    assets/             icon and the manifest that declares PerMonitorV2
    tests/cli.rs        end-to-end, against the built executable
  lockscreen-core/      no windows, therefore testable without one:
                        config, credentials, paths, logging, policy, releases
RUST-MIGRATION.md       the port's feasibility study and decision record
PARITY.md               what is verified, how, and what is still open
```

The original .NET 8 / WPF implementation is preserved at the **`dotnet-final`** tag and on the
`backup/dotnet-wpf` branch. Code comments cite it by path and line; those paths resolve there, not
here:

```bash
git worktree add ../lock-screen-dotnet dotnet-final
```

[`RUST-MIGRATION.md`](RUST-MIGRATION.md) is the full study — parity matrix, measured evidence, the
seven defects the port fixes, and why each decision went the way it did. Read it before changing
anything architectural.

## Credits

Lock Screen — © 2026 Procybro Solutions. Author: Mohd Zaki Raup.

No licence file is present in this repository; treat the code as all rights reserved unless that
changes.
