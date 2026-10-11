# Parity validation — phase 08

The last phase of the port, as `RUST-MIGRATION.md` §10 defines it: *side-by-side on mixed-DPI
multi-monitor, with and without Hello hardware, with and without a microphone, and against a
`config.ini` from the .NET build.*

This file is the record of it. Phase 08 splits cleanly in two, and the split is worth stating
because it decides what belongs in the test suite and what does not:

- **Machine-checkable.** Config compatibility, path resolution, the instance guard, DPI awareness.
  These went into the suite, where they run on every build and keep working. They are listed in §1.
- **Human-checkable.** Everything that puts the overlay on screen or raises a Windows Hello prompt.
  No test can do this: the overlay covers the display, installs a system-wide keyboard hook and
  waits for a password, and Hello deliberately cannot be answered by software. These are the
  checklist in §3.

Run the capability report first, because the checklist means little until you know what the machine
can actually do:

```bash
cargo test -p lockscreen --bin LockScreen environment -- --ignored --nocapture
```

---

## 1. What the suite now proves

Phase 08 added 9 tests, taking the suite from 118 to 126. It has since grown to **148**, and the
rows below are kept current rather than left as a snapshot of that phase: several were added because
something reached a user that should not have.

The column that matters is the last one. *A test that cannot fail is not evidence*, so where a test
was checked by deliberately breaking the thing it guards, that is said.

| Property | Where | What it catches |
| --- | --- | --- |
| The `config.ini` **beside the executable** wins over one in the working directory | `lockscreen/tests/cli.rs` | A return to `std::env::current_dir()` — defect **D6**. Every unit test would still pass. |
| `--setup` **refuses** a copy that already has credentials | `lockscreen/tests/cli.rs` | The **D7** guard being bypassed. `main.rs` tests the message; this tests that the program reaches it. |
| An unknown option fails instead of **locking the screen** | `lockscreen/tests/cli.rs` | A mistyped flag falling through to the overlay. |
| The program works from a path with **spaces and non-ASCII** in it | `lockscreen/tests/cli.rs` | A regression to a narrow or shell-quoted path. "Portable" means wherever somebody put it. |
| A **second process** cannot claim the instance lock | `lockscreen-core/tests/instance_across_processes.rs` | Two overlays stacking, each with its own hooks — which is what the .NET app does today. |
| A **killed** holder frees the lock | same | A crashed lock screen stopping the next one from starting, which would leave the screen unlocked by the very mechanism meant to lock it. |
| This process **really is per-monitor aware** | `lockscreen/src/win/monitors.rs` | `app.manifest` stopping being embedded — a silent failure that turns every overlay coordinate into a virtualised one. Live on any monitor above 100%; vacuous below. |
| Output goes to a **redirected stdout**, not a modal dialog | `lockscreen/tests/cli.rs`, by existing at all | See §2.3. |
| A folder the program **cannot write to** is refused up front | `lockscreen/tests/cli.rs` | Discovering it at save time, after somebody has typed a new password. Was **checklist item 20** until 0.2.4; the test denies itself write access with `icacls`, says so out loud if the deny does not bite, and was confirmed by disabling the check and watching it fail. |
| The updater understands **this forge's actual JSON** | `lockscreen-core/tests/forgejo_release_compat.rs` | A field named `download_url` rather than `browser_download_url`. Every hand-written test would pass while the app reported "no update" for ever. The fixture is a verbatim API response, with two releases so that *picking the newest* is distinguishable from picking the first. |
| No user-facing message carries a **flattened line continuation** | `voice`, `hello`, `setup`, `settings` | `is required.                  Try again`, which shipped in 0.2.3 and reached a user. Every message in those four modules is checked for double spaces and line breaks. |
| The **two password floors** stay 4 and 8, and the master is never the weaker | `lockscreen-core/src/credential.rs` | A "simplification" back to one number. The master password is the recovery credential. |
| A required-match level **survives the round trip** through the stored float | `lockscreen/src/win/voice.rs` | A setting that silently fails to stick: choose Medium, save, reopen, see High. |

Both cross-process properties were checked by breaking them on purpose and confirming the test
failed — first the holder not claiming, then the holder not being killed. The second attempt caught
something worth knowing: the first version of that mutation was ineffective, because the RAII guard
killed the child anyway. Without re-doing it, the test would have looked verified when only half of
it was.

The .NET config compatibility gate predates this phase and still holds: `dotnet_config_compat.rs`
reads a fixture generated by the real .NET code path, which is where the UTF-8 BOM and the
`key = value` spacing came from.

---

## 2. Measured on this machine

Two monitors, mixed DPI — exactly the hardware §11 of the study names as the thing to test on, and
which was not available until now.

