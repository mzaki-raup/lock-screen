# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Restore dependencies
dotnet restore

# Build entire solution
dotnet build LockScreenSolution.sln

# Build individual projects
dotnet build LockScreenApp/LockScreenApp.csproj
dotnet build SettingsApp/SettingsApp.csproj

# Run applications
dotnet run --project LockScreenApp/LockScreenApp.csproj
dotnet run --project SettingsApp/SettingsApp.csproj
```

## Architecture

This is a Windows-only .NET 8.0 WPF solution with two applications:

### LockScreenApp
Full-screen lock overlay that:
- Displays on all monitors with configurable transparency and blur effects
- Uses SharpHook for global keyboard/mouse hooking to detect user input
- Disables system keys (Windows key, Alt+Tab) and Task Manager via registry
- Requires password entry (DPAPI encrypted) to unlock

Key files:
- `MainWindow.xaml.cs` - Lock screen overlay with multi-monitor support and global hooks
- `PasswordWindow.xaml.cs` - Password entry dialog with sound feedback

### SettingsApp
Configuration UI that manages:
- Lock screen title text
- Transparency level (0.0-1.0)
- Blur effect toggle
- Password (encrypted with DPAPI)

### Shared Helpers (duplicated in both projects)
- `IniHelper.cs` - Reads/writes `config.ini` for settings persistence
- `EncryptionHelper.cs` - DPAPI password encryption (per-user, machine-specific)
- `SoundHelper.cs` - Windows system sound playback

## Configuration

Settings stored in `config.ini` (created on first run):
```ini
[LockScreen]
Title=Enter Password to Unlock
Password=<encrypted-base64>
BlurBackground=false
Transparency=0.5
```

Default password is "1234".

## Platform Requirements

- Windows 10 build 17763 or later
- .NET SDK 8.0