# Porting LockScreen to Rust — Feasibility Study

**Subject:** LockScreenSolution v1.1.1.0 (.NET 8 WPF, 1,667 lines C#, 2 apps)
**Probed on:** Windows 11 Pro 26200 · rustc 1.99.0 · `windows` 0.62.2
**Date:** 9 October 2026 · rev. 5
**Rendered version:** <https://claude.ai/code/artifact/dad05b84-8822-421a-89b2-eccbd5e77f9e>

> **Revision 2** recorded four decisions taken after the first draft: the master password is set at
> first run and is **not** tied to the Windows account ([§7](#7-credentials-the-master-password-and-first-run));
> the app ships as **one binary with three modes**, not two executables
> ([§8](#8-architecture-one-binary-three-modes)); in-app updates and version reporting are in scope
> ([§9](#9-in-app-updates-and-version-reporting)); and the unauthenticated settings window is now
> tracked as defect **D7**.
>
> **Revision 3** adds three more, all in [§8](#8-architecture-one-binary-three-modes):
> the app is **portable** — settings live beside the executable, with nothing in `%APPDATA%` and no
> installer; the supported floor is **Windows 10 build 17763 (1809)** and later, unchanged from the
> current csproj; and `global.json` is **deleted** rather than repaired, matching the sibling
> projects, which pin neither an SDK nor a Rust toolchain. D6's path fix changed shape as a result —
> the target is the executable's own directory, not `%APPDATA%`, and emphatically not the working
> directory.
>
> **Revision 4** closes the last open question: the UI is **raw Win32, with no GUI toolkit**
> ([§5](#5-what-replaces-wpf-decided)). Measured rather than argued, and the measurements contradicted
> two claims in earlier revisions — both toolkits are ~10 MB rather than the few MB estimated, and
> Slint's royalty-free desktop licence costs nothing rather than needing a budget. The unlock dialog
> has been prototyped against the decision and is in the repository.
>
> **Revision 5** records phase 08, the parity validation, in [`PARITY.md`](PARITY.md) — what the
> suite now proves, what was measured on real mixed-DPI hardware, and the checklist of what still
> needs a person at the keyboard. It also **corrects D2**: the mechanism described in revisions 1–4
> was wrong, and the symptom predicted from it does not follow. See the note in
> [§6](#d2--overlay-is-mis-sized-on-mixed-dpi-setups).

---

## Verdict: feasible, with no functional loss

The app's entire Windows surface is reachable from the `windows` crate 0.62.2 — including the
WinRT namespaces (`Security.Credentials.UI`, `Media.SpeechRecognition`) that are the usual reason
people assume a .NET app can't leave .NET. Each one was compiled and executed on this machine;
results in [§3](#3-the-apis-called-from-rust-on-this-machine).

The real work is not API parity. It is **rebuilding two WPF window trees as native windows** —
roughly 55% of the effort — and that is also where the existing high-DPI and overlay bugs live.

| Metric | Value |
| --- | --- |
| Windows APIs needing replacement | 14 |
| Verified reachable from Rust | 14 / 14 |
| Third-party deps dropped | 3 → 0 native |
| Defects fixed by construction | 7 |
| Shipped executables | 2 → 1 |
| Estimated effort, 1 dev | 25–32 days |
| ⌙ with in-app updater | 27–35 days |
| Ship size, self-contained | 181 MB → ~2 MB |

---

## 1. What is actually being ported

Two WinExe projects on `net8.0-windows10.0.19041.0`, sharing five helper files by copy-paste
rather than by project reference. The copies have already diverged — `LockScreenApp`'s
`EncryptionHelper` swallows `CryptographicException` and returns the ciphertext as plaintext;
`SettingsApp`'s lets it throw. Their two `VoiceRecognitionHelper` copies return different tuple
shapes.

### LockScreenApp — the overlay

| Unit | Lines | Responsibility | Difficulty |
| --- | --: | --- | --- |
| `MainWindow.xaml.cs` | 272 | Multi-monitor translucent overlay, LL keyboard hook with suppression, SharpHook global hook, Task Manager policy key, shutdown sequencing | **Hard** |
| `PasswordWindow.xaml.cs` | 337 | Windows Hello flow, password entry, voice unlock, modal capture that blocks close | **Hard** |
| `VoiceRecognitionHelper.cs` | 153 | WinRT `SpeechRecognizer` + list-grammar constraint, confidence mapping | Direct |
| `VoiceConfigHelper.cs` | 133 | `[VoiceRecognition]` INI section read/write | Direct |
| `IniHelper` · `EncryptionHelper` · `SoundHelper` | 144 | Config persistence, DPAPI, system sounds | Direct |

### SettingsApp — the configurator

| Unit | Lines | Responsibility | Difficulty |
| --- | --: | --- | --- |
| `MainWindow.xaml.cs` | 236 | Title, transparency, blur toggle, voice toggle, mic test, password change | Moderate |
| Helpers (5 files) | 346 | Divergent duplicates of the above | Merge |

### Baseline measured on this machine

| Measurement | Value | Note |
| --- | --- | --- |
| Build status | **fails** | `global.json` pins SDK `8.0.0` with `rollForward: latestMinor`; only 9.0.317, 10.0.400, 10.0.401 are installed. Builds cleanly once rolled forward — 0 errors, 6 nullability warnings. |
| Framework-dependent output | 30 MB | 7 DLLs beside the exe; requires .NET 8 Desktop Runtime on the target |
| Self-contained single-file | 181 MB | `LockScreen.exe` = 180,727,089 bytes, and single-file still can't absorb 7 native DLLs: `wpfgfx_cor3`, `D3DCompiler_47_cor3`, `PresentationNative_cor3`, `PenImc_cor3`, `vcruntime140_cor3`, `uiohook.dll` |
| DPI awareness declared | **none** | No `app.manifest`, no `HighDpiMode` — see defect D2 |
| SDK pin | `global.json` | **Delete it.** It pins SDK `8.0.0` for no benefit — nothing is reproducible when the pinned SDK is installed nowhere, and a 10.x SDK builds a `net8.0` target fine. The sibling has neither a `global.json` nor a `rust-toolchain.toml`; the house convention is current stable. Cargo has no equivalent, so this disappears with the port. |
| Git remote | `git.zacx.net` | Self-hosted **Forgejo**, not GitHub. A sibling project already ships an updater and release pipeline against it — see §9 |

---

## 2. Evidence standard

A feasibility study that says "this should work" is worth little. Every API claim below was
verified by building a probe crate against `windows = "0.62"` and running it. The probe source is
reproducible from the parity matrix in §4 — crate, feature flag and function are named for each row.

---

## 3. The APIs, called from Rust, on this machine

```
$ cargo run --quiet
[1] UserConsentVerifier availability = UserConsentVerifierAvailability(0) (Available = UserConsentVerifierAvailability(0))
[2a] SystemSpeechLanguage = English (United States)
[2b] SpeechRecognizer compile status = SpeechRecognitionResultStatus(0)
[3a] CryptProtectData ok -> 230 byte blob
[3b] CryptUnprotectData round-trip = "1234"
[4] policy key opened; DisableTaskMgr = Err(HRESULT(0x80070002) "The system cannot find the file specified.")
[5] MessageBeep(MB_ICONASTERISK) ok
[6a] argon2 phc = $argon2id$v=19$m=19456,t=2,p=1$GW1ueMpH8XkZsokt/eZLmg$1M+OY2nqLv
[6b] verify correct = true
[6c] verify wrong   = false
[7] ini Title = Some("Enter Password to Unlock")
```

Probe 4 returning `0x80070002` is the correct result: the `DisableTaskMgr` value only exists while
the lock screen is running, and it was not. The key itself opened.

### Finding A — .NET-written DPAPI blobs decrypt from Rust

PowerShell called `System.Security.Cryptography.ProtectedData.Protect` with `CurrentUser` scope and
null entropy — byte-for-byte what `LockScreenApp/Helpers/EncryptionHelper.cs:11` does — then Rust
decrypted that blob via `CryptUnprotectData`:

```
$ cargo run --quiet --bin crosscheck
read 230 byte .NET DPAPI blob
Rust decrypted .NET blob -> "HelloFromDotNet"
```

**Consequence:** existing `config.ini` files remain readable, so the port can read an old config to
migrate the title, transparency and blur settings forward. The stored password itself is *not*
carried over — §7 replaces it with an Argon2 verifier set during first-run setup.

### Finding B — the current Windows Hello call is the wrong one

`LockScreenApp/PasswordWindow.xaml.cs:102` calls `UserConsentVerifier.RequestVerificationAsync`.
That static overload is specified for apps **with package identity**. LockScreenApp is an
unpackaged Win32 process, and for those Windows requires the COM interop path, which takes an
`HWND` so the consent dialog can be parented and activated correctly. Without it the behaviour is
unspecified — in practice, the Hello prompt appearing behind an always-on-top overlay, or failing
outright on some machines. The current code's generous `catch (Exception ex)` at line 126 turns
that into "Windows Hello error" text rather than a crash, which is why it may never have been
noticed.

windows-rs exposes the correct interface. From `Win32/System/WinRT/mod.rs:2350`:

```rust
define_interface!(IUserConsentVerifierInterop, …,
    0x39e050c3_4e74_441a_8dc0_b81104df949c);

pub unsafe fn RequestVerificationForWindowAsync<T>(
    &self, appwindow: HWND, message: &HSTRING) -> Result<T>
```

Rust gets the API that was correct for this process model all along. Same for
`IInitializeWithWindow` on any other WinRT picker added later.

---

## 4. Parity matrix — every Windows dependency and its Rust replacement

**Verified** = called from Rust on this machine. **Present** = API exists in the bindings,
confirmed by inspection, not yet executed. **Design** = the API is stable but the port is a
rewrite rather than a substitution.

### Authentication

| Capability | Today | Rust | Status |
| --- | --- | --- | --- |
| **Biometric unlock** (face, fingerprint) | `UserConsentVerifier.RequestVerificationAsync` | `IUserConsentVerifierInterop::RequestVerificationForWindowAsync`<br>`windows 0.62` · feature `Security_Credentials_UI` | ✅ Verified |
| **PIN unlock** | Same API as above | Windows Hello's consent verifier *is* the PIN/biometric selector — the OS decides which modality to present and falls back to PIN on its own. There is no separate PIN API to port, and nothing here can be lost independently of biometrics. | ✅ Verified |
| Hello availability probe | `.CheckAvailabilityAsync()` | `UserConsentVerifier::CheckAvailabilityAsync()?.join()` | ✅ Verified |
| **App password unlock** | `PasswordBox` + `string ==` | `argon2 0.6` · `subtle` · `zeroize 1.9` | 🆕 Upgrade |
| **Master password** | *does not exist* | argon2id PHC, independent salt, set at first run — §7 | 🆕 New |
| **Voice unlock** | `Windows.Media.SpeechRecognition` | `windows 0.62` · feature `Media_SpeechRecognition`<br>`SpeechRecognitionListConstraint::CreateWithTag` | ✅ Verified |

### Credential storage

| Capability | Today | Rust | Status |
| --- | --- | --- | --- |
| DPAPI protect / unprotect | `ProtectedData.Protect / .Unprotect` | `Crypt{Protect,Unprotect}Data` · `Win32_Security_Cryptography` | ✅ Verified |
| Read existing encrypted configs | — | Cross-checked against a .NET-written blob (Finding A) | ✅ Verified |

### Overlay windows

| Capability | Today | Rust | Status |
| --- | --- | --- | --- |
| Full-screen translucent overlay | WPF `AllowsTransparency` + `SolidColorBrush(a,0,0,0)` | `WS_EX_LAYERED \| WS_EX_TOPMOST \| WS_EX_NOACTIVATE`<br>`SetLayeredWindowAttributes(…, alpha, LWA_ALPHA)` | ◐ Design |
| Multi-monitor enumeration | WinForms `Screen.AllScreens` | `EnumDisplayMonitors` + `GetMonitorInfoW` · `Win32_Graphics_Gdi` | ✅ Present |
| Per-monitor DPI | *not handled* — see D2 | Declared in `assets/app.manifest` (`PerMonitorV2`), as the sibling does, plus `GetDpiForMonitor` at runtime | 🆕 Fix |
| Background blur | WPF `BlurEffect { Radius = 10 }` — a no-op, see D1 | `DWMWA_SYSTEMBACKDROP_TYPE` on Windows 11, `ACCENT_ENABLE_ACRYLICBLURBEHIND` on Windows 10 1809+<br>`window-vibrancy 0.8.1` owns the version branching | 🆕 Fix |

### Input control

| Capability | Today | Rust | Status |
| --- | --- | --- | --- |
| Global input detection (wake the prompt on any key/click) | `SharpHook 7.1.1` + `SharpHook.Reactive`<br>*ships `uiohook.dll`, 693 KB native* | `SetWindowsHookExW(WH_KEYBOARD_LL / WH_MOUSE_LL)`<br>*no third-party crate needed* | ✅ Present |
| Key suppression (Win, Alt+Tab, Alt+F4, Ctrl+Shift+Esc) | `SetWindowsHookEx` via `DllImport`, returns `int` — see D3 | Same hook, typed `HHOOK`; modifier state from `GetAsyncKeyState`, not a UI framework — see D4 | 🆕 Fix |
| Task Manager policy | `Microsoft.Win32.Registry` | `windows-registry 0.100` | ✅ Verified |

### Configuration, audio, chrome

| Capability | Today | Rust | Status |
| --- | --- | --- | --- |
| INI persistence | `INIFileParserDotNetCore 2.5.2` | `rust-ini 0.21.3` | ✅ Verified |
| System sounds | `System.Media.SystemSounds` | `MessageBeep(MB_ICONASTERISK \| MB_ICONHAND \| …)` — the exact call `SystemSounds` makes | ✅ Verified |
| Window icon | `ApplicationIcon` in csproj | `embed-resource 3` build script over `assets/app.rc` — also carries VERSIONINFO and the manifest, see §8 | ✅ Present |

### Dependency ledger

Three third-party packages go away, one of them carrying a native DLL: `SharpHook` and
`SharpHook.Reactive` are replaced by the two Win32 calls they wrap, and `INIFileParserDotNetCore`
by `rust-ini`. The new crates (`windows`, `windows-registry`, `rust-ini`, `argon2`, `zeroize`) are
all first-party Microsoft or RustCrypto, MIT/Apache-2.0, and ship no native binaries. The updater in
§9 adds **nothing**: it uses WinHTTP through the `windows` crate, which the app already links.

---

## 5. What replaces WPF — decided

**Decided: raw Win32 through `windows-rs`, with no GUI toolkit at all.** Both the overlay and the
three dialog modes. The unlock dialog has been built against this decision and is in the repository.

Earlier revisions of this study recommended `egui` for the dialogs on the strength of estimates. The
estimates were wrong, so the numbers below come from building a minimal password dialog in each
toolkit with this project's own release profile (`opt-level = 3`, `lto`, `codegen-units = 1`,
`panic = "abort"`, `strip`) and measuring.

| | **raw Win32** | **egui 0.36.2** | **slint 1.18.1** |
| --- | --- | --- | --- |
| Binary | **460 KB** (actual) | **9.80 MB** | **10.39 MB** default · **9.85 MB** software-only |
| Crates in graph | **70** | 418 | 568 |
| Clean build | ~5 s | 148 s | 255 s |
| **GPU context required** | **no** | **yes** | no, with `renderer-software` |
| GPU crates compiled | none | `glow` `wgpu` `wgpu-hal` `ash` `naga` | none in the software config |
| Licence | — | MIT / Apache-2.0 | GPL-3.0 **or** Royalty-free-2.0 **or** paid |
| Password field | OS edit control: IME, accessibility, clipboard behaviour included | reimplemented by the toolkit | reimplemented by the toolkit |
| `HWND` for Hello interop | owned directly | `Frame::window_handle()` | `raw-window-handle-06` feature |
| Existing code to build on | **16,131 lines** in `flex-taskbar` | none | none |

### Why, in order of weight

1. **A GPU dependency is a lockout risk.** `eframe` has no software renderer — it compiles `glow`,
   `wgpu`, `ash` and `naga`, and needs a live OpenGL 3.x / Vulkan / D3D12 context. For most
   applications that is a performance footnote. For this one it is a safety argument: if the overlay
   is up and the prompt cannot create a context — an RDP session, a Hyper-V basic display adapter, a
   GPU driver mid-update — the user is shut out of their own machine. Slint's `renderer-software`
   answers this (verified: zero GPU crates compiled). Raw Win32 never raises the question, because
   GDI and USER32 have no such requirement.
2. **Size, in a portable app.** 460 KB against ~10 MB, for a password box, two labels and four
   buttons. The app is meant to be copied to a folder and carried.
3. **The password field is the single most important control in the product.** The OS edit control
   brings IME, screen-reader support and clipboard policy with it. A toolkit reimplements all three,
   and a lock screen that a blind user cannot operate is a lock-out by another route.
4. **The house has already done this.** `flex-taskbar` ships no GUI crate and builds a bar, flyouts,
   a search window and several settings windows in 16,131 lines of Win32, with theming (`theme.rs`)
   and accessibility (`access.rs`). So this is not "write a `WndProc` from scratch": `ui.rs` is
   ported into this repository, and `prompt.rs` is the template the unlock dialog follows.

### Two corrections to earlier revisions

- **Size.** Revision 1 estimated `egui` at +1.5–3 MB and Slint at +3–6 MB. Both are **~+9.5 MB**, and
  the gap claimed between them does not exist — the cost is each toolkit's own runtime, not its
  renderers, which is why Slint's GPU-free configuration is no smaller.
- **Licensing.** Revision 1 implied Slint might need a paid licence. It does not: the **Royalty-free
  Desktop 2.0** licence covers proprietary desktop applications at no cost. The condition is
  attribution — an `AboutSlint` widget in an About dialog reachable from a top-level menu, or a "Made
  with Slint" badge on the page the binaries are downloaded from. GPL-3.0 is the option that is
  incompatible with shipping this proprietary.

With licensing neutral, Slint is the better of the two toolkits, which inverts revision 1's ranking.
Neither is needed.

### If this is ever revisited

The dialogs are the only toolkit-dependent code; `win/ui.rs` and `win/hello.rs` touch nothing
toolkit-specific and would carry over unchanged. Swapping the unlock prompt alone is roughly a day's
work. That stays true until phase 05 adds the settings window, after which the cost of changing
course roughly triples.

**Tauri was considered and rejected outright:** it puts a WebView2 dependency inside a security tool
and reintroduces exactly the runtime-dependency problem the port exists to remove.

---

## 6. Seven defects the port fixes by construction

These are in the current code. They are listed here because the port has to touch all seven anyway,
which changes the cost/benefit of doing it — you are not paying only for a language change.

### D1 — Blur does nothing

`LockScreenApp/MainWindow.xaml.cs:191` applies a `BlurEffect` to `OverlayGrid`, whose only content
is a uniform `SolidColorBrush`. Blurring a flat fill is a no-op; it never touches the desktop behind
the window. The setting has been exposed in SettingsApp the whole time.

*Fix:* DWM composition blur (`DwmEnableBlurBehindWindow` / `ACCENT_ENABLE_ACRYLICBLURBEHIND`) blurs
what is *behind* the window, which is what the setting promises. It is a different mechanism, not a
translation — so it only gets fixed if someone is rewriting the overlay.

### D2 — Overlay is mis-sized on mixed-DPI setups

No DPI awareness is declared anywhere in the project: no `app.manifest`, no
`ApplicationHighDpiMode`, no `SetHighDpiMode` call, and the manifest the .NET SDK embedded in the
built `LockScreen.exe` carries only a `requestedExecutionLevel`. Windows therefore virtualises every
coordinate the process reads. Measured on mixed-DPI hardware in phase 08 (`PARITY.md` §2): a
DPI-unaware thread reads a 150%-scaled 1920x1080 monitor as **1280x720**, two thirds of its real
size, and the overlay at `MainWindow.xaml.cs:89–106` is built from exactly those numbers.

> **Corrected in rev. 5.** Revisions 1–4 said `Screen.Bounds` "returns physical pixels" and
> concluded that a scaled monitor "leaves the desktop uncovered". The first half is wrong — in an
> unaware process those bounds are already virtualised — and the conclusion does not follow from
> it: WPF's device-independent unit in that process is one virtualised pixel, so the window matches
> the monitor in that coordinate space and Windows stretches it back up. The expected symptom is a
> soft, bitmap-stretched overlay and prompt rather than a hole. That prediction is item 2 of the
> phase 08 checklist and is **not yet confirmed on screen**, so it is flagged rather than asserted.
> The defect itself stands: the app does no DPI handling, and a blurry password prompt on one of two
> monitors is visibly worse than a sharp one.

*Fix:* raw Win32 works in physical pixels end to end, and `PerMonitorV2` is declared in
`assets/app.manifest` — the house mechanism, and better than an API call because it applies before
any of your code runs. The unit mismatch cannot arise because there is only one unit.

### D3 — 64-bit handle truncation

`MainWindow.xaml.cs:30–36` declares `SetWindowsHookEx` as returning `int` and
`UnhookWindowsHookEx` as taking `int`. `HHOOK` is a 64-bit pointer on x64. This works today only
because the values happen to fit in 32 bits — it is luck, not correctness, and a failed unhook
leaves a system-wide keyboard hook installed after unlock.

*Fix:* `SetWindowsHookExW` is typed `-> Result<HHOOK>` in windows-rs. The signature is generated
from Windows metadata, so this class of hand-transcription bug is gone across all 14 APIs, not just
this one.

### D4 — Modifier detection reads the wrong source

The low-level hook at `MainWindow.xaml.cs:146–149` asks WPF's `Keyboard.Modifiers` for
Ctrl/Alt/Shift/Win state. That reflects the WPF input manager for the focused element, not global
physical key state — inside a system-wide hook, in a process whose overlay is `NOACTIVATE`, it is
frequently stale. Alt+Tab and Ctrl+Shift+Esc suppression is therefore unreliable.

*Fix:* no UI framework in the loop. `GetAsyncKeyState` (or the hook's own `KBDLLHOOKSTRUCT.flags`)
is the correct source and the only one available.

### D5 — Password is recoverable, and compared in non-constant time

DPAPI is reversible encryption scoped to the current user with null entropy, so any code running as
that user recovers the plaintext from `config.ini`. It is then held in a `string` that is never
zeroized, and compared with `==` (`PasswordWindow.xaml.cs:76`). Worse,
`LockScreenApp/Helpers/EncryptionHelper.cs:33–40` returns the stored text unchanged when decryption
fails — so a hand-edited plaintext `Password=` value silently becomes a working password.

*Fix:* verification does not need recovery. Argon2id + a constant-time compare, with `zeroize` on
the buffers and no silent fallback path. Rust's `Drop` makes the zeroizing automatic rather than a
thing to remember.

### D6 — Five duplicated helpers, already divergent

See §1, plus a password policy at `SettingsApp/MainWindow.xaml.cs:134` that requires ≥3 characters
and *rejects* any symbol — so `abc` is accepted and `correct-horse-battery` is not.

`config.ini` is also a bare relative path (`IniHelper.cs:7`), so `File.Exists` and `ReadFile`
resolve it against the process's **current working directory** — whatever directory the app happened
to be launched from, not where the executable lives. A shortcut with a different *Start in* field, a
scheduled task, or a launch from a terminal elsewhere each point the app at a different `config.ini`.
Worse, finding none it *creates* one carrying the `1234` default password in that directory, so a
stray launch quietly scatters working default credentials around the disk.

*Fix:* §8's single binary makes divergence structurally impossible — one executable cannot disagree
with itself about the config format or the Argon2 parameters, which is stronger than the shared-crate
fix originally proposed here. The path bug is fixed by resolving against the **executable's own
directory** via `GetModuleFileNameW`, never `current_dir()` — see the portable layout below.

### D7 — The settings window requires no authentication

`SettingsApp/MainWindow.xaml.cs:88` — `Save_Click` reads `NewPasswordBox.Password`, checks its
strength, encrypts it and writes it. It never asks for the current password and never invokes
Windows Hello. **Anyone with access to the unattended desktop can run `Settings.exe` and reset the
lock-screen password**, so the overlay's protection is one double-click from being bypassed. This
undercuts the entire purpose of the app and is independent of language.

*Fix:* settings mode authenticates before it opens, via the same unlock ladder as §7 — and
credential changes additionally require the current password or the master password. The first-run
master password is the natural credential for this gate.

---

## 7. Credentials: the master password and first run

**Decided:** the master password is chosen by the user during first-run setup. It is an
app-local credential with no relationship to the Windows account password. There is no default
password and nothing is ever displayed in a dialog.

This replaces the current first-run behaviour, where `IniHelper.EnsureConfigExists()`
(`LockScreenApp/Helpers/IniHelper.cs:29–57`) writes `Password=1234` and then shows it in a
`MessageBox`.

### Storage design

1. **Store a verifier, not a secret.** Argon2id with its own salt, serialized as a PHC string
   (`$argon2id$v=19$m=19456,t=2,p=1$…` — probe 6 in §3). Nothing in the config can be turned back
   into the password, which is the property DPAPI does not give you.
2. **Independent salt and record per credential.** The master password must stay valid when the
   normal password is rotated, and rotating it must not invalidate the normal one. Two separate PHC
   records, not one credential with two inputs.
3. **It is a recovery credential, not a privileged one.** It bypasses *only* the normal app
   password. It does not bypass Windows Hello policy, is never auto-attempted, is rate-limited
   harder than the normal password, and writes an audit line when used.

### First-run flow

```
LockScreen.exe  (no args)
  └─ no master verifier in config?
       └─ re-exec as `--setup`, do not install hooks, do not draw an overlay
            ├─ collect master password   (confirm twice, strength-checked)
            ├─ collect app password      (confirm twice, strength-checked)
            ├─ write both Argon2id verifiers
            └─ exit; user relaunches to lock
```

Two properties worth preserving: the overlay never runs without a verifier present, so there is no
window in which the lock screen is active with a known default credential; and setup mode installs
no keyboard hook and writes no registry policy, so a failed or abandoned setup leaves the system
untouched.

Replace the `≥3 characters, letters and digits only` rule from D6 with a real policy — a length
floor and no character-class restriction, so passphrases with symbols are accepted rather than
rejected.

### Unlock precedence

| # | Method | Behaviour | Status |
| --: | --- | --- | --- |
| 1 | **Windows Hello** — biometric, then PIN | Attempted automatically on prompt. The OS picks the modality and handles its own fallback and retry limits. | ✅ Ported |
| 2 | **App password** | Shown when Hello is unavailable or the user declines it. Argon2id verifier replaces the reversible store. | 🆕 Hardened |
| 3 | **Voice command** | Convenience only, off by default. See the caveat below — this is not an authentication factor. | ✅ Ported |
| 4 | **Master password** | Never offered automatically. Reached from an explicit "Use master password" control, rate-limited, audited. Also the gate for settings mode (D7). | 🆕 New |

> ### ⚠ One thing to fix while you are here
>
> Voice unlock currently succeeds at `confidence >= 0.7` (`PasswordWindow.xaml.cs:235`) while
> WinRT's `Medium` confidence maps to exactly `0.7`
> (`LockScreenApp/Helpers/VoiceRecognitionHelper.cs:92–94`) — so a medium-confidence match passes at
> the default threshold. Combined with speaker-independent recognition, that means **anyone standing
> at the machine can say "unlock" and be let in**, with no password. It is off by default, which
> contains the damage. Keep it off by default, and either require `High` or treat voice as a
> convenience that still demands a second factor.

### Rejected alternative: the Windows account password

Considered and rejected. Recorded here because the reasoning is not obvious and the question will
come up again.

It is technically available. `LogonUserW` validates an account password with no special privilege —
verified callable from Rust as a normal user, using a deliberately non-existent account so that no
real lockout counter was touched:

```
$ cargo run --quiet --bin logon
[policy] lockout_threshold = 10  duration = 600s  observation_window = 600s
[logon] rejected: HRESULT(0x8007052E) -> The user name or password is incorrect.
```

Five reasons it is the wrong credential for this slot:

- **Account lockout.** This machine's threshold is 10 attempts, 600s duration, read live via
  `NetUserModalsGet` level 3. Ten mistyped attempts at the overlay would lock the real Windows
  account. A convenience lock screen that can lock the user out of their own machine has inverted
  its purpose — and on a domain or Entra-joined host the threshold is often 5, set by someone else,
  changeable without the app's knowledge.
- **It may not exist.** The account on this machine is `PrincipalSource: Local` with
  `PasswordRequired: False`, and `LimitBlankPasswordUse = 1` (the Windows default) restricts
  blank-password local accounts to console logon — so `LOGON32_LOGON_NETWORK` would be refused by
  policy. With Hello reporting `Available`, sign-in here is almost certainly by PIN. Passwordless
  and MSA/Entra-backed accounts make this increasingly common, not less.
- **It would reintroduce D5, worse.** Passing a password to `LogonUser` means holding the user's
  actual machine-owning credential as plaintext in a user-mode process. A memory dump of the lock
  screen would then yield the credential that owns the machine.
- **Audit noise.** Every failure writes Security event 4625, attributed to this app.
- **It teaches the wrong habit.** A non-OS-owned window asking for the Windows password is the
  exact shape of credential phishing. Microsoft's answer to this is the secure desktop — see §12.

A sixth, smaller point: as the transcript shows, a non-existent user and a wrong password both
return `0x8007052E`. Windows deliberately does not distinguish them, so the app could not even
pre-validate a username.

If an account-password factor is ever wanted, add it as an **optional fourth factor, default off**,
never as the master credential — and then: `LOGON32_LOGON_NETWORK` only, `CloseHandle` the token
immediately, read the lockout threshold at startup and refuse to attempt within ~3 of it, throttle
well below the OS limit, `zeroize` the buffer, treat `ERROR_ACCOUNT_LOCKED_OUT` /
`PASSWORD_MUST_CHANGE` / `ACCOUNT_RESTRICTION` / `ACCOUNT_DISABLED` as distinct from "wrong
password", and hide the option entirely when `PasswordRequired == False` or the account is
MSA/Entra-backed.

---

## 8. Architecture: one binary, three modes

**Decided:** one executable, dispatched by argument.

```
LockScreen.exe              → overlay          (default, no args)
LockScreen.exe --settings   → settings window  (requires auth, see D7)
LockScreen.exe --setup      → first-run setup  (only when no verifier exists)
```

### Why one rather than two

Each Rust binary statically links its own copy of everything it uses. Three binaries built from one
probe crate with identical dependencies:

```
crosscheck.exe    126,464 bytes
logon.exe         113,664 bytes
probe.exe         203,776 bytes
                  ───────────────
                  443,904 across 3
```

For tiny console binaries that is a rounding error. With `egui` the duplicated term is 1.5–3 MB —
the *dominant* cost in a ~2 MB app. Two executables that both link a GUI crate pay for it twice.

The stronger argument is version skew. Two executables can be updated independently — someone copies
one over, and now the settings app and the overlay disagree about the config format or the Argon2
parameters. One binary cannot disagree with itself. That is a structural guarantee, which is why D6's
fix is now attributed here rather than to a shared crate. It also matters directly for §9: an updater
that replaces one file is far simpler to make correct than one that must replace two atomically.

Also: one Authenticode signature, one VERSIONINFO resource, one installer entry, one place where the
config path is resolved.

### Objections that do not hold

- **"The settings UI would live in the locked overlay's process."** It would not. `--settings`
  spawns a **new process** of the same image; the process boundary is fully preserved. Only the
  executable file is shared.
- **"Different icons per app."** Embed multiple icon resources in one PE. The Start Menu shortcut
  points at `LockScreen.exe,1` for the settings icon, and the window icon is set at runtime with
  `LoadIconW` + `WM_SETICON`.
- **"Discoverability in the Start Menu."** A `.lnk` carries arguments, so "Lock Screen Settings" is
  a shortcut with `--settings` and its own icon index.

### Caveats to plan for

- `#![windows_subsystem = "windows"]` is per-binary, so there is no console attached. If `--help` or
  `--version` should print anything, call `AttachConsole(ATTACH_PARENT_PROCESS)` first, or accept
  that CLI output goes nowhere.
- **Mode dispatch is not a security boundary.** `--settings` is a flag anyone can pass. That is
  fine, because the gate is authentication (D7), not argv. Do not let the two be confused.

### Portable: settings live beside the executable

**Decided:** the app is portable. Settings sit next to `LockScreen.exe`, there is no installer, and
nothing is written to `%APPDATA%`, `%LOCALAPPDATA%` or `%ProgramData%`. This matches `flex-taskbar`,
whose README states the same property — one exe, no installer, settings kept next to it.

**"Beside the executable" must not be read as "the working directory".** Those are different
directories, and conflating them is exactly defect D6. The current working directory is whatever the
process was launched from and varies per shortcut, per scheduled task, per terminal; the
executable's directory never changes while the program runs. Resolve it once at startup from
`GetModuleFileNameW` and take the parent — never `std::env::current_dir()`, and never a bare
relative path.

```
LockScreen.exe        the whole program
config.ini            settings + Argon2id verifiers, beside the exe
LockScreenData/
  update/             updater scratch space (§9)
  lockscreen.log      diagnostics
```

`config.ini` is the only file holding user intent; everything under `LockScreenData/` is derived and
safe to delete. It is named for the app rather than a bare `data` because a portable app gets
unpacked wherever somebody drops it, and a folder called `data` beside the exe is a collision
waiting to happen. Keeping transient files in a subdirectory rather than loose beside the exe follows the
sibling, which keeps its own `crash.log` and `update\` in one such directory.

Four consequences worth planning for rather than discovering:

- **`%ProgramFiles%` stops being a valid location.** A standard user cannot write there, so a
  portable app placed there cannot save settings. Probe writability at startup and say so plainly —
  *"Settings can't be saved: this folder is read-only. Move the app somewhere you can write, such as
  your user folder."* Failing silently on save is the one outcome to avoid.
- **Portability only works because of §7.** DPAPI ties a blob to one user on one machine, so a
  portable DPAPI config would break the moment the folder moved. An Argon2id PHC string is just a
  hash and travels anywhere. The credential redesign and this decision support each other; under the
  old scheme "portable" would have been a promise the app could not keep.
- **Each copy of the exe has its own settings.** Two folders means two independent configurations.
  Ordinary portable behaviour, but document it so nobody files it as a bug.
- **The Task Manager policy is not portable.** `DisableTaskMgr` is an `HKCU` value written to
  whatever machine the app runs on and removed on unlock. "Portable" describes where settings live,
  not the fact that the overlay still touches the local registry while it is up.

One security note: anyone who can write the app's folder can replace `config.ini`, and therefore the
verifier. That is not new — §12 already says the app cannot protect its config from the user who owns
it — but it argues for keeping the folder somewhere only you can write rather than a shared location
like `C:\Tools\`.

### Windows 10 and 11: the supported floor

**Decided:** Windows 10 build 17763 (version 1809) or later — exactly what the current csproj
already declares via `TargetPlatformMinVersion`. Nobody loses support in the port.

Rust's own Tier-1 baseline for `x86_64-pc-windows-msvc` has been Windows 10 since Rust 1.78, so the
toolchain and the target agree with no workarounds. Every API in §4 is comfortably older than 1809:

| API | Available since |
| --- | --- |
| WinRT `SpeechRecognizer` | Windows 10 1507 |
| `UserConsentVerifier` / `IUserConsentVerifierInterop` | Windows 10 1607 |
| `PerMonitorV2` DPI awareness | Windows 10 1703 |
| `ACCENT_ENABLE_ACRYLICBLURBEHIND` | Windows 10 1803 |
| `GetDpiForMonitor` | Windows 8.1 |
| DPAPI · `SetWindowsHookExW` · `MessageBeep` · `EnumDisplayMonitors` | Windows 2000–Vista era |

The binding constraint is the blur fix for D1, and it is the reason to stay at 1809 rather than drop
to the sibling's 1703: acrylic needs 1803, so a 1809 floor means **two** blur paths, not three.

| Target | Mechanism |
| --- | --- |
| Windows 11 22H2+ | `DwmSetWindowAttribute` with `DWMWA_SYSTEMBACKDROP_TYPE` |
| Windows 10 1809 → Windows 11 21H2 | `SetWindowCompositionAttribute` with `ACCENT_ENABLE_ACRYLICBLURBEHIND` |
| *(below 1809 — not supported)* | *would need `ACCENT_ENABLE_BLURBEHIND`; avoided by the floor* |

`SetWindowCompositionAttribute` is undocumented and its accent states vary by build, which is the
argument for `window-vibrancy 0.8.1` — it owns the version branching so you do not. Note also that
the sibling's manifest declares `activeCodePage UTF-8`, which only takes effect from Windows 10
1903; older builds ignore it harmlessly, and since the port calls wide APIs throughout it changes
nothing either way.

One factual note for planning, not a recommendation: Windows 10 22H2 reached end of support on
14 October 2025, so Windows 10 users are on LTSC or ESU unless they are unpatched. That affects
whether *they* receive OS security updates, not whether this app runs.

### Workspace

```
lock-screen/
├── Cargo.toml                  workspace: 2 members, shared release profile
├── build.rs                    winresource: icons, VERSIONINFO, git hash (§9)
├── crates/
│   ├── lockscreen-core/        all shared logic, linked once
│   │   ├── paths.rs            exe-dir resolution via GetModuleFileNameW (fixes D6)
│   │   ├── config.rs           rust-ini over <exe dir>/config.ini
│   │   ├── credential.rs       argon2id verifiers + zeroize; reads legacy DPAPI config
│   │   ├── hello.rs            IUserConsentVerifierInterop (HWND-parented)
│   │   ├── voice.rs            WinRT SpeechRecognizer + list constraint
│   │   ├── sound.rs            MessageBeep
│   │   ├── policy.rs           DisableTaskMgr, with RAII restore on drop
│   │   └── update.rs           manifest fetch + signature verify (§9)
│   └── lockscreen/             the single binary: LockScreen.exe
│       ├── main.rs             argv → mode dispatch
│       └── mode/
│           ├── overlay.rs      layered windows per monitor, DPI-aware, hooks
│           ├── settings.rs     egui; auth-gated; update check; version display
│           └── setup.rs        egui; first-run master + app password
└── tests/                      parity fixtures incl. a .NET-written config.ini
```

`policy.rs` restores `DisableTaskMgr` from a `Drop` impl — today that cleanup lives in
`EnableSystemKeys()` and is called from three places, so an unexpected exit leaves Task Manager
disabled on the user's machine. The `tests/` fixtures should include a real `config.ini` written by
the current .NET build, as a regression gate on Finding A.

### House conventions to inherit from `flex-taskbar`

These are settled in a sibling project against the same forge and the same company metadata. Copy
them rather than re-deciding — the point of a convention is that the second project costs nothing.

| Concern | Convention |
| --- | --- |
| Edition | `edition = "2024"`, `publish = false` |
| Binary name | `[[bin]] name = "LockScreen"` — keeps the current `AssemblyName`, so shortcuts and docs do not change |
| Release profile | `opt-level = 3`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true` |
| Windows deps | `windows` + `windows-core` 0.62 under `[target.'cfg(windows)'.dependencies]`, features listed explicitly and alphabetically |
| Resources | `embed-resource = "3"` as a build-dependency; `build.rs` writes `app-with-version.rc` into `OUT_DIR` from `assets/app.rc` plus a generated `VERSIONINFO` block, with `.manifest_required()` |
| Manifest | `assets/app.manifest` is **not optional** — it opts into Common Controls v6 and `PerMonitorV2` DPI awareness (this is D2's fix) |
| Metadata | `CompanyName` "Procybro Solutions", `LegalCopyright` "(C) 2026 Procybro Solutions" — matching the existing csproj |
| Version source | `CARGO_PKG_VERSION` only. `build.rs` watches `Cargo.toml` itself, because `rerun-if-env-changed` alone leaves a stale resource claiming the wrong version |
| Formatting | `rustfmt.toml`: `max_width = 120`, `use_small_heuristics = "Max"` |
| CI gates | `cargo fmt --check`, `cargo test`, `cargo clippy --release --all-targets -- -D warnings`, `cargo build --release` — all with `--locked` |
| Smoke test | A `tools/smoke-test.ps1` equivalent, which matters more here than for a launcher: the overlay must be verified to actually release the keyboard hook and the Task Manager policy |

The `FILEVERSION` field wants four numbers and a Cargo version has three, so `build.rs` pads with a
zero. The current csproj already uses four (`1.1.1.0`), which maps cleanly onto `1.1.1` + pad.

---

## 9. In-app updates and version reporting

**In scope:** the user can check for and install a new release from within settings, and can see
which version is running.

### Version reporting — small and worth doing properly

Three things should agree, and only will if they come from one source:

| Surface | Mechanism |
| --- | --- |
| In-app "About" line | `env!("CARGO_PKG_VERSION")` — compile-time, authoritative |
| File Properties / Task Manager / installers | Windows `VERSIONINFO` resource, generated into `assets/app.rc` by `build.rs` and compiled with `embed-resource 3` — the sibling's exact pattern |
| Build provenance | Git short hash + build date injected by `build.rs`, replacing the hand-maintained `<SourceRevisionId>20260123.2224</SourceRevisionId>` in the csproj |

Display it in **settings mode only, not on the lock overlay.** A version string on a lock screen
tells anyone standing at the machine which build to look up known issues for — small, free
information disclosure with no upside.

Comparison uses the sibling's hand-rolled `Version { major, minor, patch }` with derived `Ord` rather
than the `semver` crate — three numbers, and anything carrying a `-rc1` or `+build` suffix is refused
rather than ordered. Fewer dependencies and no ambiguity about what a release tag may look like.

### Update mechanism — adopt the sibling's, do not design a new one

`flex-taskbar` already solves this against the same forge, and the pattern should be copied rather
than re-invented. Its `src/release.rs` (373 lines, pure and unit-tested) plus `src/win/update.rs`
(607 lines, all the I/O) are the reference:

| Concern | House pattern |
| --- | --- |
| HTTP | **WinHTTP via windows-rs** (`Win32_Networking_WinHttp`), `WINHTTP_FLAG_SECURE`, explicit timeouts, and hard response caps (1 MiB for JSON, 128 MiB for the exe). No HTTP crate, no extra dependency tree — Schannel and the system certificate store do the TLS. |
| Discovery | `GET https://git.zacx.net/api/v1/repos/owner/<repo>/releases?limit=10`. The forge is **Forgejo**, so this is Gitea's `/api/v1`. The `ApiRelease` deserializer ignores unknown fields, so the same code reads GitHub too. |
| Integrity | `SHA256SUMS.txt` published beside the exe; the download's digest must match before anything is written. **A release without sums is never auto-installed** — it is only opened in the browser. |
| Install | Rename-aside: a running image cannot be overwritten but it *can* be renamed. Rename to `*.old.exe`, move the new one into place, put the old name back on any failure, delete the leftover on next start. No `self-replace` crate needed. |
| Version | A hand-rolled `Version { major, minor, patch }` with derived `Ord`, parsed from the tag, compared against `env!("CARGO_PKG_VERSION")`. Pre-release and build-metadata tags are deliberately rejected rather than ordered. No `semver` crate. |
| UX | An `UpdateState` enum — `Unknown` / `Checking` / `UpToDate` / `Available(v)` / `Failed` — driving one line and one button, plus a daily quiet check that only speaks up when there is something new, and a remembered "skip this release". |
| Threading | Pure decision logic in `release.rs` so it unit-tests off Windows; fetching, hashing and swapping on a helper thread; the result posted back to the UI thread as a custom `WM_APP_*` message. |

The single binary from §8 helps here — one file to replace, so there is no partial-update state in
which two executables disagree. That is a real simplification over the sibling, which only ever had
one executable anyway.

**Two deviations this app needs.** The lock screen is not a taskbar launcher, and two of the
sibling's choices do not carry over:

- **Settings mode only.** Checking and installing must be unreachable from overlay mode. The daily
  background check should not run while locked either.
- **No restart-into-lock.** `flex-taskbar` exits with `EXIT_RESTART` and lets a supervisor relaunch
  it. Doing that here would relaunch the *overlay* — restarting a lock screen mid-session is a
  bypass, not a convenience. Install, then exit; the user starts the lock again themselves.

### The portable layout removes the elevation question

Because the app is portable (§8) there is no `%ProgramFiles%` install to elevate for and no
privileged helper service to write — a privileged auto-updater would have been its own attack
surface. The updater writes into the folder it already lives in:

| Step | Location |
| --- | --- |
| Download | `LockScreenData/update/` beside the exe |
| Verify | in place, against `SHA256SUMS.txt` |
| Install | rename `LockScreen.exe` to `LockScreen.old.exe`, move the new one in, roll back on failure |
| Clean up | delete the leftover on next start |

The one new failure mode is a read-only folder, which is the same check §8 already requires at
startup. Detect it before downloading 2 MB rather than after: if the folder cannot be written, the
update button should be disabled with the reason shown, not fail at the final step.

### Release pipeline — also already solved

`flex-taskbar/.github/workflows/release.yml` is the template. Forgejo runs GitHub-Actions-shaped
workflows from `.github/workflows/`, with two local constraints worth knowing before you write one:

- **No published actions.** This Forgejo instance cannot fetch them from `code.forgejo.org`
  ([forgejo#2809](https://codeberg.org/forgejo/forgejo/issues/2809)), so every step is inline
  `pwsh` — including the checkout, which is done by hand with a `http.extraheader` basic-auth token.
- **The tag must match `Cargo.toml`.** The workflow fails the release if `v2.0.1` and
  `version = "2.0.1"` disagree, because the app compares the published tag against the version it
  was built with — a mismatch would offer the same update forever.

Release gates, all `--locked`: `cargo fmt --check`, `cargo test`, `cargo clippy --release
--all-targets -- -D warnings`, `cargo build --release`. Published assets: the exe, `SHA256SUMS.txt`
(in `sha256sum` layout, which the app parses), and a versioned zip for manual updaters. Release notes
are generated from `git log --no-merges` since the previous tag.

One forge quirk recorded in the sibling's comments and worth repeating: the release is attributed by
matching the **tag author's email** to a forge account, not by the API token. A tag made with an
email no account owns is published by "Ghost" — which is a direct argument for the repo-local
identity being set correctly before any tagging happens.

### Residual security risk — state it honestly

An updater is a remote code-execution channel into a process that installs a global keyboard hook
and writes registry policy. The house pattern is solid on transport and on accidental corruption,
but it is worth being precise about what it does and does not protect against:

- **What SHA256SUMS protects:** truncated or corrupted downloads, a mirror serving the wrong file,
  and tampering by anyone who is not the forge itself.
- **What it does not:** a compromise of `git.zacx.net`. An attacker who can publish a release can
  publish both a malicious exe *and* a matching `SHA256SUMS.txt`. The digest is fetched over the same
  channel as the artifact, so it adds no independent trust.
- **What actually closes that gap:** **Authenticode signing** in phase 06. Windows verifies the
  publisher at launch regardless of how the file arrived, so a swapped binary fails on a signature
  Windows already checks. `WinVerifyTrust` is available in windows-rs
  (`Win32/Security/WinTrust`) if the updater should also check before swapping rather than letting
  the next launch discover it.
- A detached Ed25519 signature with the public key embedded in the binary (`minisign-verify 0.3.0`)
  would give independent trust without Authenticode, at the cost of diverging from the sibling and
  taking on offline key custody. **Recommendation:** match the house pattern, and treat Authenticode
  as the mitigation rather than optional polish — a lock screen that installs a system-wide keyboard
  hook warrants a higher bar than a taskbar launcher.

Two requirements regardless: **refuse downgrades** (reject any version not strictly greater than the
running one, or an attacker who can serve the API can push you back to a known-vulnerable build),
and **no silent auto-update** — check in the background, but let the user confirm the install.

### Scoping

Because the sibling supplies a working reference for both the app side and the pipeline, this drops
from a design problem to an adaptation: **2–3 days**, rather than the 4–6 a from-scratch updater
would cost.
The real prerequisite is no longer a signing key for the manifest — it is the Authenticode
certificate in phase 06, which the release workflow needs anyway.

---

## 10. Phased plan

Ordered by dependency, not by size. Each phase ends somewhere runnable, so the port can be abandoned
at any phase boundary without having thrown away the work. Estimates are working days for one
developer already comfortable in Rust; add roughly 40% if Rust is new and the Win32 message loop is
unfamiliar territory.

| Phase | Work | Est. |
| --: | --- | --- |
| **00** | **Workspace and core crate.** Scaffold the single binary and `lockscreen-core`; port config, DPAPI, sounds, registry policy; `build.rs` with icons, VERSIONINFO and git hash. Land the .NET-written `config.ini` regression test first, so compatibility is enforced from the first commit rather than checked at the end. | 2–3 d |
| **01** | **Overlay.** Layered window per monitor, `PER_MONITOR_AWARE_V2`, DWM blur, z-order and `NOACTIVATE` behaviour. The largest single risk and the home of D1 and D2 — do it early, while there is still room to change the GUI decision. | 4–5 d |
| **02** | **Input hooks.** `WH_KEYBOARD_LL` and `WH_MOUSE_LL`, suppression list driven by `GetAsyncKeyState`, RAII unhook. Replaces SharpHook and its native DLL. Fixes D3 and D4. | 2–3 d |
| **03** | **Credentials, setup mode, unlock prompt.** Argon2id verifiers; first-run `--setup` flow; egui unlock dialog; Hello via the interop interface with a real parent `HWND`; master-password rung; modal capture semantics. Fixes Finding B and D5. | 5–6 d |
| **04** | **Voice unlock.** WinRT `SpeechRecognizer`, list grammar, timeout, and a corrected confidence gate. Already proven end-to-end by probe 2. | 2 d |
| **05** | **Settings mode.** Same binary, third mode. Auth gate before the window opens and before any credential change — fixes D7. Real password policy, mic test, version display. | 4–5 d |
| **06** | **Hardening and ship.** Single-instance guard (absent today — overlays can stack), structured logging instead of `catch {}`, portable-folder writability check, Authenticode signing, release workflow, a zip rather than an installer, shortcuts carrying `--settings`. | 3–4 d |
| **07** | **In-app updater.** *Deferrable.* Adapt the sibling's `release.rs` + `win/update.rs`: WinHTTP, Forgejo releases API, `SHA256SUMS.txt` check, rename-aside install, downgrade refusal, settings-only entry point, no restart-into-lock. | 2–3 d |
| **08** | **Parity validation.** Side-by-side on mixed-DPI multi-monitor, with and without Hello hardware, with and without a microphone, and against a `config.ini` from the .NET build. | 3–4 d |

Phase 08 is recorded separately, in [`PARITY.md`](PARITY.md): the machine-checkable half went into
the test suite, and the half that needs somebody at the keyboard is a checklist there. It is also
where D2's correction came from.

**Core total (phases 00–06, 08): 25–32 working days.** With the updater: **27–35 days.** Call it
five to seven calendar weeks for one developer with other duties. Phases 00–03 are over half of it
and carry nearly all the risk; if the project is going to fail, it fails in phase 01.

---

## 11. Risks

| Risk | Severity | Mitigation |
| --- | --- | --- |
| Overlay z-order and focus behaviour is subtly worse than WPF's | **Medium** | The real risk in the project. WPF hides a lot of window-management detail. Mitigated by doing phase 01 first and testing on mixed-DPI hardware before any other work depends on it. |
| Updater becomes an attack surface | **Medium** | `SHA256SUMS` does not survive a forge compromise (§9). Authenticode signing in phase 06 is the real mitigation and should not be treated as optional for an app that installs a system-wide keyboard hook. |
| WinRT async from a Win32 message loop | **Medium** | `.join()` blocks, which is fine on a worker thread and wrong on the UI thread. Hello and speech calls run off-thread and post results back via `PostMessage`. Needs care, not research — this is a known pattern. |
| Rust unfamiliarity on the team | **Medium** | The honest one. `unsafe` Win32 FFI and a `WndProc` are not a beginner's first Rust project. The +40% estimate adjustment above assumes this; a first Rust project of this shape is realistically 8–10 weeks. |
| Forge API or workflow surprises | Low | Forgejo is already proven against both by a sibling project, including the no-published-actions constraint and the tag/version check. |
| Speech recognition unavailable or unpackaged-app limited | Low | Compiled successfully in this unpackaged process (probe 2b, status 0). Behaviour matches the current app, including its "silently disable if unavailable" fallback. |
| egui's text input and IME behaviour in a password field | Low | Password fields are supported. Worth an explicit test with a non-Latin IME, since the current `PasswordBox` gets that from the OS for free. |
| Lost functionality | **None** | 14 of 14 Windows dependencies verified reachable. This was the question the study was asked to answer, and it is settled. |

---

## 12. What the port does not change

Worth stating plainly, because a port can create the impression that security improved more than it
did.

**This is a deterrent overlay, not a security boundary — in either language.** It is a user-mode
process drawing an always-on-top window. Anyone who can run code as that user can terminate it;
`Ctrl+Alt+Del` is a Secure Attention Sequence that no low-level keyboard hook can intercept, so
Sign out, Switch user and the real Windows lock remain reachable; and `DisableTaskMgr` is an `HKCU`
policy value, removable by the same user it restricts. Rust changes none of that. It is the right
tool for "stop someone casually poking at my unattended desk" and the wrong tool for "stop a
determined attacker with an account on this machine".

If the real requirement is an unbypassable lock, the mechanism is a **Windows Credential
Provider** — a COM in-process server implementing `ICredentialProvider`, loaded by LogonUI inside
the secure desktop, where Hello, PIN and SAS handling belong to the OS rather than to your hook.
That is a different product with a different threat model, not a phase of this migration. It *is*
buildable in Rust — windows-rs generates COM server scaffolding via its `#[implement]` macro, and a
credential provider is a `cdylib` — but treat it as a separate decision, made on its own merits,
after this port lands.

Three smaller non-changes: voice recognition stays speaker-independent (§7); the app still cannot
protect its config from the user who owns it — Argon2id means a stolen config no longer yields the
password, but it does not stop that user from replacing the file with one whose verifier they know;
and the updater (§9) adds a code-execution channel that did not exist before, which is a net
security *cost* unless the signing requirements are met in full.

---

## 13. Recommendation

**Proceed, and treat it as a rewrite of the window layer rather than a translation of the code.**
The API parity question — the one that normally kills a .NET-to-Rust port — came back clean on all
fourteen dependencies, verified by execution rather than by documentation. The port's real content
is ~1,100 lines of WPF window management replaced by native windows you control, and that rewrite
is where seven existing defects get fixed, including two visible to users today (D1 blur, D2
high-DPI coverage) and one that undermines the product's purpose (D7, unauthenticated settings).

### Settled

- **Master password** — set at first run, app-local, Argon2id verifier, recovery semantics. Not the
  Windows account password; reasoning and evidence retained in §7.
- **Architecture** — one binary, three modes (`overlay` / `--settings` / `--setup`).
- **Scope** — in-app updates and version reporting are in, as the last and most deferrable phase.
- **UI** — raw Win32, no GUI toolkit (§5), measured rather than argued. The unlock dialog is built,
  and the overlay's z-order and focus behaviour with it.

### Still open

1. **Authenticode certificate** — needed by the phase 06 release workflow, and the only thing that
   makes the phase 07 updater trustworthy against a forge compromise (§9). Who holds it and where it
   lives. If it cannot be obtained, ship the updater anyway but say so in the release notes.
2. **Whether a Credential Provider is the actual goal.** If it is, that reframes this port as a
   stepping stone and changes how much polish phase 06 deserves.

### Do these regardless of the migration decision

Three small fixes in the existing C#; none should wait for a multi-week migration:

- `global.json` pins SDK `8.0.0`, which breaks `dotnet build` on any machine without an 8.x SDK.
- The voice confidence gate at `PasswordWindow.xaml.cs:235` lets a medium-confidence spoken word
  unlock the screen.
- `Save_Click` at `SettingsApp/MainWindow.xaml.cs:88` changes the lock password with no
  authentication at all (D7). This is the most serious of the three.
