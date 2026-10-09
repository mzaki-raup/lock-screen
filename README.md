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
- No installer. Settings are kept in a `config.ini` beside the executable — though see
  [Known issues](#known-issues), because the current version resolves that path against the working
  directory rather than the executable's own folder.

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
dotnet build LockScreenApp/LockScreenApp.csproj
dotnet build SettingsApp/SettingsApp.csproj
```

Run either app:

```bash
dotnet run --project LockScreenApp/LockScreenApp.csproj
dotnet run --project SettingsApp/SettingsApp.csproj
```

> **Note.** `global.json` pins the SDK to `8.0.0` with `rollForward: latestMinor` — meaning *stay on
> major version 8* — so the build fails outright on a machine that has only a 9.x or 10.x SDK. The
> pin buys nothing: it cannot give reproducible builds when the SDK it names is installed nowhere,
> and a 10.x SDK compiles a `net8.0` target perfectly well (the SDK version and the target framework
> are independent). Delete the file, or set `rollForward` to `latestMajor` if you want to keep the
> documented 8.0 floor.

Build a self-contained copy that needs no runtime installed:

```bash
dotnet publish LockScreenApp/LockScreenApp.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true
```

That produces about 181 MB across eight files — WPF's native libraries cannot be absorbed into a
single file, so `wpfgfx_cor3.dll`, `D3DCompiler_47_cor3.dll` and five others ship alongside the exe.

## Configuration

Settings live in `config.ini`, created on first run. `Settings.exe` is the intended way to edit it.

> **Where that file lands.** `config.ini` is a bare relative path, so it resolves against the
> process's **current working directory** — not the folder the executable sits in. Launch the app
> from a shortcut with a different *Start in* field, from a scheduled task, or from a terminal
> elsewhere, and you get a different `config.ini`; if none is found there, a fresh one is created
> with the default password. Keep the working directory set to the app's own folder until this is
> fixed in the port.

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
- **The overlay is mis-sized on mixed-DPI setups** (D2). No DPI awareness is declared anywhere, so
  Windows virtualises every coordinate the app reads: on a 150%-scaled monitor it reads 1280x720
  where the screen is 1920x1080. The overlay is built in those units and stretched back up, which
  makes it and the password prompt soft; whether it also leaves an uncovered edge has not been
  confirmed on screen. Measured in `PARITY.md` §2.
- **Voice unlock accepts medium-confidence matches** at the default threshold, and speech
  recognition is speaker-independent — so with it enabled, anyone standing at the machine can say
  "unlock" and get in without a password. It is off by default. Leave it off unless you have
  thought about that.
- Key suppression for Alt+Tab and Ctrl+Shift+Esc is unreliable (D4), because modifier state is read
  from the UI framework rather than from the physical keyboard.
- **`config.ini` follows the working directory, not the executable** (part of D6). Launching from a
  different directory silently uses — or creates — a different config, including one carrying the
  default password.
- There is no single-instance guard, so launching the overlay twice stacks two of them.

## What's next

A port to Rust is underway — native Win32 instead of WPF, one executable instead of two, a first-run
master password instead of the `1234` default, and in-app updates from this repository. Every
Windows API the current app depends on has been verified callable from Rust, and the defects above
get fixed as part of the rewrite.

The port is **properly portable**: `config.ini` resolves against the executable's own folder via
`GetModuleFileNameW`, so it no longer matters where the app is launched from. Everything it writes
sits beside the exe:

```
LockScreen.exe
config.ini            your settings
LockScreenData/       logs and update downloads - derived, safe to delete
```

 Nothing is written to
`%APPDATA%` or `%LOCALAPPDATA%`, there is no installer, and a copy of the folder carries its settings
with it — which works because the password becomes an Argon2id hash rather than a DPAPI blob tied to
one user on one machine. The supported floor stays **Windows 10 build 17763 (1809) and later**,
unchanged from today.

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