```text
Lock Screen 0.1.0
  monitor     \\.\DISPLAY2   1920x1080 at (0,0) dpi 96 (100%) primary
  monitor     \\.\DISPLAY1   1920x1080 at (1920,0) dpi 144 (150%)
  hello       available
  voice       a recogniser compiled, so the microphone path works
```

The name in that first line is as 0.1.0 printed it. It is one word, `LockScreen`, from 0.2.4 --
the block is left as captured rather than tidied, because it is a record of what the machine
answered and not a sample of current output.

### 2.1 What each DPI awareness sees

```text
device              unaware  per-monitor    dpi
\\.\DISPLAY2      1920x1080    1920x1080     96
\\.\DISPLAY1       1280x720    1920x1080    144
```

The port reads **1920x1080** for both, which is what they physically are. A DPI-unaware thread
reads the 150% monitor as **1280x720** — the physical size scaled by `96/dpi`, two thirds.

The .NET app is in the second column. It declares no DPI awareness anywhere: there is no
`app.manifest` in `LockScreenApp.csproj`, no `ApplicationHighDpiMode`, no `SetHighDpiMode` call, and
the manifest the .NET SDK embedded in the built `LockScreen.exe` carries only a
`requestedExecutionLevel`. That was read out of the binary rather than inferred.

### 2.2 A correction to D2

The study says of D2: *"`Screen.Bounds` returns physical pixels; assigning them to WPF's
`Left/Top/Width/Height` treats them as device-independent units … a 150%-scaled monitor leaves the
desktop uncovered."*

The first clause is **wrong**, and the measurement above is why. In a DPI-unaware process
`Screen.Bounds` does *not* return physical pixels for a scaled monitor; it returns the virtualised
1280x720. WPF's device-independent unit in that process is one virtualised pixel, so the window it
builds matches the monitor in that coordinate space, and Windows then stretches it back up to
1920x1080. The expected symptom on this layout is therefore a **bitmap-stretched, soft overlay and
prompt** — not a hole.

D2 remains a real defect: the app does no DPI handling, and a lock screen whose password prompt is
blurry on one of two monitors is visibly worse than one that is not. What changes is the mechanism
and the predicted symptom. **Which of the two it actually is, is item 2 of the checklist below** —
that needs the .NET overlay on screen, and reasoning about DWM stretching is not evidence. The
numbers above are measured; the visual consequence is not, and is flagged rather than asserted.

Either way the port is unaffected: it works in physical pixels end to end, and the test in §1 now
fails if that ever stops being true.

### 2.3 Something phase 08 found and fixed

`report` in `main.rs` looked for a **console** and otherwise showed a message box. A GUI-subsystem
binary started with its output redirected but without a console — a pipe, a file, a test harness —
therefore got a modal dialog instead of a line of text, with nothing to click it with. It now
prefers an inherited stdout, falls back to the parent's console, and only then to the dialog.

The end-to-end tests in §1 are exactly that caller. Without the fix they would have hung rather
than failed, which is why every run in `cli.rs` also carries a deadline.

---

## 3. The checklist

What is left needs a person at the keyboard. Each item says what to do, what is right, and what the
failure looks like — the last part matters most, because several of these fail in ways that look
like nothing happening.

Have a way out before starting: **Ctrl+Alt+Del** still works (by design — §12 of the study), so
Sign out and Switch user remain reachable if the overlay misbehaves.

> **Part of this has now happened in anger, and it earned its place.** Running the overlay with
> Hello and with voice enabled turned up two defects in 0.2.0 that the whole suite had missed: the
> screen stayed locked for seconds after Hello verified, because a worker put its answer on a
> channel while the UI thread waited on a *window message*; and a correctly recognised word was
> reported as the wrong word, because one outcome covered both "wrong word" and "not confident
> enough". Both are fixed, both now have tests, and neither was reachable without a person at the
> keyboard. The items below are annotated with what that established — and, carefully, with what it
> did not.

### Overlay, on mixed DPI

1. **Both monitors covered, and no prompt yet.** Lock. Check both screens edge to edge, including
   under the taskbar. *Failure:* any desktop visible at an edge, or one monitor left entirely
   uncovered.
   > **Also the wake behaviour, which 0.2.5 fixes.** Nothing should be asked of you until you press
   > a key or click: the overlay goes up bare, and the first input brings the prompt and Windows
   > Hello with it. The pointer should be an ordinary arrow throughout -- an arrow with an
   > hourglass means a window class lost its cursor again, and on a lock screen that reads as
   > hung. Until 0.2.5 the prompt opened immediately, which meant Hello started as the
   > owner was walking away -- and on face-recognition hardware it could verify them and unlock the
   > screen a second after it locked. *Failure:* a password box, or a Hello prompt, before you
   > touched anything.
