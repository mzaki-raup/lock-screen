# Lock Screen

A configurable lock-screen overlay for Windows 10 (build 17763 or later) and Windows 11. It covers
every connected monitor with a translucent overlay, and any key or mouse click brings up an unlock
prompt that tries Windows Hello first — face or fingerprint, falling back to your PIN — then an app
password, with an optional spoken command as a convenience. Title text, transparency and the blur
toggle are all adjustable from a separate settings app.

While the overlay is up it suppresses the Windows key, Alt+Tab, Alt+F4 and Ctrl+Shift+Esc, and sets
the per-user policy that disables Task Manager. Both are undone on unlock.

- Written in C# against .NET 8 and WPF, using WinRT for Windows Hello and speech recognition.
- Two executables: `LockScreen.exe` (the overlay) and `Settings.exe` (the configurator).
- Fully local. No telemetry, no network access of any kind.

> **Status — please read.** This is a deterrent overlay, **not a security boundary.** It is an
> ordinary user-mode process drawing an always-on-top window, so anyone who can run code as your
> user can close it, and `Ctrl+Alt+Del` is a Secure Attention Sequence that no application can
> intercept — Sign out, Switch user and the real Windows lock all remain reachable. Use it to stop
> someone casually poking at an unattended desk, not to stop a determined attacker with an account
> on the machine. See [Known issues](#known-issues) before relying on it, and
> [RUST-MIGRATION.md](RUST-MIGRATION.md) for where the project is going next.

## Unlock methods

| Order | Method | Notes |
| --: | --- | --- |
| 1 | **Windows Hello** — face, fingerprint, or PIN | Attempted automatically when the prompt appears. Windows chooses the modality and handles its own retry limits. Skipped when Hello is not set up on the machine. |
| 2 | **App password** | Shown when Hello is unavailable or you dismiss it. Stored in `config.ini`, encrypted with DPAPI for the current user. |
| 3 | **Voice command** | Off by default. Say "unlock". Convenience only — see [Known issues](#known-issues). |

## Requirements

- Windows 10 build 17763 or later (Windows 11 supported)
- [.NET 8 Desktop Runtime](https://dotnet.microsoft.com/download/dotnet/8.0) to run, or the .NET 8
  SDK to build
- Windows Hello hardware or a configured PIN for method 1; without it the app falls back to the
  password
- A microphone and Windows speech recognition for method 3, if you enable it

## Building

```bash
dotnet restore
dotnet build LockScreenSolution.sln
```

Run either app:

```bash
dotnet run --project LockScreenApp/LockScreenApp.csproj
dotnet run --project SettingsApp/SettingsApp.csproj
```

> **Note.** `global.json` pins the SDK to `8.0.0` with `rollForward: latestMinor`, so the build
> fails outright on a machine that only has a 9.x or 10.x SDK installed. Either install an 8.x SDK
> or change `rollForward` to `latestMajor`.

Build a self-contained copy that needs no runtime installed:

```bash
dotnet publish LockScreenApp/LockScreenApp.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true
```

That produces about 181 MB across eight files — WPF's native libraries cannot be absorbed into a
single file, so `wpfgfx_cor3.dll`, `D3DCompiler_47_cor3.dll` and five others ship alongside the exe.

## Configuration

Settings live in `config.ini`, created next to the executable on first run. `Settings.exe` is the
intended way to edit it.

```ini
[LockScreen]
Title=Enter Password to Unlock
Password=<DPAPI-encrypted, base64>
BlurBackground=false
Transparency=0.5

[VoiceRecognition]
Enabled=false
UnlockCommand=unlock
ConfidenceThreshold=0.7
ListeningTimeout=10
```

| Key | Meaning |
| --- | --- |
| `Title` | Text shown on the unlock prompt |
| `Password` | Encrypted with DPAPI, scoped to the current Windows user on this machine. A config copied to another machine or user profile cannot be decrypted, and the app will tell you to reconfigure it. |
| `BlurBackground` | See [Known issues](#known-issues) — currently has no visible effect |
| `Transparency` | `0.0` (invisible) to `1.0` (opaque black) |
| `ConfidenceThreshold` | `0.0`–`1.0`. Maps onto WinRT's Low (0.4) / Medium (0.7) / High (1.0) |

**On first run the password is set to `1234` and shown in a dialog.** Change it in `Settings.exe`
before using the app for anything that matters.

## Known issues

These are real and worth knowing before you rely on the app. Each is analysed in
[RUST-MIGRATION.md](RUST-MIGRATION.md) with a file and line reference.

- **The settings app requires no authentication** (D7). Anyone at the unattended desktop can run
  `Settings.exe` and change the lock password without entering the current one. This is the most
  serious item here, and it means the overlay can be reset by anyone who reaches the desktop.
- **"Enable Blur" does nothing** (D1). The blur is applied to the overlay's own flat colour rather
  than to the desktop behind it, which is a no-op.
- **The overlay can leave gaps on mixed-DPI setups** (D2). Monitor bounds are read in physical
  pixels and applied as device-independent units, and no DPI awareness is declared, so a scaled
  monitor may not be fully covered.
- **Voice unlock accepts medium-confidence matches** at the default threshold, and speech
  recognition is speaker-independent — so with it enabled, anyone standing at the machine can say
  "unlock" and get in without a password. It is off by default. Leave it off unless you have
  thought about that.
- Key suppression for Alt+Tab and Ctrl+Shift+Esc is unreliable (D4), because modifier state is read
  from the UI framework rather than from the physical keyboard.
- There is no single-instance guard, so launching the overlay twice stacks two of them.

## What's next

A port to Rust is being evaluated — native Win32 instead of WPF, one executable instead of two, a
first-run master password instead of the `1234` default, and in-app updates from this repository.
Every Windows API the current app depends on has been verified callable from Rust, and the seven
defects above get fixed as part of the rewrite.

**[RUST-MIGRATION.md](RUST-MIGRATION.md)** is the full feasibility study: parity matrix, measured
evidence, phased plan and effort estimates. A rendered version is
[here](https://claude.ai/code/artifact/dad05b84-8822-421a-89b2-eccbd5e77f9e).

## Repository layout

```
LockScreenApp/        the overlay — LockScreen.exe
  MainWindow          multi-monitor overlay, global hooks, key suppression
  PasswordWindow      Windows Hello, password entry, voice unlock
  Helpers/            config, DPAPI, sounds, voice
SettingsApp/          the configurator — Settings.exe
  Helpers/            duplicated copies of the above (see D6)
RUST-MIGRATION.md     feasibility study for the Rust port
```

## Credits

Lock Screen — © 2026 Procybro Solutions. Author: Mohd Zaki Raup.

No licence file is present in this repository; treat the code as all rights reserved unless that
changes.