2. **Side by side with the .NET app, on the 150% monitor.** The C# is no longer in this tree, so
   get it from the tag first:

   ```bash
   git worktree add ../lock-screen-dotnet dotnet-final
   dotnet build ../lock-screen-dotnet/LockScreenApp/LockScreenApp.csproj
   ```

   Run it, look at `DISPLAY1`, then do the same with the port. *What this settles:* whether the .NET
   overlay is soft and stretched (the §2.2 prediction) or actually short of the edges (the study's
   original claim). Note which, and correct §2.2 or D2 accordingly. **This is the one item holding
   anything open** — it is the last unsettled claim in the study.
3. **The prompt lands on the right monitor and is the right size.** Move the pointer to each monitor
   in turn, lock, and look at where the prompt appears. *Failure:* a prompt on the wrong screen, or
   one two thirds or one and a half times the size it should be — that is the DPI mixture D2 is
   about, now on the port's side.
4. **A display change while locked.** Lock, then unplug the second monitor, then plug it back in.
   *Right:* the overlay rebuilds and still covers everything. *Failure:* a bare desktop where the
   monitor used to be, or an overlay left at the old size.
5. **Blur on and off.** Set `BlurBackground` both ways. These build the window two different ways
   (`win/backdrop.rs`), so both need looking at. *Failure:* with blur on, a window that replaces the
   desktop rather than blurring it; with blur off, an invisible overlay — the layered-window trap in
   `CLAUDE.md`.

### Windows Hello

6. **Biometric, then PIN.** Lock with Hello available. *Right:* the Hello prompt appears **above**
   the overlay and is focused. *Failure:* the prompt behind the overlay, or no prompt — this is
   Finding B, the one thing the port fixes that cannot be seen any other way.
   > **Observed on 0.2.0.** The prompt appeared, was answerable, and verified. It could not have
   > been answered from behind an always-on-top overlay, so Finding B's fix holds. What this did
   > *not* establish is which modality was offered, or item 7.
7. **Dismiss it.** *Right:* the password box, with Hello offered again. *Failure:* unlocking, or a
   dead prompt.
   > **And then cancel the whole prompt.** It should return to the bare locked overlay and stay
   > there until you press a key again. Until 0.2.6 it reopened immediately: the hooks counted
   > every keystroke as a request for a prompt, including the Escape that had just closed one, so
   > cancelling looked like it did nothing. *Failure:* a prompt that comes straight back, or an
   > overlay that stops responding to input afterwards.
8. **Hello unavailable.** Remove the PIN and biometrics for the account, or use a machine without
   them. *Right:* straight to the password, with no retry offered. *Failure:* a Hello button that
   does nothing, or a wait before the prompt appears.

### Voice

9. **On and recognised.** Enable voice, lock, say the word. *Failure:* no response, or a response to
   something that was not the word.
   > **Half observed on 0.2.0 and 0.2.3.** The recogniser ran, listened and returned the right
   > word, so the microphone path and the list constraint work end to end. It was then refused by
   > the confidence gate — at *low* confidence for a coined word — so **an accepted voice unlock
   > has still never happened**. That is the half this item is really about, and it needs either a
   > command the recogniser is sure of or Required match lowered.
10. **Somebody else says it.** *Right:* it unlocks — recognition is speaker-independent, which is
    why it is off by default and documented as a convenience rather than a factor. Confirm that is
    still understood rather than filed as a bug.
11. **No microphone.** Disable or unplug it. *Right:* the voice button reports that it is
    unavailable. *Failure:* a hang, or silence.

### Credentials

12. **The password rung.** A wrong password three times — the fourth attempt should be slowed
    (`SLOW_AFTER`). *Failure:* no slowing, or a lockout. Also the two length floors, which only the
    window shows: a 4-character unlock password is accepted, a 4-character master password is
    refused, and each message quotes **its own** number rather than one shared number. The rules
    themselves are unit-tested; that the right message reaches the right field is not.
13. **The master password is never offered on its own.** It has to be asked for. *Failure:* it being
    presented alongside the password, which would make it a second password rather than a recovery
    path.
14. **Settings authenticates first.** `--settings`, then a wrong password. *Failure:* a settings
    window appearing at all — that is D7, and the thing the mode exists to fix.

### Input suppression

15. **The suppression list.** With the overlay up: `Win`, `Alt+Tab`, `Alt+F4`, `Ctrl+Shift+Esc`,
    `Ctrl+Esc`, `Alt+Esc`. *Right:* nothing happens. The last two are gaps the .NET version has and
    the port closes, so they are the interesting ones.
16. **`Ctrl+Alt+Del` still works.** Not a defect; a Secure Attention Sequence no hook can intercept.
    Confirm it behaves as documented rather than appearing to be suppressed.
17. **Task Manager policy restored.** Unlock, then open Task Manager. *Right:* it opens. *Failure:*
    "disabled by your administrator" — the policy was left behind. Then the other half: set
    `DisableTaskMgr` yourself beforehand and confirm the overlay leaves it alone on exit.

### Migration from the .NET build

18. **A real `config.ini` from the .NET app.** Put one beside `LockScreen.exe`. *Right:* setup runs
    and says the old password cannot be carried over. *Then:* the appearance settings survived, the
    old password does not work, and the `Password` key is gone from the file. *Failure:* the overlay
    starting with no credential, or the DPAPI blob still in the file.
19. **Two instances.** Start the overlay twice. *Right:* the second exits quietly — the screen is
    already locked, which is what was wanted. *Failure:* two overlays, each needing its own unlock.
20. ~~**A read-only folder.**~~ **Closed in 0.2.4** — automated, see §1. The test denies itself
    write access to the program's folder and checks the refusal names it. Nothing left to do by
    hand.

### The updater

21. **Unreachable from the overlay.** Confirm there is no path to it while the screen is locked.
22. **An update, end to end.** There are seven releases now, so this is no longer the empty-list
    case. On the newest build, settings → check should say "this is the newest release". On the
    0.0.9 build in `lock-screen-update-test`, it should offer **the newest**, not the oldest —
    download, checksum, rename aside, and a prompt to restart by hand. *Then:* `--version` reports
    the new one and `LockScreen.old.exe` disappears on the next start. *Failure to watch for:* a
    leftover `.old.exe`, or an install that leaves no working program.
    > The empty-list case — `200` with `[]`, which must read as "nothing newer" and not as a failed
    > check — is pinned as a test case and can no longer be reproduced against the live forge.
23. **A GitHub copy updates from GitHub, and only from there.** Added 11 October 2026, when the
    repository gained its GitHub mirror; nothing below has been run yet. Download `LockScreen.exe`
    from a GitHub release, not the Forgejo one, and run settings → check. *Right:* the log line
    `checking for updates at https://api.github.com/repos/…` names GitHub, and a release that
    exists only on GitHub is offered. *Then* install it: GitHub answers asset downloads with a `302`
    to `objects.githubusercontent.com`, which the Forgejo never does, so this is the first time the
    client's redirect handling is exercised at all. *Failure to watch for:* "the forge answered 302",
    which would mean WinHTTP's default redirect policy is not what `win/http.rs` says it is.
24. **Capture GitHub's answer as a fixture.** Once two releases exist on GitHub, save the verbatim
    response of the `per_page=10` URL beside `forgejo-releases.json` and give it the same test. The
    unit test in `release.rs` was shaped from the documentation, which is the thing
    `forgejo_release_compat.rs` exists to distrust.

---

## 4. Not validated, and why

The honest gaps, so nobody reads §1 as more than it is.

| What | Why not | What it would take |
| --- | --- | --- |
| **Authenticode signing** | No certificate exists. Releases are unsigned, and the workflow says so twice in its log. The *mechanics* are no longer unverified: the signing step was run against a throwaway self-signed certificate, and the base64 round-trip, the signtool discovery, the command line and the DigiCert timestamp server all work. What is missing is only the certificate. | A certificate whose key CI can use — and that is narrower than it sounds. Since June 2023 the CA/Browser Forum requires code-signing keys to live in hardware, so no CA issues an exportable PFX for a publicly trusted certificate: the `CERT_PFX_BASE64` secret can only ever hold a self-signed or internal-CA one, which `signtool verify` correctly refuses. A trusted signature from CI needs a signing service that holds the key (Azure Trusted Signing, DigiCert KeyLocker, SSL.com eSigner) or a hardware token, which does not suit an unattended runner. That is a decision with a cost attached, not a code change. **Decided on 9 October 2026: stay unsigned for now.** The workflow is ready for whichever service is chosen later; revisit if the app is ever distributed beyond people who can check a hash. |
| **Windows 10** | No Windows 10 machine. The supported floor is build 17763 and the code paths are there, but the Win10 blur branch has never run. | A 1809-or-later machine, and items 1–5. |
| **Hardware without Hello, or without a microphone** | This machine has both. | Items 8 and 11 can be approximated by disabling them, which is not the same as absent hardware. |
| **An install that has actually been updated** | Seven releases are published now, so the obstacle is gone — but no copy has yet replaced itself. The download half *is* verified: every release's published `LockScreen.exe` was fetched and hashed against its own `SHA256SUMS.txt`, and matched. What has never run is the rename-aside install. | Item 22, on the 0.0.9 build left in `lock-screen-update-test` for exactly this. It must offer the newest release rather than merely "something newer", there being seven to choose between. |
| **Whether a Credential Provider is the goal** | Still open. The overlay is a deterrent, not a security boundary, in either language — §12 of the study. If an unbypassable lock is the real requirement, this port does not deliver it and never was going to. | A decision, not a test. |
